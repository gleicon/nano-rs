/**
 * kv-counter.js — persistent request counter using nano:kv
 *
 * Each request increments a counter stored in EdgeStore.
 * The count persists across restarts.
 *
 * Run:
 *   nano-rs run -c examples/configs/kv-counter.json
 *
 * Test:
 *   curl http://localhost:8080/
 */

import { kv } from 'nano:kv';

export default {
  async fetch(request) {
    const raw = await kv.get('hits');
    const hits = raw ? parseInt(new TextDecoder().decode(raw), 10) : 0;
    const next = hits + 1;
    await kv.set('hits', new TextEncoder().encode(String(next)));

    return new Response(JSON.stringify({ hits: next }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  },
};
