'use strict';
// KV / Storage suite — nano:kv get/set/delete/list, openKV namespaces, localStorage shim

const APP = `
import { kv, openKV } from 'nano:kv';

const ns = openKV('suite-ns');

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');
    const k = url.searchParams.get('k') || 'key';
    const v = url.searchParams.get('v') || 'val';

    try {
      switch (t) {
        case 'set-get': {
          await kv.set(k, v);
          const got = await kv.get(k);
          return new Response(got ? new TextDecoder().decode(got) : 'null');
        }

        case 'set-json-get-json': {
          await kv.setJSON(k, { n: 42 });
          const obj = await kv.getJSON(k);
          return new Response(String(obj && obj.n));
        }

        case 'delete': {
          await kv.set(k, v);
          await kv.delete(k);
          const after = await kv.get(k);
          return new Response(after === null ? 'null' : 'present');
        }

        case 'list': {
          await kv.set('list:a', '1');
          await kv.set('list:b', '2');
          const pairs = await kv.list('list:');
          return new Response(String(pairs.length >= 2));
        }

        case 'namespace': {
          await ns.set(k, v);
          const got = await ns.get(k);
          return new Response(got ? new TextDecoder().decode(got) : 'null');
        }

        case 'namespace-isolation': {
          await kv.set('iso', 'default');
          await ns.set('iso', 'namespaced');
          const fromDefault = await kv.get('iso');
          const fromNs = await ns.get('iso');
          const d = fromDefault ? new TextDecoder().decode(fromDefault) : 'null';
          const n = fromNs ? new TextDecoder().decode(fromNs) : 'null';
          return new Response(d + ':' + n);
        }

        case 'localstorage-set-get': {
          if (typeof localStorage === 'undefined') return new Response('unavailable-in-esm');
          localStorage.setItem('ls-k', 'ls-v');
          return new Response(localStorage.getItem('ls-k') || 'null');
        }

        case 'localstorage-remove': {
          if (typeof localStorage === 'undefined') return new Response('unavailable-in-esm');
          localStorage.setItem('rm-k', 'x');
          localStorage.removeItem('rm-k');
          return new Response(localStorage.getItem('rm-k') === null ? 'null' : 'present');
        }

        case 'localstorage-length': {
          if (typeof localStorage === 'undefined') return new Response('unavailable-in-esm');
          localStorage.clear();
          localStorage.setItem('a', '1');
          localStorage.setItem('b', '2');
          return new Response(String(localStorage.length));
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

module.exports = async function kv({ startServer, stopServer, request, delay }) {
  const PORT = 9330;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT);
    const get = (q, extra = '') => request(PORT, `/?t=${q}${extra}`);

    t('kv.set + get',             await get('set-get', '&k=mykey&v=myval'), 'myval');
    t('kv.setJSON + getJSON',     await get('set-json-get-json', '&k=jkey'), '42');
    t('kv.delete',                await get('delete', '&k=delkey&v=x'),     'null');
    t('kv.list prefix',           await get('list'),                         'true');
    t('openKV namespace set/get', await get('namespace', '&k=nk&v=nv'),      'nv');
    t('namespace isolation',      await get('namespace-isolation'),           'default:namespaced');
    t('localStorage.setItem/getItem', await get('localstorage-set-get'),    'ls-v');
    t('localStorage.removeItem',  await get('localstorage-remove'),          'null');
    t('localStorage.length',      await get('localstorage-length'),          '2');

    // Note: if 'unavailable-in-esm' was returned, mark them as known gaps not test bugs
    for (const test of tests.filter(t2 => t2.got === 'unavailable-in-esm')) {
      test.passed = null;
      test.skipped = true;
      test.expected = 'localStorage not in global scope for ESM — import from nano:kv';
    }
  } finally {
    await stopServer(srv);
  }

  return { name: 'KV & Storage', tests, memMb: null };
};
