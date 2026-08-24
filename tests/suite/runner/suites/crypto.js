'use strict';
// Crypto suite — crypto.subtle (digest, HMAC, AES-GCM), getRandomValues, btoa/atob

const APP = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const t = url.searchParams.get('t');

    try {
      switch (t) {
        case 'random-values': {
          const buf = new Uint8Array(16);
          crypto.getRandomValues(buf);
          return new Response(String(buf.length));
        }

        case 'random-nonzero': {
          const buf = new Uint8Array(32);
          crypto.getRandomValues(buf);
          const nonzero = buf.some(b => b !== 0);
          return new Response(String(nonzero));
        }

        case 'sha256': {
          const enc = new TextEncoder().encode('hello');
          const hash = await crypto.subtle.digest('SHA-256', enc);
          return new Response(String(new Uint8Array(hash).length));
        }

        case 'sha256-value': {
          const enc = new TextEncoder().encode('abc');
          const hash = await crypto.subtle.digest('SHA-256', enc);
          const hex = Array.from(new Uint8Array(hash)).map(b => b.toString(16).padStart(2,'0')).join('');
          return new Response(hex.startsWith('ba7816bf') ? 'ok' : 'fail:' + hex.slice(0,8));
        }

        case 'aes-gcm-keygen': {
          const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, true, ['encrypt', 'decrypt']);
          return new Response(key ? 'ok' : 'fail');
        }

        case 'aes-gcm-roundtrip': {
          const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, true, ['encrypt', 'decrypt']);
          const iv = crypto.getRandomValues(new Uint8Array(12));
          const plain = new TextEncoder().encode('secret');
          const ct = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plain);
          const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, ct);
          return new Response(new TextDecoder().decode(pt));
        }

        case 'hmac': {
          const key = await crypto.subtle.generateKey({ name: 'HMAC', hash: 'SHA-256' }, true, ['sign', 'verify']);
          const data = new TextEncoder().encode('msg');
          const sig = await crypto.subtle.sign('HMAC', key, data);
          const valid = await crypto.subtle.verify('HMAC', key, sig, data);
          return new Response(String(valid));
        }

        case 'pbkdf2': {
          const base = await crypto.subtle.importKey('raw', new TextEncoder().encode('pass'), 'PBKDF2', false, ['deriveBits']);
          const bits = await crypto.subtle.deriveBits(
            { name: 'PBKDF2', hash: 'SHA-256', salt: new Uint8Array(16), iterations: 1000 },
            base, 256
          );
          return new Response(String(new Uint8Array(bits).length));
        }

        case 'btoa-atob': {
          const encoded = btoa('nano-rs');
          const decoded = atob(encoded);
          return new Response(decoded);
        }

        case 'subtle-type':
          return new Response(String(typeof crypto.subtle));

        default:
          return new Response('unknown', { status: 404 });
      }
    } catch (e) {
      return new Response('ERROR:' + e.message, { status: 500 });
    }
  }
};
`;

module.exports = async function cryptoSuite({ startServer, stopServer, request, delay }) {
  const PORT = 9340;
  const tests = [];

  function t(name, res, expected) {
    const pass = res.status === 200 && res.body === expected;
    tests.push({ name, passed: pass, got: res.body, expected, latency: res.latency });
  }

  let srv;
  try {
    srv = await startServer(APP, PORT);
    const get = (q) => request(PORT, `/?t=${q}`);

    t('crypto.subtle type',        await get('subtle-type'),        'object');
    t('getRandomValues length',    await get('random-values'),      '16');
    t('getRandomValues non-zero',  await get('random-nonzero'),     'true');
    t('SHA-256 digest size',       await get('sha256'),             '32');
    t('SHA-256 known value',       await get('sha256-value'),       'ok');
    t('AES-GCM keygen',            await get('aes-gcm-keygen'),     'ok');
    t('AES-GCM encrypt→decrypt',   await get('aes-gcm-roundtrip'),  'secret');
    t('HMAC sign+verify',          await get('hmac'),               'true');
    t('PBKDF2 deriveBits',         await get('pbkdf2'),             '32');
    t('btoa→atob roundtrip',       await get('btoa-atob'),          'nano-rs');
  } finally {
    await stopServer(srv);
  }

  return { name: 'Crypto', tests, memMb: null };
};
