//! nano:gas — Google Apps Script compatibility shim for nano-rs.
//!
//! Two usage modes:
//!
//! **Classic mode** (recommended for non-tech users):
//! Name your entrypoint `*.gs` — the runtime auto-detects the Google Apps Script
//! flavor from the extension (app config `compat: "auto"`, the default) and it can
//! be forced with `compat: "gas"`. Upload your `.gs` file unchanged. The runtime
//! injects GAS_SHIM_PREFIX before and GAS_SHIM_SUFFIX after your code, making
//! SpreadsheetApp, Logger, PropertiesService, etc. available. Functions must be
//! `async` when they call Sheets/Docs/Drive APIs. The Google service-account key
//! and spreadsheet id still come from `env_vars` (they are secrets, not flags).
//!
//! ```javascript
//! // handler.js — your GAS script (no modification except adding async where needed)
//! async function doGet(e) {
//!   const sheet = SpreadsheetApp.getActiveSpreadsheet().getActiveSheet();
//!   const values = await sheet.getDataRange().getValues();
//!   return JSON.stringify(values);
//! }
//! ```
//!
//! **ESM mode** (for operators preferring explicit imports):
//! ```javascript
//! import { dispatch } from 'nano:gas';
//! async function doGet(e) { ... }
//! async function doPost(e) { ... }
//! export default { fetch: req => dispatch(req, { doGet, doPost }) };
//! ```
//!
//! **Required env_vars:**
//! - `GOOGLE_SERVICE_ACCOUNT_KEY`: JSON string of the service account key file
//! - `SPREADSHEET_ID`: target spreadsheet (for `SpreadsheetApp.getActiveSpreadsheet()`)
//! - `SHEET_NAME`: (optional) name of the active sheet, defaults to first sheet
//! - `GAS_USER_EMAIL`: (optional) returned by `Session.getEffectiveUser().getEmail()`
//!
//! **Entry-point dispatch order** (classic mode suffix):
//! 1. GET → `doGet(e)` if defined
//! 2. POST with `{"function":"name","args":[...]}` body → direct function call
//! 3. POST → `doPost(e)` if defined
//! 4. Fallback → `main()` if defined

// ── Classic-script prefix ────────────────────────────────────────────────────
// Injected before the user's GAS script when the app's compat flavor resolves to
// GAS (a `.gs` entrypoint under the default `auto`, or `compat: "gas"`).
// Sets up all GAS globals using nano-rs primitives:
//   __gasFetch  – original outbound fetch (captured before suffix could reassign)
//   __nano_kv_* – synchronous KV globals from kv.rs bind_kv()
//   Nano.env    – per-tenant env_vars from vfs_bindings.rs set_current_env()

pub const GAS_SHIM_PREFIX: &str = r#"
var __gasFetch = fetch;
var __gasToken = null, __gasTokenExpiry = 0, __gasPendingWrites = [];
var __gasSpreadsheetId = (typeof Nano !== 'undefined' && Nano.env && Nano.env.SPREADSHEET_ID) || null;

