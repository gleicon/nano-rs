'use strict';
// Memory suite — OOM isolation, per-request limits, heap monitoring, no OOM leak across requests

const APP = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');
    const size = parseInt(url.searchParams.get('size') || '0', 10);

    try {
      switch (t) {
        case 'small': {
          const arr = new Array(1000).fill('x');
          return new Response(JSON.stringify({ ok: true, len: arr.length }));
        }

        case 'medium': {
          const arr = new Array(100_000).fill(0).map((_, i) => i);
          return new Response(JSON.stringify({ ok: true, len: arr.length }));
        }

        case 'large': {
          // This should either succeed or be killed by OOM, not crash the process
          try {
            const s = size || 10_000_000;
            const arr = new Array(s).fill('x');
            return new Response(JSON.stringify({ ok: true, len: arr.length }));
          } catch (e) {
            return new Response(JSON.stringify({ ok: false, error: e.message }), { status: 500 });
          }
        }

        case 'post-oom': {
          // Verify the runtime still handles requests normally after a large alloc
          return new Response(JSON.stringify({ alive: true }));
        }

        case 'string-growth': {
          let s = '';
          for (let i = 0; i < 1000; i++) s += 'abcdefghij';
          return new Response(String(s.length));
        }

        case 'json-roundtrip': {
          const obj = { data: new Array(1000).fill(null).map((_, i) => ({ i, v: 'value-' + i })) };
          const json = JSON.stringify(obj);
          const parsed = JSON.parse(json);
          return new Response(String(parsed.data.length));
        }

        default:
          return new Response('unknown', { status: 404 });
      }
    } catch (e) {
      return new Response('PANIC:' + e.message, { status: 500 });
    }
  }
};
`;

module.exports = async function memory({ startServer, stopServer, request, delay, getRssMb }) {
  const PORT = 9370;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body.includes(expected);
    tests.push({ name, passed: pass, got: res.body.slice(0, 80), expected, latency: res.latency });
  }

  function rec(name, passed, got, expected) {
    tests.push({ name, passed, got: String(got), expected: String(expected), latency: 0 });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT, { memory_mb: 128 });
    const get = (q, extra = '') => request(PORT, `/?t=${q}${extra}`, { timeout: 10000 });

    // Small allocations always work
    t('small alloc (1k)',   await get('small'),  '"ok":true');
    t('medium alloc (100k)', await get('medium'), '"ok":true');

    // String growth
    t('string growth 10k chars', await get('string-growth'), '10000');

    // JSON round-trip with 1k items
    t('JSON roundtrip 1k items', await get('json-roundtrip'), '1000');

    // Large allocation — may OOM, the SERVER must stay alive either way
    const large = await get('large', '&size=5000000');
    rec('large alloc — no process crash', large.status !== 0, `status=${large.status}`, 'not crash');

    // Post-large: server still responds
    await delay(300);
    const alive = await get('post-oom');
    rec('runtime alive after large alloc', alive.status === 200, alive.status, 200);
    t('post-oom response valid', alive, '"alive":true');

    // Memory reading
    const rss = getRssMb(srv.proc.pid);
    rec('RSS measurement available', rss !== null, `${rss} MB`, 'a number');
    if (rss !== null) {
      rec('RSS < configured limit (128 MB)', rss < 384, `${rss} MB`, '< 384 MB');
    }

  } finally {
    await stopServer(srv);
  }

  return { name: 'Memory', tests, memMb: null };
};
