/**
 * node-compat.js — Node.js compat layer demo
 *
 * Shows require('path'), require('buffer'), require('assert'), and process.env
 * working in a WinterTC worker — the same APIs used by common npm packages.
 *
 * Run:
 *   nano-rs run -c examples/configs/node-compat.json
 *
 * Try it:
 *   curl http://localhost:8080/path
 *   → {"joined":"/var/app/config.json","dir":"/var/app","base":"config.json","ext":".json"}
 *
 *   curl http://localhost:8080/buffer
 *   → {"text":"hello world","isBuffer":true}
 *
 *   curl http://localhost:8080/assert
 *   → {"ok":true}
 *
 *   curl http://localhost:8080/env
 *   → {"node_env":"not set","version":"v18.0.0"}
 */

const path = require('path');
const { from: bufferFrom, isBuffer, concat } = require('buffer');
const assert = require('assert');

export default {
  async fetch(request) {
    const url = new URL(request.url);

    if (url.pathname === '/path') {
      const joined = path.join('/var', 'app', 'config.json');
      const dir = path.dirname(joined);
      const base = path.basename(joined);
      const ext = path.extname(joined);
      return new Response(JSON.stringify({ joined, dir, base, ext }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/buffer') {
      const a = bufferFrom('hello ');
      const b = bufferFrom('world');
      const combined = concat([a, b]);
      const text = new TextDecoder().decode(combined);
      return new Response(JSON.stringify({ text, isBuffer: isBuffer(a) }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/assert') {
      try {
        assert.equal(1 + 1, 2);
        assert.ok(true, 'ok works');
        return new Response(JSON.stringify({ ok: true }), {
          headers: { 'content-type': 'application/json' },
        });
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), {
          status: 500,
          headers: { 'content-type': 'application/json' },
        });
      }
    }

    if (url.pathname === '/env') {
      // process.env exposes host environment variables
      const node_env = process.env.NODE_ENV ?? 'not set';
      const version = process.version;
      return new Response(JSON.stringify({ node_env, version }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    return new Response('Not Found', { status: 404 });
  },
};
