'use strict';
// GAS suite — PropertiesService, Logger, Utilities, Session, dispatch.
// Uses a .gs extension so nano-rs auto-detects the flavor and routes through the GAS shim.

const GS_CODE = `
function doGet(e) {
  var t = e.parameter && e.parameter.t;

  switch (t) {
    case 'props-set-get': {
      var p = PropertiesService.getScriptProperties();
      p.setProperty('suite-key', 'suite-val');
      return p.getProperty('suite-key');
    }

    case 'props-delete': {
      var p = PropertiesService.getScriptProperties();
      p.setProperty('del-key', 'x');
      p.deleteProperty('del-key');
      return String(p.getProperty('del-key') === null);
    }

    case 'props-all': {
      var p = PropertiesService.getScriptProperties();
      p.setProperty('k1', 'v1');
      p.setProperty('k2', 'v2');
      var all = p.getProperties();
      return String(typeof all === 'object');
    }

    case 'uuid': {
      var id = Utilities.getUuid();
      return String(id.length === 36 && id.split('-').length === 5);
    }

    case 'base64': {
      var enc = Utilities.base64Encode('hello');
      return enc;
    }

    case 'base64-decode': {
      var dec = Utilities.base64Decode('aGVsbG8=');
      return String(Array.from(dec).map(function(b){return String.fromCharCode(b);}).join(''));
    }

    case 'logger': {
      Logger.log('GAS suite check');
      return 'logged';
    }

    case 'session': {
      return Session.getEffectiveUser().getEmail();
    }

    case 'cache': {
      var c = CacheService.getScriptCache();
      c.put('ck', 'cv', 60);
      return c.get('ck') || 'null';
    }

    default:
      return 'unknown';
  }
}

function doPost(e) {
  var body;
  try { body = JSON.parse(e.postData.contents); } catch(_) { return 'bad-json'; }
  return JSON.stringify({ echo: body });
}

function directFn() {
  return 'direct-called';
}
`;

module.exports = async function gas({ startServer, stopServer, request, delay }) {
  const PORT = 9380;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  function skip(name, reason) {
    tests.push({ name, passed: null, got: 'skipped', expected: reason, latency: 0, skipped: true });
  }

  let srv;
  try {
    srv = await startServer(GS_CODE, PORT, {
      ext: '.gs', // .gs extension auto-selects the GAS shim (compat: auto)
    });

    const get = (q) => request(PORT, `/?t=${q}`);

    // Run tests — each may fail if GAS shim has runtime issues
    const propsSetGet = await get('props-set-get');
    t('PropertiesService.setProperty/getProperty', propsSetGet, 'suite-val');

    t('PropertiesService.deleteProperty', await get('props-delete'), 'true');
    t('PropertiesService.getProperties', await get('props-all'), 'true');

    const uuid = await get('uuid');
    t('Utilities.getUuid format', uuid, 'true');

    t('Utilities.base64Encode', await get('base64'), 'aGVsbG8=');
    t('Utilities.base64Decode', await get('base64-decode'), 'hello');
    t('Logger.log', await get('logger'), 'logged');
    t('Session.getEffectiveUser().getEmail()', await get('session'), 'service-account@nano.local');
    t('CacheService.put/get', await get('cache'), 'cv');

    // Direct dispatch via POST
    const directDispatch = await request(PORT, '/', {
      method: 'POST',
      body: JSON.stringify({ function: 'directFn', args: [] }),
      headers: { 'content-type': 'application/json', Host: 'localhost' },
    });
    t('Direct function dispatch', directDispatch, 'direct-called');

    // doPost with JSON body
    const doPost = await request(PORT, '/', {
      method: 'POST',
      body: JSON.stringify({ hello: 'world' }),
      headers: { 'content-type': 'application/json', Host: 'localhost' },
    });
    const doPostBody = (() => { try { return JSON.parse(doPost.body); } catch (_) { return null; } })();
    const doPostPassed = doPost.status === 200 && doPostBody && doPostBody.echo && doPostBody.echo.hello === 'world';
    tests.push({ name: 'doPost JSON echo', passed: doPostPassed, got: doPost.body, expected: '{"echo":{"hello":"world"}}', latency: doPost.latency });

  } finally {
    await stopServer(srv);
  }

  return { name: 'Google Apps Script (.gs auto-detect)', tests, memMb: null };
};
