'use strict';
// WinterTC suite — fetch API, Request/Response/Headers, body handling, status codes, ReadableStream

const APP = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');

    try {
      switch (t) {
        case 'echo-method':
          return new Response(request.method);

        case 'echo-header':
          return new Response(request.headers.get('x-custom') || 'missing');

        case 'echo-url':
          return new Response(request.url.includes('/') ? 'ok' : 'fail');

        case 'response-200':
          return new Response('body', { status: 200 });

        case 'response-404':
          return new Response('nope', { status: 404 });

        case 'response-headers': {
          const r = new Response('ok', { headers: { 'x-out': 'hello' } });
          return new Response(r.headers.get('x-out'));
        }

        case 'headers-append': {
          const h = new Headers();
          h.append('a', '1');
          h.append('a', '2');
          return new Response(h.get('a'));
        }

        case 'headers-has': {
          const h = new Headers({ 'x-test': 'y' });
          return new Response(String(h.has('x-test')));
        }

        case 'headers-delete': {
          const h = new Headers({ 'x-del': 'v' });
          h.delete('x-del');
          return new Response(String(h.has('x-del')));
        }

        case 'body-text':
          return new Response(await request.text());

        case 'body-json': {
          const j = await request.json();
          return new Response(String(j.n));
        }

        case 'clone': {
          const cloned = request.clone();
          return new Response(cloned.method + ':' + (cloned.url ? 'ok' : 'fail'));
        }

        case 'new-request': {
          const r = new Request('http://test.com/path', { method: 'POST', body: 'hi' });
          return new Response(r.method + ':' + new URL(r.url).pathname);
        }

        case 'response-ok':
          return new Response(String(new Response('', { status: 200 }).ok));

        case 'response-not-ok':
          return new Response(String(new Response('', { status: 404 }).ok));

        case 'response-json': {
          const r = Response.json({ x: 42 });
          const j = await r.json();
          return new Response(String(j.x));
        }

        default:
          return new Response('unknown', { status: 404 });
      }
    } catch (e) {
      return new Response('ERROR:' + e.message, { status: 500 });
    }
  }
};
`;

module.exports = async function wintertc({ startServer, stopServer, request, delay }) {
  const PORT = 9310;
  const tests = [];

  function t(name, res, expected, expectedStatus = 200) {
    const pass = res.status === expectedStatus && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT);
    const get = (q, opts) => request(PORT, `/?t=${q}`, opts);

    t('request.method GET',    await get('echo-method'), 'GET');
    t('request.method POST',   await request(PORT, '/?t=echo-method', { method: 'POST' }), 'POST');
    t('request.headers.get',   await get('echo-header', { headers: { 'x-custom': 'world', Host: 'localhost' } }), 'world');
    t('request.url contains /', await get('echo-url'), 'ok');
    t('Response 200',          await get('response-200'), 'body', 200);
    t('Response 404',          await get('response-404'), 'nope', 404);
    t('Response custom header', await get('response-headers'), 'hello');
    t('Headers.append multi',  await get('headers-append'), '1, 2');
    t('Headers.has true',      await get('headers-has'), 'true');
    t('Headers.delete → false', await get('headers-delete'), 'false');
    t('request.text()',        await request(PORT, '/?t=body-text', { method: 'POST', body: 'ping' }), 'ping');
    t('request.json()',        await request(PORT, '/?t=body-json', { method: 'POST', body: '{"n":7}', headers: { 'content-type': 'application/json', Host: 'localhost' } }), '7');
    t('request.clone()',       await get('clone'), 'GET:ok');
    t('new Request()',         await get('new-request'), 'POST:/path');
    t('Response.ok true',      await get('response-ok'), 'true');
    t('Response.ok false',     await get('response-not-ok'), 'false');
    t('Response.json()',       await get('response-json'), '42');
  } finally {
    await stopServer(srv);
  }

  return { name: 'WinterTC / Fetch API', tests, memMb: null };
};