async function __gasGetToken() {
  var now = Date.now();
  if (__gasToken && now < __gasTokenExpiry) return __gasToken;
  var sk = typeof Nano !== 'undefined' && Nano.env && Nano.env.GOOGLE_SERVICE_ACCOUNT_KEY;
  if (!sk) throw new Error('nano:gas: env_vars.GOOGLE_SERVICE_ACCOUNT_KEY is required');
  var sa = JSON.parse(sk), ns = Math.floor(now / 1000);
  var claim = {iss: sa.client_email, scope: 'https://www.googleapis.com/auth/spreadsheets https://www.googleapis.com/auth/documents https://www.googleapis.com/auth/drive', aud: 'https://oauth2.googleapis.com/token', exp: ns + 3600, iat: ns};
  function b64u(s) { return btoa(s).replace(/=/g,'').replace(/\+/g,'-').replace(/\//g,'_'); }
  var u = b64u(JSON.stringify({alg:'RS256',typ:'JWT'})) + '.' + b64u(JSON.stringify(claim));
  var pem = sa.private_key.replace(/-----[^-]+-----/g,'').replace(/\n/g,'').trim();
  var kb = Uint8Array.from(atob(pem), function(c){return c.charCodeAt(0);});
  var ck = await crypto.subtle.importKey('pkcs8', kb.buffer, {name:'RSASSA-PKCS1-v1_5',hash:{name:'SHA-256'}}, false, ['sign']);
  var sb = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', ck, new TextEncoder().encode(u));
  var jwt = u + '.' + b64u(String.fromCharCode.apply(null, new Uint8Array(sb)));
  var r = await __gasFetch('https://oauth2.googleapis.com/token', {method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:'grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion='+jwt});
  var d = await r.json();
  if (!d.access_token) throw new Error('nano:gas: token exchange failed: ' + JSON.stringify(d));
  __gasToken = d.access_token; __gasTokenExpiry = now + 3500000;
  return __gasToken;
}

async function __gasFlush() {
  if (!__gasPendingWrites.length) return;
  var tok = await __gasGetToken(), bySid = {};
  __gasPendingWrites.forEach(function(w){ (bySid[w.s] = bySid[w.s] || []).push({range:w.r,values:w.v}); });
  for (var sid in bySid) {
    await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/' + sid + '/values:batchUpdate', {method:'POST',headers:{'Authorization':'Bearer '+tok,'Content-Type':'application/json'},body:JSON.stringify({valueInputOption:'USER_ENTERED',data:bySid[sid]})});
  }
  __gasPendingWrites = [];
}

function __gasRange(sid, sheet, r, c, nr, nc) {
  nr=nr||1; nc=nc||1;
  var addr=(sheet?sheet+'!':'')+'R'+r+'C'+c+':R'+(r+nr-1)+'C'+(c+nc-1);
  return {
    async getValues() {
      var tok=await __gasGetToken();
      var res=await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/'+sid+'/values/'+encodeURIComponent(addr),{headers:{'Authorization':'Bearer '+tok}});
      var d=await res.json(); return d.values||[];
    },
    async getValue() { var v=await this.getValues(); return (v[0]&&v[0][0]!=null)?v[0][0]:null; },
    setValues(v) { __gasPendingWrites.push({s:sid,r:addr,v:v}); return this; },
    setValue(v) { return this.setValues([[v]]); }
  };
}

function __gasSheet(sid, sheet) {
  return {
    getName() { return sheet||'Sheet1'; },
    getRange(r,c,nr,nc) { return __gasRange(sid,sheet,r,c,nr,nc); },
    async getDataRange() {
      var tok=await __gasGetToken();
      var res=await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/'+sid+'/values/'+encodeURIComponent(sheet||'Sheet1'),{headers:{'Authorization':'Bearer '+tok}});
      var vals=(await res.json()).values||[];
      if(!vals.length) return __gasRange(sid,sheet,1,1,1,1);
      return __gasRange(sid,sheet,1,1,vals.length,Math.max.apply(null,vals.map(function(r){return r.length;})));
    },
    async getLastRow() {
      var tok=await __gasGetToken();
      var res=await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/'+sid+'/values/'+encodeURIComponent(sheet||'Sheet1'),{headers:{'Authorization':'Bearer '+tok}});
      return ((await res.json()).values||[]).length;
    },
    async appendRow(row) {
      var tok=await __gasGetToken();
      await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/'+sid+'/values/'+encodeURIComponent(sheet||'Sheet1')+':append?valueInputOption=USER_ENTERED',{method:'POST',headers:{'Authorization':'Bearer '+tok,'Content-Type':'application/json'},body:JSON.stringify({values:[row]})});
    },
    async clearContents() {
      var tok=await __gasGetToken();
      await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/'+sid+'/values/'+encodeURIComponent(sheet||'Sheet1')+':clear',{method:'POST',headers:{'Authorization':'Bearer '+tok}});
    }
  };
}

function __gasSpreadsheet(sid) {
  var active=(typeof Nano !== 'undefined' && Nano.env && Nano.env.SHEET_NAME)||null;
  return {
    getId() { return sid; },
    getName() { return sid; },
    getActiveSheet() { return __gasSheet(sid,active); },
    getSheetByName(n) { return __gasSheet(sid,n); },
    async getSheets() {
      var tok=await __gasGetToken();
      var r=await __gasFetch('https://sheets.googleapis.com/v4/spreadsheets/'+sid+'?fields=sheets.properties.title',{headers:{'Authorization':'Bearer '+tok}});
      return ((await r.json()).sheets||[]).map(function(s){return __gasSheet(sid,s.properties.title);});
    },
    async flush() { await __gasFlush(); }
  };
}

globalThis.SpreadsheetApp = {
  getActiveSpreadsheet() {
    if (!__gasSpreadsheetId) throw new Error('nano:gas: set env_vars.SPREADSHEET_ID or use SpreadsheetApp.openById(id)');
    return __gasSpreadsheet(__gasSpreadsheetId);
  },
  openById(id) { return __gasSpreadsheet(id); },
  async flush() { await __gasFlush(); }
};

globalThis.Logger = {
  log() { console.log.apply(console, arguments); },
  info() { console.log.apply(console, arguments); },
  warning() { console.warn.apply(console, arguments); },
  error() { console.error.apply(console, arguments); }
};

globalThis.UrlFetchApp = {
  async fetch(url, params) {
    params=params||{};
    var opts={method:params.method||'GET',headers:params.headers||{}};
    if(params.payload) opts.body=typeof params.payload==='string'?params.payload:JSON.stringify(params.payload);
    var r=await __gasFetch(url,opts), text=await r.text();
    return {getContentText(){return text;},getResponseCode(){return r.status;},getHeaders(){var h={};r.headers.forEach(function(v,k){h[k]=v;});return h;}};
  },
  async fetchAll(reqs) {
    return Promise.all(reqs.map(function(r){return globalThis.UrlFetchApp.fetch(r.url||r,r);}));
  }
};

function __gasProps(ns) {
  var pfx='_gp_'+ns+'_';
  return {
    getProperty(k) { var b=__nano_kv_get(pfx+k,'_gas'); return b?new TextDecoder().decode(b):null; },
    setProperty(k,v) { __nano_kv_set(pfx+k,new TextEncoder().encode(String(v)),'_gas'); },
    deleteProperty(k) { __nano_kv_delete(pfx+k,'_gas'); },
    getProperties() {
      var p=__nano_kv_list(pfx,'_gas'),r={};
      for(var i=0;i<p.length;i++) r[p[i][0].slice(pfx.length)]=new TextDecoder().decode(p[i][1]);
      return r;
    },
    deleteAllProperties() {
      var p=__nano_kv_list(pfx,'_gas');
      for(var i=0;i<p.length;i++) __nano_kv_delete(p[i][0],'_gas');
      return this;
    }
  };
}
globalThis.PropertiesService = {
  getScriptProperties() { return __gasProps('s'); },
  getUserProperties() { return __gasProps('u'); },
  getDocumentProperties() { return __gasProps('d'); }
};

function __gasCache(ns) {
  var pfx='_gc_'+ns+'_';
  return {
    get(k) {
      var b=__nano_kv_get(pfx+k,'_gas');
      if(!b) return null;
      try { var e=JSON.parse(new TextDecoder().decode(b)); if(e.x&&Date.now()>e.x){__nano_kv_delete(pfx+k,'_gas');return null;} return e.v; } catch(ex){return null;}
    },
    put(k,v,ttl) { __nano_kv_set(pfx+k,new TextEncoder().encode(JSON.stringify({v:String(v),x:ttl?Date.now()+ttl*1000:0})),'_gas'); },
    remove(k) { __nano_kv_delete(pfx+k,'_gas'); }
  };
}
globalThis.CacheService = {
  getScriptCache() { return __gasCache('s'); },
  getUserCache() { return __gasCache('u'); },
  getDocumentCache() { return __gasCache('d'); }
};

globalThis.Utilities = {
  base64Encode(d) { return typeof d==='string'?btoa(d):btoa(String.fromCharCode.apply(null,d)); },
  base64Decode(s) { var d=atob(s); return Array.from(d,function(c){return c.charCodeAt(0);}); },
  base64EncodeWebSafe(d) { return globalThis.Utilities.base64Encode(d).replace(/\+/g,'-').replace(/\//g,'_').replace(/=/g,''); },
  base64DecodeWebSafe(s) { return globalThis.Utilities.base64Decode(s.replace(/-/g,'+').replace(/_/g,'/')); },
  newBlob(data,ct,name) {
    var b=typeof data==='string'?new TextEncoder().encode(data):new Uint8Array(data);
    return {getBytes(){return Array.from(b);},getDataAsString(cs){return new TextDecoder(cs||'utf-8').decode(b);},getContentType(){return ct||'application/octet-stream';},getName(){return name||'';},setName(n){name=n;return this;}};
  },
  sleep(ms) { var e=Date.now()+ms; while(Date.now()<e){} },
  formatDate(d,tz) { return new Date(d).toLocaleString('en-US',{timeZone:tz}); },
  parseCsv(csv) { return csv.split('\n').map(function(r){return r.split(',');}); },
  getUuid() { return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,function(c){var r=Math.random()*16|0;return(c==='x'?r:(r&0x3|0x8)).toString(16);}); }
};

globalThis.DocumentApp = {
  async openById(id) {
    var tok=await __gasGetToken();
    var r=await __gasFetch('https://docs.googleapis.com/v1/documents/'+id,{headers:{'Authorization':'Bearer '+tok}});
    var doc=await r.json();
    function xt(c) {
      return (c||[]).map(function(el){
        if(el.paragraph) return (el.paragraph.elements||[]).map(function(e){return (e.textRun&&e.textRun.content)||'';}).join('');
        if(el.table) return (el.table.tableRows||[]).map(function(row){return (row.tableCells||[]).map(function(cell){return xt(cell.content);}).join('\t');}).join('\n');
        return '';
      }).join('');
    }
    var bodyText=xt(doc.body&&doc.body.content);
    return {
      getId(){return id;},getName(){return doc.title||'';},
      getBody(){
        return {
          getText(){return bodyText;},
          getParagraphs(){
            return ((doc.body&&doc.body.content)||[]).filter(function(e){return e.paragraph;}).map(function(e){
              var t=(e.paragraph.elements||[]).map(function(el){return(el.textRun&&el.textRun.content)||'';}).join('');
              return {getText(){return t;}};
            });
          }
        };
      }
    };
  }
};

globalThis.DriveApp = {
  async getFileById(id) {
    var tok=await __gasGetToken();
    var r=await __gasFetch('https://www.googleapis.com/drive/v3/files/'+id+'?fields=id,name,mimeType,size',{headers:{'Authorization':'Bearer '+tok}});
    var m=await r.json();
    return {
      getId(){return id;},getName(){return m.name;},getMimeType(){return m.mimeType;},getSize(){return m.size;},
      async getBlob() {
        var cr=await __gasFetch('https://www.googleapis.com/drive/v3/files/'+id+'?alt=media',{headers:{'Authorization':'Bearer '+tok}});
        var b=new Uint8Array(await cr.arrayBuffer());
        return {getDataAsString(cs){return new TextDecoder(cs||'utf-8').decode(b);},getBytes(){return Array.from(b);},getContentType(){return m.mimeType;},getName(){return m.name;}};
      }
    };
  },
  async getFolderById(id) {
    var tok=await __gasGetToken();
    return {
      getId(){return id;},
      async getFiles() {
        var r=await __gasFetch("https://www.googleapis.com/drive/v3/files?q='"+id+"'+in+parents&fields=files(id,name,mimeType)",{headers:{'Authorization':'Bearer '+tok}});
        var files=(await r.json()).files||[],i=0;
        return {hasNext(){return i<files.length;},next(){return globalThis.DriveApp.getFileById(files[i++].id);}};
      }
    };
  }
};

function __gasStub(name) {
  try {
    return new Proxy({},{get:function(_,p){return function(){throw new Error('nano:gas: '+name+'.'+String(p)+'() is not available in this deployment.');};},});
  } catch(e) { return {}; }
}
globalThis.GmailApp=__gasStub('GmailApp');
globalThis.CalendarApp=__gasStub('CalendarApp');
globalThis.FormApp=__gasStub('FormApp');
globalThis.SlidesApp=__gasStub('SlidesApp');
globalThis.MailApp=__gasStub('MailApp');
globalThis.HtmlService={createHtmlOutput:function(html){return{getContent(){return html;},setTitle(){return this;},setXFrameOptionsMode(){return this;}};}};
globalThis.ScriptApp={getOAuthToken(){return __gasToken;},newTrigger(){return{timeBased(){return{everyMinutes(){return{create(){}};},everyHours(){return{create(){}};},atHour(){return this;},nearMinute(){return this;},create(){}};}};}};
globalThis.Session={
  getEffectiveUser(){return{getEmail(){return(typeof Nano!=='undefined'&&Nano.env&&Nano.env.GAS_USER_EMAIL)||'service-account@nano.local';},getUsername(){return 'service-account';}};},
  getActiveUser(){return this.getEffectiveUser();}
};
"#;

// ── Classic-script suffix ────────────────────────────────────────────────────
// Injected after the user's GAS script.
// Installs __nano_user_fetch which handler.rs looks for (checked before globalThis.fetch).
// Routes requests to doGet / doPost / function dispatch / main.

pub const GAS_SHIM_SUFFIX: &str = r#"
globalThis.__nano_user_fetch = async function(request) {
  var method=request.method||'GET', result;
  var url; try{url=new URL(request.url);}catch(e){url={searchParams:{forEach:function(){}},search:''};}
  var body=''; if(method!=='GET'){try{body=await request.text();}catch(e){}}

  if(method==='GET'&&typeof doGet==='function'){
    var params={};
    try{url.searchParams.forEach(function(v,k){params[k]=v;});}catch(e){}
    result=await Promise.resolve(doGet({parameter:params,queryString:url.search?url.search.slice(1):''}));
  } else if(method==='POST'){
    var dispatched=false;
    if(body){
      try{
        var j=JSON.parse(body);
        if(j['function']&&typeof globalThis[j['function']]==='function'){
          result=await Promise.resolve(globalThis[j['function']].apply(null,j.args||[]));
          dispatched=true;
        }
      }catch(e){}
    }
    if(!dispatched&&typeof doPost==='function')
      result=await Promise.resolve(doPost({postData:{contents:body,type:(request.headers&&request.headers.get?request.headers.get('content-type'):'')||''},parameter:{}}));
  }

  if(result===undefined&&typeof main==='function') result=await Promise.resolve(main());

  await __gasFlush();

  if(result==null) return new Response('',{status:204});
  if(result instanceof Response) return result;
  if(typeof result==='string') return new Response(result,{headers:{'Content-Type':'text/plain'}});
  if(result&&typeof result==='object'&&typeof result.getContent==='function') return new Response(result.getContent(),{headers:{'Content-Type':'text/html'}});
  return new Response(JSON.stringify(result),{headers:{'Content-Type':'application/json'}});
};
"#;

// ── ESM module code for `import 'nano:gas'` ──────────────────────────────────
// Sets up the same GAS globals and exports `dispatch(request, handlers)`.
// Arrow functions and template literals are safe in ESM context (V8 full ES2020+).

/// Return GAS_MODULE_CODE — convenience alias used by get_nano_module_code().
pub fn module_code() -> &'static str {
    GAS_MODULE_CODE
}

pub const GAS_MODULE_CODE: &str = r#"
const __gF = fetch;
let __gT = null, __gTE = 0, __gW = [];
const __gSid = (typeof Nano !== 'undefined' && Nano.env?.SPREADSHEET_ID) || null;

async function __gToken() {
  const now = Date.now();
  if (__gT && now < __gTE) return __gT;
  const sk = typeof Nano !== 'undefined' && Nano.env?.GOOGLE_SERVICE_ACCOUNT_KEY;
  if (!sk) throw new Error('nano:gas: env_vars.GOOGLE_SERVICE_ACCOUNT_KEY is required');
  const sa = JSON.parse(sk), ns = Math.floor(now / 1000);
  const claim = {iss:sa.client_email,scope:'https://www.googleapis.com/auth/spreadsheets https://www.googleapis.com/auth/documents https://www.googleapis.com/auth/drive',aud:'https://oauth2.googleapis.com/token',exp:ns+3600,iat:ns};
  const b64u = s => btoa(s).replace(/=/g,'').replace(/\+/g,'-').replace(/\//g,'_');
  const u = b64u(JSON.stringify({alg:'RS256',typ:'JWT'})) + '.' + b64u(JSON.stringify(claim));
  const pem = sa.private_key.replace(/-----[^-]+-----/g,'').replace(/\n/g,'').trim();
  const kb = Uint8Array.from(atob(pem), c => c.charCodeAt(0));
  const ck = await crypto.subtle.importKey('pkcs8',kb.buffer,{name:'RSASSA-PKCS1-v1_5',hash:{name:'SHA-256'}},false,['sign']);
  const sb = await crypto.subtle.sign('RSASSA-PKCS1-v1_5',ck,new TextEncoder().encode(u));
  const jwt = u + '.' + b64u(String.fromCharCode(...new Uint8Array(sb)));
  const r = await __gF('https://oauth2.googleapis.com/token',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:'grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion='+jwt});
  const d = await r.json();
  if (!d.access_token) throw new Error('nano:gas: token exchange failed: ' + JSON.stringify(d));
  __gT = d.access_token; __gTE = now + 3500000;
  return __gT;
}

async function __gFlush() {
  if (!__gW.length) return;
  const tok = await __gToken(), bySid = {};
  __gW.forEach(w => (bySid[w.s] = bySid[w.s] || []).push({range:w.r,values:w.v}));
  for (const sid in bySid) {
    await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}/values:batchUpdate`,{method:'POST',headers:{'Authorization':'Bearer '+tok,'Content-Type':'application/json'},body:JSON.stringify({valueInputOption:'USER_ENTERED',data:bySid[sid]})});
  }
  __gW = [];
}

const __gRange = (sid, sheet, r, c, nr=1, nc=1) => {
  const addr = (sheet?sheet+'!':'') + `R${r}C${c}:R${r+nr-1}C${c+nc-1}`;
  return {
    async getValues() {
      const tok = await __gToken();
      const res = await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}/values/${encodeURIComponent(addr)}`,{headers:{'Authorization':'Bearer '+tok}});
      return (await res.json()).values || [];
    },
    async getValue() { const v = await this.getValues(); return v[0]?.[0] ?? null; },
    setValues(v) { __gW.push({s:sid,r:addr,v}); return this; },
    setValue(v) { return this.setValues([[v]]); }
  };
};

