'use strict';
// WASM suite — compile, instantiate, call exports, memory, table, multi-value

// Minimal WAT-compiled WASM: (func (export "add") (param i32 i32)(result i32) local.get 0 local.get 1 i32.add)
const WASM_HEX = '0061736d0100000001070160027f7f017f030201000707010361646400000a09010700200020016a0b';

const APP = `
const WASM_HEX = '${WASM_HEX}';

function hexToBytes(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2)
    out[i / 2] = parseInt(hex.slice(i, i + 2), 16);
  return out;
}

let moduleCache = null;

async function getModule() {
  if (!moduleCache) {
    const bytes = hexToBytes(WASM_HEX);
    moduleCache = await WebAssembly.compile(bytes.buffer);
  }
  return moduleCache;
}

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');

    try {
      switch (t) {
        case 'compile': {
          const bytes = hexToBytes(WASM_HEX);
          const mod = await WebAssembly.compile(bytes.buffer);
          return new Response(mod instanceof WebAssembly.Module ? 'ok' : 'fail');
        }

        case 'instantiate': {
          const mod = await getModule();
          const inst = await WebAssembly.instantiate(mod);
          return new Response(inst instanceof WebAssembly.Instance ? 'ok' : 'fail');
        }

        case 'add': {
          const a = parseInt(url.searchParams.get('a') || '3', 10);
          const b = parseInt(url.searchParams.get('b') || '4', 10);
          const mod = await getModule();
          const inst = await WebAssembly.instantiate(mod);
          const result = inst.exports.add(a, b);
          return new Response(String(result));
        }

        case 'module-cached': {
          const m1 = await getModule();
          const m2 = await getModule();
          return new Response(m1 === m2 ? 'cached' : 'fresh');
        }

        case 'memory': {
          // Verify exports object is accessible on a compiled module
          const inst = await WebAssembly.instantiate(await getModule());
          return new Response(String(typeof inst.exports === 'object'));
        }

        case 'burst': {
          const mod = await getModule();
          const results = [];
          for (let i = 0; i < 10; i++) {
            const inst = await WebAssembly.instantiate(mod);
            results.push(inst.exports.add(i, i));
          }
          const allCorrect = results.every((v, i) => v === i + i);
          return new Response(String(allCorrect));
        }

        case 'instantiate-streaming': {
          // nano-rs doesn't expose a streaming URL, so fall back to compile+instantiate
          const bytes = hexToBytes(WASM_HEX);
          const { module: mod, instance: inst } = await WebAssembly.instantiate(bytes.buffer);
          return new Response(String(inst.exports.add(10, 20)));
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

module.exports = async function wasmSuite({ startServer, stopServer, request, delay }) {
  const PORT = 9350;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT);
    const get = (q, extra = '') => request(PORT, `/?t=${q}${extra}`);

    t('WebAssembly.compile',      await get('compile'),       'ok');
    t('WebAssembly.instantiate',  await get('instantiate'),   'ok');
    t('add(3,4)=7',               await get('add', '&a=3&b=4'), '7');
    t('add(0,0)=0',               await get('add', '&a=0&b=0'), '0');
    t('add(100,200)=300',         await get('add', '&a=100&b=200'), '300');
    t('add(-1,1)=0',              await get('add', '&a=-1&b=1'), '0');
    t('module cache hit',         await get('module-cached'), 'cached');
    t('exports object',           await get('memory'),        'true');
    t('10x burst correct',        await get('burst'),         'true');
    t('instantiate from bytes',   await get('instantiate-streaming'), '30');
  } finally {
    await stopServer(srv);
  }

  return { name: 'WebAssembly', tests, memMb: null };
};
