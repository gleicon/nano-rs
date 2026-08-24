'use strict';
// Node.js compat suite — require('path'), require('buffer'), process.env, assert, stream basics

const APP = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');

    try {
      switch (t) {
        case 'path-join':
          return new Response(require('path').join('/a', 'b', 'c.txt'));

        case 'path-dirname':
          return new Response(require('path').dirname('/a/b/c.txt'));

        case 'path-basename':
          return new Response(require('path').basename('/a/b/c.txt'));

        case 'path-extname':
          return new Response(require('path').extname('/a/b/c.txt'));

        case 'buffer-from': {
          const { from } = require('buffer');
          return new Response(from('hello').toString());
        }

        case 'buffer-is': {
          const { from, isBuffer } = require('buffer');
          return new Response(String(isBuffer(from('x'))));
        }

        case 'buffer-concat': {
          const { from, concat } = require('buffer');
          return new Response(concat([from('fo'), from('o')]).toString());
        }

        case 'buffer-hex': {
          const { from } = require('buffer');
          return new Response(from([0xde, 0xad]).toString('hex'));
        }

        case 'process-version':
          return new Response(process.version);

        case 'process-env': {
          const v = process.env.SUITE_TEST_VAR || 'absent';
          return new Response(v);
        }

        case 'assert-ok': {
          const assert = require('assert');
          assert.ok(true);
          assert.equal(1 + 1, 2);
          return new Response('ok');
        }

        case 'assert-throws': {
          const assert = require('assert');
          try {
            assert.equal(1, 2);
            return new Response('no-throw');
          } catch (_) {
            return new Response('threw');
          }
        }

        case 'events': {
          const { EventEmitter } = require('events');
          const ee = new EventEmitter();
          let fired = false;
          ee.on('x', () => { fired = true; });
          ee.emit('x');
          return new Response(String(fired));
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

module.exports = async function nodeCompat({ startServer, stopServer, request, delay }) {
  const PORT = 9320;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT);
    const get = (q) => request(PORT, `/?t=${q}`);

    t('path.join',            await get('path-join'),     '/a/b/c.txt');
    t('path.dirname',         await get('path-dirname'),  '/a/b');
    t('path.basename',        await get('path-basename'), 'c.txt');
    t('path.extname',         await get('path-extname'),  '.txt');
    t('Buffer.from().toString()', await get('buffer-from'), 'hello');
    t('Buffer.isBuffer()',    await get('buffer-is'),     'true');
    t('Buffer.concat()',      await get('buffer-concat'), 'foo');
    t('Buffer hex encoding',  await get('buffer-hex'),    'dead');
    t('process.version',      await get('process-version'), /^v\d+/.test((await get('process-version')).body) ? (await get('process-version')).body : '__check__');
    t('process.env absent',   await get('process-env'),   'absent');
    t('assert.ok / equal',    await get('assert-ok'),     'ok');
    t('assert throws on mismatch', await get('assert-throws'), 'threw');
    t('EventEmitter',         await get('events'),        'true');
  } finally {
    await stopServer(srv);
  }

  // Fix the process.version test — it just needs to start with 'v'
  const verTest = tests.find((x) => x.name === 'process.version');
  if (verTest) {
    verTest.passed = /^v\d+/.test(verTest.got);
    verTest.expected = 'v<semver>';
  }

  return { name: 'Node.js Compat', tests, memMb: null };
};