const __gSheet = (sid, sheet) => ({
  getName: () => sheet || 'Sheet1',
  getRange: (r,c,nr,nc) => __gRange(sid,sheet,r,c,nr,nc),
  async getDataRange() {
    const tok = await __gToken();
    const res = await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}/values/${encodeURIComponent(sheet||'Sheet1')}`,{headers:{'Authorization':'Bearer '+tok}});
    const vals = (await res.json()).values || [];
    if (!vals.length) return __gRange(sid,sheet,1,1,1,1);
    return __gRange(sid,sheet,1,1,vals.length,Math.max(...vals.map(r => r.length)));
  },
  async getLastRow() {
    const tok = await __gToken();
    const res = await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}/values/${encodeURIComponent(sheet||'Sheet1')}`,{headers:{'Authorization':'Bearer '+tok}});
    return ((await res.json()).values || []).length;
  },
  async appendRow(row) {
    const tok = await __gToken();
    await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}/values/${encodeURIComponent(sheet||'Sheet1')}:append?valueInputOption=USER_ENTERED`,{method:'POST',headers:{'Authorization':'Bearer '+tok,'Content-Type':'application/json'},body:JSON.stringify({values:[row]})});
  },
  async clearContents() {
    const tok = await __gToken();
    await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}/values/${encodeURIComponent(sheet||'Sheet1')}:clear`,{method:'POST',headers:{'Authorization':'Bearer '+tok}});
  }
});

