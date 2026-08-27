# NANO JavaScript API Reference

**Version:** v2.0a  
**Last Updated:** 2026-05-17

---

## Overview

NANO provides a WinterTC-compatible JavaScript runtime with additional NANO-specific APIs for filesystem access and edge-specific functionality.

**Quick Links:**
- [WinterTC APIs](#wintertc-apis) — Standard web APIs
- [WebCrypto](#webcrypto-api) — Cryptographic operations
- [WebSocket](#websocket-api) — WebSocket upgrade and messaging
- [NANO-Specific APIs](#nano-specific-apis) — Nano.fs.* for file access
- [Node.js Compatibility](#nodejs-compatibility-polyfills) — Buffer, timers, fs

---

## WinterTC APIs

### console

Standard console implementation with `log`, `error`, `warn` methods.

```javascript
console.log("Hello", "world");
console.error("Error:", err);
console.warn("Warning: deprecated API");
```

**Methods:**
- `console.log(...args)` — Log to stdout (structured JSON in production)
- `console.error(...args)` — Log to stderr
- `console.warn(...args)` — Alias for console.error with warning prefix

**Production Behavior:**
In production mode, console output is structured JSON:
```json
{"level":"info","message":"Hello world","timestamp":"2026-05-02T10:30:00Z"}
```

---

### TextEncoder / TextDecoder

WinterTC-standard encoding/decoding.

```javascript
const encoder = new TextEncoder();
const bytes = encoder.encode("Hello"); // Uint8Array

const decoder = new TextDecoder();
const str = decoder.decode(bytes); // "Hello"
```

**TextEncoder:**
- `encode(string)` → Uint8Array
- Always UTF-8

**TextDecoder:**
- `decode(uint8array)` → string
- Optional `encoding` parameter (default: 'utf-8')

---

### fetch() / Request / Response / Headers

Full WinterTC HTTP client implementation.

#### fetch()

```javascript
// GET request
const response = await fetch("https://api.example.com/data");
const data = await response.json();

// POST request with body
const response = await fetch("https://api.example.com/items", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ name: "item" })
});
```

**Options:**
| Option | Type | Description |
|--------|------|-------------|
| `method` | string | HTTP method (GET, POST, PUT, DELETE, etc.) |
| `headers` | Headers \| object | Request headers |
| `body` | string \| Uint8Array \| ReadableStream | Request body |
| `redirect` | string | "follow", "error", or "manual" |

#### Request

```javascript
// Request object
const request = new Request("https://example.com", {
  method: "POST",
  headers: new Headers({ "X-Custom": "value" }),
  body: "request body"
});

// From existing request (clone)
const newRequest = new Request(request);
```

**Properties:**
- `url` — Request URL
- `method` — HTTP method
- `headers` — Headers object
- `body` — Body stream

#### Response

```javascript
// Return from handler
return new Response("Hello", {
  status: 200,
  headers: { "Content-Type": "text/plain" }
});

// JSON response
return new Response(JSON.stringify({ hello: "world" }), {
  status: 200,
  headers: { "Content-Type": "application/json" }
});
```

**Constructor:**
```javascript
new Response(body, options)
```

**Options:**
| Option | Type | Description |
|--------|------|-------------|
| `status` | number | HTTP status code (default: 200) |
| `statusText` | string | Status text |
| `headers` | object \| Headers | Response headers |

**Methods:**
- `text()` → Promise<string>
- `json()` → Promise<any>
- `arrayBuffer()` → Promise<ArrayBuffer>
- `blob()` → Promise<Blob>

#### Headers

```javascript
const headers = new Headers();
headers.set("Content-Type", "application/json");
headers.append("X-Custom", "value");

// From object
const headers2 = new Headers({
  "Content-Type": "application/json"
});
```

**Methods:**
- `get(name)` → string \| null
- `set(name, value)`
- `append(name, value)`
- `delete(name)`
- `has(name)` → boolean
- `forEach(callback)`

**Case-insensitive:** Header names are case-insensitive per HTTP spec.

---

### URL / URLSearchParams

WinterTC URL manipulation.

#### URL

```javascript
const url = new URL("https://example.com/path?foo=bar");

console.log(url.protocol);  // "https:"
console.log(url.hostname);  // "example.com"
console.log(url.pathname);  // "/path"
console.log(url.search);    // "?foo=bar"
console.log(url.hash);      // ""

// Modify
url.searchParams.append("baz", "qux");
console.log(url.toString()); // "https://example.com/path?foo=bar&baz=qux"
```

**Properties:**
- `href` — Full URL string
- `protocol` — Protocol ("http:", "https:")
- `hostname` — Host without port
- `port` — Port number
- `pathname` — Path
- `search` — Query string ("?...")
- `hash` — Fragment ("#...")
- `searchParams` — URLSearchParams object

#### URLSearchParams

```javascript
const params = new URLSearchParams("?foo=bar&baz=qux");

console.log(params.get("foo"));  // "bar"

// Modify
params.append("key", "value");
params.delete("foo");
params.set("new", "param");

// Iterate
for (const [key, value] of params) {
  console.log(`${key}=${value}`);
}
```

---

## WebCrypto API

WebCrypto implementation via Rust crypto crates (`ring`, `aes_gcm`).

### crypto.getRandomValues()

Cryptographically secure random number generation.

```javascript
const array = new Uint8Array(16);
crypto.getRandomValues(array);
// array now contains cryptographically secure random bytes
```

**Parameters:**
- `typedArray` — Uint8Array, Uint16Array, Uint32Array, or Int8Array variants

**Returns:**
- Same typedArray (modified in place)

---

### crypto.subtle.digest()

Hashing algorithms.

```javascript
const encoder = new TextEncoder();
const data = encoder.encode("Hello");

const hashBuffer = await crypto.subtle.digest("SHA-256", data);

// Convert to hex string
const hashArray = Array.from(new Uint8Array(hashBuffer));
const hashHex = hashArray.map(b => b.toString(16).padStart(2, "0")).join("");
// "185f8db32271fe25f561a6fc938b2e264306ec304eda518007d1764826381969"
```

**Supported Algorithms:**
- `"SHA-256"`
- `"SHA-512"`

**Returns:** Promise<ArrayBuffer>

---

### crypto.subtle.generateKey()

Generate cryptographic keys.

```javascript
// AES-GCM key for encryption
const aesKey = await crypto.subtle.generateKey(
  { name: "AES-GCM", length: 256 },
  true, // extractable
  ["encrypt", "decrypt"]
);

// HMAC key for signing
const hmacKey = await crypto.subtle.generateKey(
  { name: "HMAC", hash: "SHA-256" },
  true,
  ["sign", "verify"]
);
```

**Algorithm Parameters:**

| Algorithm | Parameters |
|-----------|------------|
| AES-GCM | `{ name: "AES-GCM", length: 128 \| 256 }` |
| HMAC | `{ name: "HMAC", hash: "SHA-256" \| "SHA-512" }` |

**Returns:** Promise<CryptoKey>

---

### crypto.subtle.importKey()

Import key from external format.

```javascript
// Import JWK
const key = await crypto.subtle.importKey(
  "jwk",
  { 
    kty: "oct", 
    k: "base64url-encoded-key", 
    alg: "A256GCM" 
  },
  { name: "AES-GCM" },
  true,
  ["encrypt", "decrypt"]
);
```

**Formats:**
- `"jwk"` — JSON Web Key
- `"raw"` — Raw bytes (for AES keys)

---

### crypto.subtle.exportKey()

Export key to external format.

```javascript
// Export as JWK
const jwk = await crypto.subtle.exportKey("jwk", key);

// Export as raw bytes
const raw = await crypto.subtle.exportKey("raw", key);
```

---

### crypto.subtle.encrypt() / decrypt()

AES-GCM encryption/decryption.

```javascript
// Generate or import key
const key = await crypto.subtle.generateKey(
  { name: "AES-GCM", length: 256 },
  true,
  ["encrypt", "decrypt"]
);

// Encrypt
const iv = crypto.getRandomValues(new Uint8Array(12));
const data = new TextEncoder().encode("Secret message");

const encrypted = await crypto.subtle.encrypt(
  { name: "AES-GCM", iv },
  key,
  data
);

// Decrypt
const decrypted = await crypto.subtle.decrypt(
  { name: "AES-GCM", iv },
  key,
  encrypted
);

const plaintext = new TextDecoder().decode(decrypted);
```

**Important:**
- IV must be 12 bytes for AES-GCM
- Never reuse IV with same key
- Store IV alongside ciphertext

---

### crypto.subtle.sign() / verify()

HMAC signing/verification.

```javascript
// Generate HMAC key
const key = await crypto.subtle.generateKey(
  { name: "HMAC", hash: "SHA-256" },
  false, // not extractable
  ["sign", "verify"]
);

// Sign data
const data = new TextEncoder().encode("message");
const signature = await crypto.subtle.sign("HMAC", key, data);

// Verify
const isValid = await crypto.subtle.verify(
  "HMAC",
  key,
  signature,
  data
);
```

---

### WebCrypto Notes

**Implemented:**
- `deriveBits` with PBKDF2 (HMAC-SHA-256/384/512). `length` must be a positive
  multiple of 8, at most 8192 bits.

**Not implemented:**
- RSA operations (RSA-OAEP, RSASSA-PKCS1-v1_5)
- ECDSA (P-256, P-384, P-521)
- `deriveKey`, and `deriveBits` for HKDF / ECDH


---

## NANO-Specific APIs

### Nano.fs

Virtual filesystem API for per-isolate file storage.

**All methods are async and return Promises.**

#### Nano.fs.readFile()

Read file contents.

```javascript
// Read as Uint8Array (default)
const bytes = await Nano.fs.readFile("/data/config.json");

// Read as string
const text = await Nano.fs.readFile("/data/config.json", { encoding: "utf-8" });

// Parse JSON
const json = JSON.parse(
  await Nano.fs.readFile("/data/config.json", { encoding: "utf-8" })
);
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `path` | string | File path (relative to VFS root) |
| `options` | object | Optional options |
| `options.encoding` | string | `"utf-8"` or undefined (returns Uint8Array) |

**Returns:** Promise<Uint8Array \| string>

**Errors:**
- `ENOENT` — File not found
- `EACCES` — Permission denied
- `EISDIR` — Path is a directory

---

#### Nano.fs.writeFile()

Write file contents.

```javascript
// Write string
await Nano.fs.writeFile("/data/log.txt", "Log entry");

// Write Uint8Array
const bytes = new TextEncoder().encode("Binary data");
await Nano.fs.writeFile("/data/file.bin", bytes);
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `path` | string | File path |
| `data` | string \| Uint8Array | File contents |
| `options` | object | Optional options |

**Returns:** Promise<void>

**Errors:**
- `EACCES` — Permission denied
- `ENOSPC` — Disk/VFS full
- `EINVAL` — Invalid path

---

#### Nano.fs.exists()

Check if file exists.

```javascript
if (await Nano.fs.exists("/data/config.json")) {
  const config = await Nano.fs.readFile("/data/config.json", { encoding: "utf-8" });
}
```

**Parameters:**
- `path` — string

**Returns:** Promise<boolean>

---

#### Nano.fs.deleteFile()

Delete a file.

```javascript
await Nano.fs.deleteFile("/data/temp.txt");
```

**Parameters:**
- `path` — string

**Returns:** Promise<void>

**Errors:**
- `ENOENT` — File not found
- `EACCES` — Permission denied

---

#### Nano.fs.listDir()

List directory contents.

```javascript
const entries = await Nano.fs.listDir("/data");
for (const entry of entries) {
  console.log(entry.name);  // File/directory name
  console.log(entry.type);  // "file" or "directory"
}
```

**Status:** Basic implementation (v2.0 will enhance)

---

### Timers

#### setTimeout / clearTimeout

```javascript
const timeoutId = setTimeout(() => {
  console.log("Delayed execution");
}, 1000);

clearTimeout(timeoutId);
```

**Note:** Timer duration counts toward request CPU time limits.

---

#### setInterval / clearInterval

```javascript
const intervalId = setInterval(() => {
  console.log("Periodic execution");
}, 5000);

clearInterval(intervalId);
```

---

## Node.js Compatibility Polyfills

### Buffer

Partial Buffer polyfill for Node.js compatibility.

```javascript
// From string
const buf1 = Buffer.from("Hello");

// From array
const buf2 = Buffer.from([0x48, 0x65, 0x6c, 0x6c, 0x6f]);

// From hex/base64
const buf3 = Buffer.from("48656c6c6f", "hex");
const buf4 = Buffer.from("SGVsbG8=", "base64");

// Allocate
const buf5 = Buffer.alloc(16, 0);

// To string
const str = buf1.toString("utf-8"); // "Hello"
```

**Supported Encodings:**
- `"utf-8"` or `"utf8"`
- `"hex"`
- `"base64"`

**Note:** For new code, prefer `Uint8Array` and `TextEncoder`/`TextDecoder` (standard Web APIs).

---

### require('fs')

Node.js-style fs module polyfill using VFS.

```javascript
const fs = require('fs');

// Async with callbacks
fs.readFile('/data/config.json', 'utf8', (err, data) => {
  if (err) throw err;
  console.log(data);
});

// Sync methods (limited support)
const data = fs.readFileSync('/data/config.json', 'utf8');
fs.writeFileSync('/data/output.txt', 'Hello');
const exists = fs.existsSync('/data/config.json');
```

**Supported Methods:**
- `fs.readFile(path, [encoding], callback)`
- `fs.readFileSync(path, [encoding])`
- `fs.writeFile(path, data, [encoding], callback)`
- `fs.writeFileSync(path, data, [encoding])`
- `fs.exists(path, callback)`
- `fs.existsSync(path)`

**Note:** Sync methods have performance implications. Prefer async.

---

## Streams (WinterTC)

### ReadableStream

```javascript
const stream = new ReadableStream({
  start(controller) {
    controller.enqueue(new TextEncoder().encode("Hello"));
    controller.close();
  }
});

// Consume
const reader = stream.getReader();
const { value, done } = await reader.read();
```

---

### WritableStream

```javascript
const stream = new WritableStream({
  write(chunk) {
    console.log("Received:", chunk);
  }
});

const writer = stream.getWriter();
await writer.write(new TextEncoder().encode("Hello"));
await writer.close();
```

---

## Error Handling

APIs throw or reject with standard error types:

```javascript
// WebCrypto errors
try {
  await crypto.subtle.decrypt({ name: "AES-GCM", iv: badIv }, key, data);
} catch (err) {
  console.log(err.name); // "OperationError"
  console.log(err.message); // "Invalid IV length"
}

// File system errors
try {
  await Nano.fs.readFile("/nonexistent");
} catch (err) {
  console.log(err.code); // "ENOENT"
  console.log(err.message); // "No such file or directory"
}

// Fetch errors
try {
  await fetch("https://invalid-url");
} catch (err) {
  console.log(err.name); // "TypeError"
}
```

---

## Examples

### HTTP Handler (Standard Pattern)

```javascript
export default {
  async fetch(request) {
    const url = new URL(request.url);
    
    // Read request body
    const body = await request.text();
    
    // Return response
    return new Response(`Hello from ${url.pathname}`, {
      headers: { "Content-Type": "text/plain" }
    });
  }
};
```

### File Storage App

```javascript
export default {
  async fetch(request) {
    const url = new URL(request.url);
    
    if (request.method === "POST") {
      const body = await request.text();
      await Nano.fs.writeFile(`/data${url.pathname}`, body);
      return new Response("Saved", { status: 201 });
    }
    
    if (await Nano.fs.exists(`/data${url.pathname}`)) {
      const content = await Nano.fs.readFile(`/data${url.pathname}`, { encoding: "utf-8" });
      return new Response(content);
    }
    
    return new Response("Not found", { status: 404 });
  }
};
```

### Crypto Operations

```javascript
export default {
  async fetch(request) {
    // Hash request body
    const body = await request.arrayBuffer();
    const hash = await crypto.subtle.digest("SHA-256", body);
    
    // Convert to hex
    const hex = Array.from(new Uint8Array(hash))
      .map(b => b.toString(16).padStart(2, "0"))
      .join("");
    
    return new Response(JSON.stringify({ hash: hex }), {
      headers: { "Content-Type": "application/json" }
    });
  }
};
```

---

## See Also

- [Configuration](CONFIG.md) — Runtime configuration options
- [Node.js Compatibility](NODEJS_COMPAT.md) — Migration from Node.js
- [Compatibility Matrix](COMPATIBILITY.md) — Full API compatibility status
- [WinterTC Spec](https://wintertc.org/) — Standard APIs NANO implements

---

## WebSocket API

> **Status:** Implemented (server-side `WebSocketPair`). The browser-style
> outbound `new WebSocket(url)` client is a WinterCG compatibility stub only.

WebSocket support follows the [Cloudflare Workers WebSocket API](https://developers.cloudflare.com/workers/runtime-apis/websockets/).

See [WebSocket Guide](WEBSOCKET.md) for full documentation including upgrade flow, limits, and architecture.

### WebSocketPair

```javascript
const [client, server] = new WebSocketPair();
```

Returns two linked `WebSocket` objects. Pass `client` in the `Response`; use `server` to send/receive frames.

### Example

```javascript
export default {
  async fetch(request) {
    if (request.headers.get('Upgrade') !== 'websocket') {
      return new Response('Expected WebSocket', { status: 426 });
    }
    const [client, server] = new WebSocketPair();
    server.addEventListener('message', (event) => {
      server.send(`Echo: ${event.data}`);
    });
    server.accept();
    return new Response(null, { status: 101, webSocket: client });
  }
};
```

### WebSocket Methods

| Method | Description |
|--------|-------------|
| `send(data)` | Send text (string) or binary (ArrayBuffer) frame |
| `close(code?, reason?)` | Send Close frame |
| `accept()` | Accept the connection (required before sending) |
| `addEventListener(type, fn)` | Register `message`, `close`, or `error` handler |

### WebSocket Properties

| Property | Values |
|----------|--------|
| `readyState` | 0=CONNECTING, 1=OPEN, 2=CLOSING, 3=CLOSED |

---

*Last updated: 2026-05-17*
