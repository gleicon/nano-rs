# WebSocket Support

**Status:** Implemented (v2.1.x)  

---

## Overview

NANO-RS supports WebSocket connections via HTTP Upgrade. Each WebSocket connection runs on a dedicated worker thread sharing the V8 isolate lifecycle with that connection — when the connection closes, the isolate is recycled.

JavaScript handlers use `addEventListener` (Cloudflare Workers pattern) to register message/close/error callbacks before the first frame arrives.

---

## Architecture

### Upgrade Flow

```
HTTP GET /path
  Upgrade: websocket
  Connection: Upgrade
        │
        ▼
router.rs — detect_ws_upgrade()
        │  checks headers; 101 handshake via axum ws extractor
        ▼
relay task (tokio) — WsChannels
        │  mpsc: client→worker (WsInbound), worker→client (WsOutbound)
        ▼
TenantPool::dispatch_ws()
        │  lazy-spawn WS worker thread if pool not at capacity
        ▼
Worker thread — ws_messages loop
        │  JS fetch handler called first (registers addEventListener callbacks)
        │  'ws_messages: loop { recv_timeout(idle_timeout_ms) }
        ▼
V8 isolate — MessageEvent / CloseEvent dispatch
```

### Key Components

| Component | File | Role |
|-----------|------|------|
| `WsChannels` | `src/worker/tenant_pool.rs` | mpsc channel pair for frame relay |
| `detect_ws_upgrade` | `src/http/router.rs` | Header check + axum handshake |
| `relay_task` | `src/http/router.rs` | tokio task bridging axum↔WsChannels |
| `dispatch_ws` | `src/worker/tenant_pool.rs` | Route WS request to worker thread |
| `ws_messages loop` | `src/worker/pool.rs` | Per-frame JS dispatch, lifecycle |
| `WebSocketPair` | `src/runtime/apis.rs` | V8 binding (Plan 05) |

### Thread-Locals

Set before the `'ws_messages` loop, cleared after:

| Thread-Local | Type | Purpose |
|-------------|------|---------|
| `WS_OUTBOUND` | `Sender<WsFrame>` | Send frames to client |
| `WS_ACCEPTED` | `bool` | Whether WS is active |
| `WS_MESSAGE_HANDLERS` | `Vec<Global<Function>>` | `onmessage` callbacks |
| `WS_CLOSE_HANDLERS` | `Vec<Global<Function>>` | `onclose` callbacks |
| `WS_ERROR_HANDLERS` | `Vec<Global<Function>>` | `onerror` callbacks |
| `WS_SERVER_SOCKET` | `Global<Object>` | JS WebSocket object |

### Isolation Model

- One V8 isolate per WebSocket connection (worker thread dedicated to connection)
- Isolate recycled after connection closes (`break 'requests` forces fresh isolate)
- `ws_busy` counter incremented by worker thread (not dispatch) — avoids TOCTOU
- Per-message `CpuTimeoutGuard` prevents runaway JS in message handlers
- OOM check per message; sends Close 1011 if heap limit exceeded

---

## JavaScript API

WebSocket handlers follow the Cloudflare Workers `addEventListener` pattern:

```javascript
export default {
  async fetch(request, env) {
    // Check for WebSocket upgrade
    if (request.headers.get('Upgrade') !== 'websocket') {
      return new Response('Expected WebSocket', { status: 426 });
    }

    // Create WebSocketPair
    const [client, server] = new WebSocketPair();

    server.addEventListener('message', (event) => {
      console.log('Received:', event.data);
      server.send(`Echo: ${event.data}`);
    });

    server.addEventListener('close', (event) => {
      console.log('Closed:', event.code, event.reason);
    });

    server.addEventListener('error', (event) => {
      console.error('Error:', event.message);
    });

    server.accept();

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }
};
```

### WebSocketPair

```javascript
const [client, server] = new WebSocketPair();
```

Returns two connected `WebSocket` objects. `client` is returned in the `Response`. `server` is used by the handler to send/receive frames.

### WebSocket Object

| Member | Type | Description |
|--------|------|-------------|
| `send(data)` | method | Send text or binary frame |
| `close(code?, reason?)` | method | Send Close frame |
| `accept()` | method | Accept the connection (required) |
| `addEventListener(type, fn)` | method | Register callback |
| `readyState` | property | 0=CONNECTING, 1=OPEN, 2=CLOSING, 3=CLOSED |

**Event types:** `message`, `close`, `error`

**MessageEvent:**
- `.data` — `string` for text frames, `ArrayBuffer` for binary frames

**CloseEvent:**
- `.code` — WebSocket close code (1000, 1001, etc.)
- `.reason` — Close reason string

---

## Limits

Configured in `AppLimits` / `config.json`:

| Limit | Default | Config Key |
|-------|---------|-----------|
| Max WS connections per app | 100 | `ws_max_connections` |
| Max message size | 32 MiB | `ws_max_message_bytes` |
| Idle timeout | 60 000 ms | `ws_idle_timeout_ms` |

Messages exceeding `ws_max_message_bytes` cause the connection to close with code 1009 (message too large).

Idle timeout fires when no frame is received within `ws_idle_timeout_ms`; connection closes with code 1001 (going away).

---

## Cloudflare Workers Compatibility

NANO-RS WebSocket follows the [Cloudflare Workers WebSocket API](https://developers.cloudflare.com/workers/runtime-apis/websockets/):

- `WebSocketPair` constructor
- `server.accept()` required before sending
- `addEventListener` for event registration
- `Response` with `webSocket` property for upgrade response

**Not supported (out of scope):**
- Hibernatable WebSockets (Durable Objects pattern)
- `cf.webSocket` properties

---

## See Also

- [API Reference](API.md) — Full JavaScript API docs
- [Cloudflare Compatibility](CLOUDFLARE_COMPATIBILITY.md) — Worker compatibility mode
- [Architecture](../ARCHITECTURE.md) — Request lifecycle and threading model