const __gSpreadsheet = (sid) => {
  const active = (typeof Nano !== 'undefined' && Nano.env?.SHEET_NAME) || null;
  return {
    getId: () => sid,
    getName: () => sid,
    getActiveSheet: () => __gSheet(sid, active),
    getSheetByName: n => __gSheet(sid, n),
    async getSheets() {
      const tok = await __gToken();
      const r = await __gF(`https://sheets.googleapis.com/v4/spreadsheets/${sid}?fields=sheets.properties.title`,{headers:{'Authorization':'Bearer '+tok}});
      return ((await r.json()).sheets || []).map(s => __gSheet(sid, s.properties.title));
    },
    flush: () => __gFlush()
  };
};

// Set up globals (side-effect of importing this module)
const SpreadsheetApp = {
  getActiveSpreadsheet() {
    if (!__gSid) throw new Error('nano:gas: set env_vars.SPREADSHEET_ID or use SpreadsheetApp.openById(id)');
    return __gSpreadsheet(__gSid);
  },
  openById: id => __gSpreadsheet(id),
  flush: () => __gFlush()
};
globalThis.SpreadsheetApp = SpreadsheetApp;

const Logger = {
  log: (...a) => console.log(...a),
  info: (...a) => console.log(...a),
  warning: (...a) => console.warn(...a),
  error: (...a) => console.error(...a)
};
globalThis.Logger = Logger;

