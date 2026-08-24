'use strict';
// Foundation suite — URL, TextEncoder/Decoder, fetch API types, console, timers, btoa/atob

const APP = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');

    try {
      switch (t) {
        case 'url-pathname':  return new Response(new URL('http://x.com/path').pathname);
        case 'url-search':    return new Response(new URL('http://x.com/?a=1').search);
        case 'url-params':    return new Response(new URL('http://x.com/?k=v').searchParams.get('k'));
        case 'text-enc':      return new Response(String(new TextEncoder().encode('hi') instanceof Uint8Array));
        case 'text-dec':      return new Response(new TextDecoder().decode(new Uint8Array([104,105])));
        case 'response-type': return new Response(String(typeof Response));
        case 'request-type':  return new Response(String(typeof Request));
        case 'headers-type':  return new Response(String(typeof Headers));
        case 'fetch-type':    return new Response(String(typeof fetch));
        case 'console-log':   console.log('suite-check'); return new Response('ok');
        case 'json-parse':    return new Response(String(JSON.parse('{"x":1}').x));
        case 'btoa':          return new Response(btoa('hello'));
        case 'atob':          return new Response(atob('aGVsbG8='));
        case 'timeout-type':  return new Response(String(typeof setTimeout));
        case 'interval-type': return new Response(String(typeof setInterval));
        case 'date-now':      return new Response(String(typeof Date.now() === 'number'));
        case 'structuredclone': return new Response(JSON.stringify(structuredClone({a:1})));
        case 'promise':       return new Response(await Promise.resolve('resolved'));
        case 'uint8':         return new Response(String(new Uint8Array([1,2,3]).length));
        case 'arraybuffer':   return new Response(String(new ArrayBuffer(8).byteLength));
        default:              return new Response('unknown', { status: 404 });
      }
    } catch (e) {
      return new Response('ERROR:' + e.message, { status: 500 });
    }
  }
};
`;

module.exports = async function foundation({ startServer, stopServer, request, delay }) {
  const PORT = 9300;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT);
    const get = (q) => request(PORT, `/?t=${q}`);

    t('URL pathname',         await get('url-pathname'),   '/path');
    t('URL search',           await get('url-search'),     '?a=1');
    t('URLSearchParams.get',  await get('url-params'),     'v');
    t('TextEncoder→Uint8Array', await get('text-enc'),     'true');
    t('TextDecoder.decode',   await get('text-dec'),       'hi');
    t('Response type',        await get('response-type'),  'function');
    t('Request type',         await get('request-type'),   'function');
    t('Headers type',         await get('headers-type'),   'function');
    t('fetch type',           await get('fetch-type'),     'function');
    t('console.log',          await get('console-log'),    'ok');
    t('JSON.parse',           await get('json-parse'),     '1');
    t('btoa',                 await get('btoa'),           'aGVsbG8=');
    t('atob',                 await get('atob'),           'hello');
    t('setTimeout type',      await get('timeout-type'),   'function');
    t('setInterval type',     await get('interval-type'),  'function');
    t('Date.now',             await get('date-now'),       'true');
    t('structuredClone',      await get('structuredclone'),'{"a":1}');
    t('Promise.resolve',      await get('promise'),        'resolved');
    t('Uint8Array',           await get('uint8'),          '3');
    t('ArrayBuffer',          await get('arraybuffer'),    '8');
  } finally {
    await stopServer(srv);
  }

  return { name: 'Foundation APIs', tests, memMb: null };
};
