/**
 * kv-namespaced.js — multiple KV namespaces with JSON helpers
 *
 * Shows openKV() for separate KV stores within one app.
 *
 * Run:
 *   nano-rs run -c examples/configs/kv-namespaced.json
 *
 * Try it:
 *   # Store a value
 *   curl -X POST http://localhost:8080/set \
 *        -H 'content-type: application/json' \
 *        -d '{"key":"color","value":"blue"}'
 *   → {"ok":true}
 *
 *   # Read it back
 *   curl "http://localhost:8080/get?key=color"
 *   → {"key":"color","value":"blue"}
 *
 *   # List all keys with a prefix
 *   curl "http://localhost:8080/list?prefix=col"
 *   → [{"key":"color","value":"blue"}]
 */

import { kv, openKV } from 'nano:kv';

const cache = openKV('cache');
const sessions = openKV('sessions');

export default {
  async fetch(request) {
    const url = new URL(request.url);

    if (url.pathname === '/set' && request.method === 'POST') {
      const body = await request.json();
      await cache.setJSON(body.key, body.value);
      return new Response(JSON.stringify({ ok: true }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/get') {
      const key = url.searchParams.get('key');
      const value = await cache.getJSON(key);
      return new Response(JSON.stringify({ key, value }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/list') {
      const prefix = url.searchParams.get('prefix') ?? '';
      const entries = await cache.list(prefix);
      const result = entries.map(([k, v]) => ({
        key: k,
        value: new TextDecoder().decode(v),
      }));
      return new Response(JSON.stringify(result), {
        headers: { 'content-type': 'application/json' },
      });
    }

    return new Response('Not Found', { status: 404 });
  },
};