const UrlFetchApp = {
  async fetch(url, params = {}) {
    const opts = {method:params.method||'GET',headers:params.headers||{}};
    if (params.payload) opts.body = typeof params.payload === 'string' ? params.payload : JSON.stringify(params.payload);
    const r = await __gF(url, opts), text = await r.text();
    return {getContentText:()=>text,getResponseCode:()=>r.status,getHeaders:()=>{const h={};r.headers.forEach((v,k)=>h[k]=v);return h;}};
  },
  fetchAll: reqs => Promise.all(reqs.map(r => UrlFetchApp.fetch(r.url||r, r)))
};
globalThis.UrlFetchApp = UrlFetchApp;

const __mkProps = ns => {
  const pfx = '_gp_' + ns + '_';
  return {
    getProperty: k => { const b = __nano_kv_get(pfx+k,'_gas'); return b ? new TextDecoder().decode(b) : null; },
    setProperty: (k,v) => __nano_kv_set(pfx+k, new TextEncoder().encode(String(v)), '_gas'),
    deleteProperty: k => __nano_kv_delete(pfx+k, '_gas'),
    getProperties() {
      const pairs = __nano_kv_list(pfx, '_gas'), r = {};
      for (const [k, v] of pairs) r[k.slice(pfx.length)] = new TextDecoder().decode(v);
      return r;
    },
    deleteAllProperties() {
      for (const [k] of __nano_kv_list(pfx,'_gas')) __nano_kv_delete(k, '_gas');
      return this;
    }
  };
};
const PropertiesService = {
  getScriptProperties: () => __mkProps('s'),
  getUserProperties: () => __mkProps('u'),
  getDocumentProperties: () => __mkProps('d')
};
globalThis.PropertiesService = PropertiesService;

