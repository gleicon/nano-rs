/**
 * localStorage-shim.js — localStorage built on nano:kv
 *
 * A drop-in shim for browser localStorage semantics.
 * Import this and use localStorage.getItem / setItem / removeItem / clear.
 *
 * This is a standalone module — copy it into your app or import it with
 * a relative path:
 *
 *   import './localStorage-shim.js';
 *   localStorage.setItem('theme', 'dark');
 *
 * Keys are stored in the 'localStorage' KV namespace, hostname-scoped.
 */

import { openKV } from 'nano:kv';

const store = openKV('localStorage');

const localStorage = {
  async getItem(key) {
    const bytes = await store.get(String(key));
    return bytes ? new TextDecoder().decode(bytes) : null;
  },

  async setItem(key, value) {
    await store.set(String(key), new TextEncoder().encode(String(value)));
  },

  async removeItem(key) {
    await store.delete(String(key));
  },

  async clear() {
    const entries = await store.list('');
    await Promise.all(entries.map(([k]) => store.delete(k)));
  },

  async length() {
    const entries = await store.list('');
    return entries.length;
  },

  async key(index) {
    const entries = await store.list('');
    return index < entries.length ? entries[index][0] : null;
  },
};

globalThis.localStorage = localStorage;

export { localStorage };

// ─── Demo handler ────────────────────────────────────────────────────────────

export default {
  async fetch(request) {
    const url = new URL(request.url);

    if (url.pathname === '/set') {
      const key = url.searchParams.get('key') ?? 'test';
      const value = url.searchParams.get('value') ?? 'hello';
      await localStorage.setItem(key, value);
      return new Response(JSON.stringify({ ok: true, key, value }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/get') {
      const key = url.searchParams.get('key') ?? 'test';
      const value = await localStorage.getItem(key);
      return new Response(JSON.stringify({ key, value }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    return new Response('Use /set?key=k&value=v or /get?key=k', { status: 200 });
  },
};