const __mkCache = ns => {
  const pfx = '_gc_' + ns + '_';
  return {
    get(k) {
      const b = __nano_kv_get(pfx+k,'_gas');
      if (!b) return null;
      try { const e = JSON.parse(new TextDecoder().decode(b)); if (e.x && Date.now() > e.x) { __nano_kv_delete(pfx+k,'_gas'); return null; } return e.v; } catch { return null; }
    },
    put: (k,v,ttl) => __nano_kv_set(pfx+k, new TextEncoder().encode(JSON.stringify({v:String(v),x:ttl?Date.now()+ttl*1000:0})), '_gas'),
    remove: k => __nano_kv_delete(pfx+k,'_gas')
  };
};
const CacheService = {
  getScriptCache: () => __mkCache('s'),
  getUserCache: () => __mkCache('u'),
  getDocumentCache: () => __mkCache('d')
};
globalThis.CacheService = CacheService;

const Utilities = {
  base64Encode: d => typeof d === 'string' ? btoa(d) : btoa(String.fromCharCode(...d)),
  base64Decode: s => Array.from(atob(s), c => c.charCodeAt(0)),
  base64EncodeWebSafe: d => Utilities.base64Encode(d).replace(/\+/g,'-').replace(/\//g,'_').replace(/=/g,''),
  base64DecodeWebSafe: s => Utilities.base64Decode(s.replace(/-/g,'+').replace(/_/g,'/')),
  newBlob(data, ct, name) {
    const b = typeof data === 'string' ? new TextEncoder().encode(data) : new Uint8Array(data);
    return {getBytes:()=>Array.from(b),getDataAsString:(cs)=>new TextDecoder(cs||'utf-8').decode(b),getContentType:()=>ct||'application/octet-stream',getName:()=>name||'',setName(n){name=n;return this;}};
  },
  sleep: ms => { const e = Date.now()+ms; while(Date.now()<e){} },
  formatDate: (d,tz) => new Date(d).toLocaleString('en-US',{timeZone:tz}),
  parseCsv: csv => csv.split('\n').map(r => r.split(',')),
  getUuid: () => 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,c=>{const r=Math.random()*16|0;return(c==='x'?r:(r&0x3|0x8)).toString(16);})
};
globalThis.Utilities = Utilities;

const DocumentApp = {
  async openById(id) {
    const tok = await __gToken();
    const r = await __gF(`https://docs.googleapis.com/v1/documents/${id}`,{headers:{'Authorization':'Bearer '+tok}});
    const doc = await r.json();
    const xt = c => (c||[]).map(el => {
      if (el.paragraph) return (el.paragraph.elements||[]).map(e=>(e.textRun?.content)||'').join('');
      if (el.table) return (el.table.tableRows||[]).map(row=>(row.tableCells||[]).map(cell=>xt(cell.content)).join('\t')).join('\n');
      return '';
    }).join('');
    const bodyText = xt(doc.body?.content);
    return {
      getId: () => id, getName: () => doc.title || '',
      getBody: () => ({
        getText: () => bodyText,
        getParagraphs: () => (doc.body?.content||[]).filter(e=>e.paragraph).map(e=>{
          const t = (e.paragraph.elements||[]).map(el=>(el.textRun?.content)||'').join('');
          return {getText:()=>t};
        })
      })
    };
  }
};
globalThis.DocumentApp = DocumentApp;

const DriveApp = {
  async getFileById(id) {
    const tok = await __gToken();
    const r = await __gF(`https://www.googleapis.com/drive/v3/files/${id}?fields=id,name,mimeType,size`,{headers:{'Authorization':'Bearer '+tok}});
    const m = await r.json();
    return {
      getId:()=>id, getName:()=>m.name, getMimeType:()=>m.mimeType, getSize:()=>m.size,
      async getBlob() {
        const cr = await __gF(`https://www.googleapis.com/drive/v3/files/${id}?alt=media`,{headers:{'Authorization':'Bearer '+tok}});
        const b = new Uint8Array(await cr.arrayBuffer());
        return {getDataAsString:(cs)=>new TextDecoder(cs||'utf-8').decode(b),getBytes:()=>Array.from(b),getContentType:()=>m.mimeType,getName:()=>m.name};
      }
    };
  },
  async getFolderById(id) {
    const tok = await __gToken();
    return {
      getId: () => id,
      async getFiles() {
        const r = await __gF(`https://www.googleapis.com/drive/v3/files?q='${id}'+in+parents&fields=files(id,name,mimeType)`,{headers:{'Authorization':'Bearer '+tok}});
        const files = (await r.json()).files || []; let i = 0;
        return {hasNext:()=>i<files.length, next:()=>DriveApp.getFileById(files[i++].id)};
      }
    };
  }
};
globalThis.DriveApp = DriveApp;

const __stub = name => new Proxy({},{get:(_,p)=>()=>{throw new Error(`nano:gas: ${name}.${String(p)}() is not available in this deployment`);}});
globalThis.GmailApp = __stub('GmailApp');
globalThis.CalendarApp = __stub('CalendarApp');
globalThis.FormApp = __stub('FormApp');
globalThis.SlidesApp = __stub('SlidesApp');
globalThis.MailApp = __stub('MailApp');
globalThis.HtmlService = {createHtmlOutput:html=>({getContent:()=>html,setTitle:()=>this,setXFrameOptionsMode:()=>this})};
globalThis.ScriptApp = {getOAuthToken:()=>__gT};
globalThis.Session = {
  getEffectiveUser:()=>({getEmail:()=>(typeof Nano!=='undefined'&&Nano.env?.GAS_USER_EMAIL)||'service-account@nano.local',getUsername:()=>'service-account'}),
  getActiveUser(){return this.getEffectiveUser();}
};

// ── dispatch() — the recommended ESM entry point ──────────────────────────────
// Usage: export default { fetch: req => dispatch(req, { doGet, doPost }) };
export async function dispatch(request, handlers) {
  handlers = handlers || {};
  const {doGet, doPost, main, ...rest} = handlers;
  const method = request.method || 'GET';
  let url; try{url=new URL(request.url);}catch{url={searchParams:{forEach:()=>{}},search:''};}
  let result, body = '';
  if (method !== 'GET') { try{body=await request.text();}catch{} }

  if (method === 'GET' && typeof doGet === 'function') {
    const params = {};
    url.searchParams?.forEach((v,k) => params[k]=v);
    result = await doGet({parameter:params, queryString:url.search?.slice(1)||''});
  } else if (method === 'POST') {
    let dispatched = false;
    if (body) {
      try {
        const j = JSON.parse(body);
        const fn = j['function'] && (rest[j['function']] || globalThis[j['function']]);
        if (typeof fn === 'function') { result = await fn(...(j.args||[])); dispatched = true; }
      } catch {}
    }
    if (!dispatched && typeof doPost === 'function')
      result = await doPost({postData:{contents:body,type:request.headers?.get('content-type')||''},parameter:{}});
  }

  if (result === undefined && typeof main === 'function') result = await main();

  await __gFlush();

  if (result == null) return new Response('', {status:204});
  if (result instanceof Response) return result;
  if (typeof result === 'string') return new Response(result, {headers:{'Content-Type':'text/plain'}});
  if (typeof result?.getContent === 'function') return new Response(result.getContent(), {headers:{'Content-Type':'text/html'}});
  return new Response(JSON.stringify(result), {headers:{'Content-Type':'application/json'}});
}

export { SpreadsheetApp, DocumentApp, DriveApp, UrlFetchApp, Logger, PropertiesService, CacheService, Utilities };
export default { dispatch, SpreadsheetApp, DocumentApp, DriveApp, UrlFetchApp, Logger, PropertiesService, CacheService, Utilities };
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_is_non_empty() {
        assert!(!GAS_SHIM_PREFIX.is_empty());
    }

    #[test]
    fn suffix_is_non_empty() {
        assert!(!GAS_SHIM_SUFFIX.is_empty());
    }

    #[test]
    fn module_code_is_non_empty() {
        assert!(!GAS_MODULE_CODE.is_empty());
        assert_eq!(module_code(), GAS_MODULE_CODE);
    }

    #[test]
    fn prefix_contains_core_globals() {
        assert!(GAS_SHIM_PREFIX.contains("SpreadsheetApp"));
        assert!(GAS_SHIM_PREFIX.contains("PropertiesService"));
        assert!(GAS_SHIM_PREFIX.contains("CacheService"));
        assert!(GAS_SHIM_PREFIX.contains("Utilities"));
        assert!(GAS_SHIM_PREFIX.contains("DocumentApp"));
        assert!(GAS_SHIM_PREFIX.contains("DriveApp"));
        assert!(GAS_SHIM_PREFIX.contains("UrlFetchApp"));
        assert!(GAS_SHIM_PREFIX.contains("Logger"));
    }

    #[test]
    fn prefix_captures_original_fetch() {
        assert!(GAS_SHIM_PREFIX.contains("__gasFetch = fetch"));
    }

    #[test]
    fn suffix_installs_user_fetch_handler() {
        assert!(GAS_SHIM_SUFFIX.contains("__nano_user_fetch"));
        assert!(GAS_SHIM_SUFFIX.contains("doGet"));
        assert!(GAS_SHIM_SUFFIX.contains("doPost"));
        assert!(GAS_SHIM_SUFFIX.contains("__gasFlush"));
    }

    #[test]
    fn module_code_has_esm_exports() {
        assert!(GAS_MODULE_CODE.contains("export async function dispatch"));
        assert!(GAS_MODULE_CODE.contains("export default"));
        assert!(GAS_MODULE_CODE.contains("export {"));
    }

    #[test]
    fn stubs_defined_in_prefix() {
        assert!(GAS_SHIM_PREFIX.contains("GmailApp"));
        assert!(GAS_SHIM_PREFIX.contains("CalendarApp"));
        assert!(GAS_SHIM_PREFIX.contains("FormApp"));
        assert!(GAS_SHIM_PREFIX.contains("SlidesApp"));
        assert!(GAS_SHIM_PREFIX.contains("MailApp"));
    }

    #[test]
    fn token_cache_uses_correct_ttl() {
        // 3500 seconds (slightly under 1h) to avoid serving a token right before expiry
        assert!(GAS_SHIM_PREFIX.contains("3500000"));
        assert!(GAS_MODULE_CODE.contains("3500000"));
    }

    #[test]
    fn properties_service_uses_kv_globals() {
        assert!(GAS_SHIM_PREFIX.contains("__nano_kv_get"));
        assert!(GAS_SHIM_PREFIX.contains("__nano_kv_set"));
        assert!(GAS_SHIM_PREFIX.contains("__nano_kv_delete"));
        assert!(GAS_SHIM_PREFIX.contains("__nano_kv_list"));
    }

    #[test]
    fn sheets_api_uses_batch_update() {
        // Writes are batched, not sent one-by-one
        assert!(GAS_SHIM_PREFIX.contains("batchUpdate"));
        assert!(GAS_MODULE_CODE.contains("batchUpdate"));
    }
}
