//! WebSocket V8 bindings — Cloudflare Workers WebSocketPair API
//!
//! Implements the CF Workers WebSocket API surface:
//! - `new WebSocketPair()` → object with keys "0" (client) and "1" (server)
//! - `socket.accept()` — transitions server socket to OPEN state (D-14b gate)
//! - `socket.send(data)` — pushes message to outbound channel (TypeError if not accepted)
//! - `socket.close(code, reason)` — sends Close frame, transitions to CLOSING
//! - `socket.addEventListener(type, fn)` — stores callbacks in thread-local handlers
//! - `socket.readyState` — 0=CONNECTING, 1=OPEN, 2=CLOSING, 3=CLOSED
//! - `socket.binaryType` — "arraybuffer" (read-only; setter throws TypeError per D-18b)
//!
//! All V8 callbacks interact with the thread-local state defined in
//! `src/worker/tenant_pool.rs` (WS_OUTBOUND, WS_ACCEPTED, WS_MESSAGE_HANDLERS,
//! WS_CLOSE_HANDLERS, WS_ERROR_HANDLERS, WS_SERVER_SOCKET). Both sides run on
//! the same worker thread, so thread-local access is safe and lock-free.

use crate::worker::tenant_pool::{
    set_ws_readystate, WS_ACCEPTED, WS_CLOSE_HANDLERS, WS_ERROR_HANDLERS, WS_MESSAGE_HANDLERS,
    WS_OUTBOUND, WS_SERVER_SOCKET,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Bind `WebSocketPair` and `WebSocket` constructors to the V8 global scope.
///
/// After this call, JS code can do:
/// ```js
/// const pair = new WebSocketPair();
/// const [client, server] = Object.values(pair);
/// typeof WebSocket !== 'undefined'; // true — WinterCG compat stub
/// ```
///
/// `WebSocket` is a no-op stub (readyState=3, no outbound connection).
/// Follows the exact same pattern as `bind_streams` in `stream.rs`.
pub fn bind_websocket_pair(
    scope: &mut v8::PinnedRef<v8::HandleScope<()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    let wsp_template = v8::FunctionTemplate::new(&mut ctx_scope, websocket_pair_constructor);
    let wsp_ctor = wsp_template.get_function(&mut ctx_scope).unwrap();

    let key = v8::String::new(&mut ctx_scope, "WebSocketPair").unwrap();
    global.set(&mut ctx_scope, key.into(), wsp_ctor.into());

    // WinterCG browser-compatible WebSocket client constructor.
    // Exposes `typeof WebSocket !== 'undefined'`; actual connection is not
    // supported in the server-side execution model — use WebSocketPair instead.
    let ws_template = v8::FunctionTemplate::new(&mut ctx_scope, websocket_client_constructor);
    let ws_ctor = ws_template.get_function(&mut ctx_scope).unwrap();
    let ws_key = v8::String::new(&mut ctx_scope, "WebSocket").unwrap();
    global.set(&mut ctx_scope, ws_key.into(), ws_ctor.into());

    tracing::debug!("Bound WebSocketPair and WebSocket APIs");
}

// ---------------------------------------------------------------------------
// WebSocketPair constructor helpers
// ---------------------------------------------------------------------------

/// Attach all WebSocket methods and properties to an existing V8 Object in place.
///
/// Takes `obj` by value (v8::Local is Copy) so no Local is returned — returning
/// a Local from a helper ties the scope lifetime and causes multiple-borrow
/// errors when the constructor needs to create two sockets sequentially.
fn ws_attach_to_object(scope: &mut v8::PinnedRef<v8::HandleScope>, obj: v8::Local<v8::Object>) {
    if let Some(f) = v8::Function::new(scope, ws_accept_callback) {
        let key = v8::String::new(scope, "accept").unwrap();
        obj.set(scope, key.into(), f.into());
    }
    if let Some(f) = v8::Function::new(scope, ws_send_callback) {
        let key = v8::String::new(scope, "send").unwrap();
        obj.set(scope, key.into(), f.into());
    }
    if let Some(f) = v8::Function::new(scope, ws_close_callback) {
        let key = v8::String::new(scope, "close").unwrap();
        obj.set(scope, key.into(), f.into());
    }
    if let Some(f) = v8::Function::new(scope, ws_add_event_listener_callback) {
        let key = v8::String::new(scope, "addEventListener").unwrap();
        obj.set(scope, key.into(), f.into());
    }
    // readyState: 0 (CONNECTING)
    let rs_key = v8::String::new(scope, "readyState").unwrap();
    let rs_val = v8::Integer::new_from_unsigned(scope, 0);
    obj.set(scope, rs_key.into(), rs_val.into());
    // binaryType: accessor — getter returns "arraybuffer", setter throws TypeError (D-18b).
    let bt_key = v8::String::new(scope, "binaryType").unwrap();
    obj.set_accessor_with_setter(
        scope,
        bt_key.into(),
        ws_binary_type_getter_callback,
        ws_binary_type_setter_callback,
    );
}

// ---------------------------------------------------------------------------
// WebSocketPair constructor
// ---------------------------------------------------------------------------

/// V8 callback for `new WebSocketPair()`.
///
/// Creates a client socket and a server socket, each with all WebSocket methods.
/// The server socket is stored in `WS_SERVER_SOCKET` so the worker loop can
/// update `readyState` on state transitions.
/// The pair object has string keys "0" (client) and "1" (server) to match CF
/// Workers — `Object.values()` returns them in insertion order per ECMAScript spec (D-04).
fn websocket_pair_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Build client socket — attach methods/props in place, no Local returned.
    let client_socket = v8::Object::new(scope);
    ws_attach_to_object(scope, client_socket);

    let server_socket = v8::Object::new(scope);
    ws_attach_to_object(scope, server_socket);

    // Store server socket as Global for worker-loop readyState updates.
    let server_global = v8::Global::new(scope, server_socket);
    WS_SERVER_SOCKET.with(|cell| {
        *cell.borrow_mut() = Some(server_global);
    });

    // Recover a Local for server_socket from the just-stored Global.
    let server_local = {
        let g = WS_SERVER_SOCKET.with(|cell| cell.borrow().as_ref().unwrap().clone());
        v8::Local::new(scope, g)
    };

    // Build pair object with string keys for insertion-order preservation.
    let pair = v8::Object::new(scope);

    let key0 = v8::String::new(scope, "0").unwrap();
    pair.set(scope, key0.into(), client_socket.into());

    let key1 = v8::String::new(scope, "1").unwrap();
    pair.set(scope, key1.into(), server_local.into());

    retval.set(pair.into());
}

// ---------------------------------------------------------------------------
// WebSocket client constructor (browser-compat stub)
// ---------------------------------------------------------------------------

/// WinterCG compat stub — satisfies `typeof WebSocket !== 'undefined'`.
/// Outbound connections not supported in server-side execution model.
fn websocket_client_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let obj = v8::Object::new(scope);

    let url_val: v8::Local<v8::Value> = if args.length() > 0 {
        args.get(0)
    } else {
        v8::String::new(scope, "").unwrap().into()
    };
    let url_key = v8::String::new(scope, "url").unwrap();
    obj.set(scope, url_key.into(), url_val);

    let rs_key = v8::String::new(scope, "readyState").unwrap();
    let rs_val = v8::Integer::new_from_unsigned(scope, 3);
    obj.set(scope, rs_key.into(), rs_val.into());

    let bt_key = v8::String::new(scope, "binaryType").unwrap();
    let bt_val = v8::String::new(scope, "arraybuffer").unwrap();
    obj.set(scope, bt_key.into(), bt_val.into());

    // send() must throw on a CLOSED socket (readyState=3) per WebSocket spec.
    if let Some(f) = v8::Function::new(scope, ws_client_send_closed) {
        let key = v8::String::new(scope, "send").unwrap();
        obj.set(scope, key.into(), f.into());
    }
    // close/addEventListener/removeEventListener are spec-correct no-ops for readyState=3.
    for method in &["close", "addEventListener", "removeEventListener"] {
        if let Some(f) = v8::Function::new(scope, ws_client_noop) {
            let key = v8::String::new(scope, method).unwrap();
            obj.set(scope, key.into(), f.into());
        }
    }

    retval.set(obj.into());
}

fn ws_client_send_closed(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let msg = v8::String::new(scope, "WebSocket is already in CLOSED state").unwrap();
    let err = v8::Exception::error(scope, msg);
    scope.throw_exception(err);
}

fn ws_client_noop(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
}

// ---------------------------------------------------------------------------
// FunctionCallbacks
// ---------------------------------------------------------------------------

/// `ws.accept()` — transitions socket from CONNECTING to OPEN (D-14b gate).
fn ws_accept_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    WS_ACCEPTED.with(|cell| cell.set(true));
    set_ws_readystate(scope, 1); // OPEN
}

/// `ws.send(data)` — push a message to the outbound channel.
///
/// Throws `TypeError("WebSocket is not accepted")` if `ws.accept()` has not been
/// called yet (D-14b). Silently ignores send failures if the channel is closed.
fn ws_send_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // D-14b: enforce accept() guard.
    let accepted = WS_ACCEPTED.with(|cell| cell.get());
    if !accepted {
        if let Some(msg) = v8::String::new(scope, "WebSocket is not accepted") {
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
        }
        return;
    }

    if args.length() < 1 {
        return;
    }

    let arg = args.get(0);

    // Build tungstenite message from JS argument (T-23-12: only String/ArrayBuffer accepted).
    let message: Option<tungstenite::Message> = if arg.is_string() {
        if let Some(s) = arg.to_string(scope) {
            let text = s.to_rust_string_lossy(scope);
            Some(tungstenite::Message::Text(text))
        } else {
            None
        }
    } else if arg.is_array_buffer() {
        match arg.try_cast::<v8::ArrayBuffer>() {
            Ok(ab) => {
                let store = ab.get_backing_store();
                let length = ab.byte_length();
                let bytes: Vec<u8> = (0..length)
                    .filter_map(|i| store.get(i).map(|cell| cell.get()))
                    .collect();
                Some(tungstenite::Message::Binary(bytes))
            }
            Err(_) => None,
        }
    } else if arg.is_array_buffer_view() {
        // TypedArray / DataView — extract slice from the underlying ArrayBuffer.
        arg.to_object(scope)
            .and_then(|o| o.try_cast::<v8::ArrayBufferView>().ok())
            .and_then(|view| {
                let ab = view.buffer(scope)?;
                let byte_offset = view.byte_offset();
                let byte_length = view.byte_length();
                let store = ab.get_backing_store();
                let bytes: Vec<u8> = (byte_offset..byte_offset + byte_length)
                    .filter_map(|i| store.get(i).map(|cell| cell.get()))
                    .collect();
                Some(tungstenite::Message::Binary(bytes))
            })
    } else {
        // Other types silently ignored per threat model T-23-12.
        None
    };

    if let Some(msg) = message {
        WS_OUTBOUND.with(|cell| {
            let borrow = cell.borrow();
            if let Some(ref sender) = *borrow {
                // Silently ignore send errors (channel closed = connection closing).
                let _ = sender.try_send(msg);
            }
        });
    }
}

/// `ws.close([code[, reason]])` — send a Close frame and transition to CLOSING.
fn ws_close_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if !WS_ACCEPTED.with(|cell| cell.get()) {
        return;
    }

    let code: u16 = if args.length() > 0 {
        args.get(0)
            .to_number(scope)
            .map(|n| n.value() as u16)
            .unwrap_or(1000)
    } else {
        1000
    };

    let reason: String = if args.length() > 1 {
        args.get(1)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default()
    } else {
        String::new()
    };

    set_ws_readystate(scope, 2);

    let close_frame = tungstenite::protocol::CloseFrame {
        code: tungstenite::protocol::frame::coding::CloseCode::from(code),
        reason: std::borrow::Cow::Owned(reason),
    };
    let msg = tungstenite::Message::Close(Some(close_frame));

    WS_OUTBOUND.with(|cell| {
        let borrow = cell.borrow();
        if let Some(ref sender) = *borrow {
            let _ = sender.try_send(msg);
        }
    });
}

/// `ws.addEventListener(type, fn)` — register an event handler.
///
/// Recognized event types: "message", "close", "error".
/// Non-function handlers are silently ignored (threat T-23-13).
/// Unknown event types are silently ignored.
fn ws_add_event_listener_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 {
        return;
    }

    let event_type = match args.get(0).to_string(scope) {
        Some(s) => s.to_rust_string_lossy(scope),
        None => return,
    };

    // Verify handler is a function (T-23-13: non-functions silently ignored).
    let handler_arg = args.get(1);
    if !handler_arg.is_function() {
        return;
    }
    // safe: is_function() verified above
    let handler_global = v8::Global::new(scope, handler_arg.cast::<v8::Function>());

    match event_type.as_str() {
        "message" => {
            WS_MESSAGE_HANDLERS.with(|cell| cell.borrow_mut().push(handler_global));
        }
        "close" => {
            WS_CLOSE_HANDLERS.with(|cell| cell.borrow_mut().push(handler_global));
        }
        "error" => {
            WS_ERROR_HANDLERS.with(|cell| cell.borrow_mut().push(handler_global));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// binaryType accessor callbacks (D-18b)
// ---------------------------------------------------------------------------

/// Getter for `socket.binaryType` — always returns "arraybuffer".
fn ws_binary_type_getter_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _name: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut retval: v8::ReturnValue<v8::Value>,
) {
    if let Some(s) = v8::String::new(scope, "arraybuffer") {
        retval.set(s.into());
    }
}

/// Setter for `socket.binaryType` — always throws TypeError per D-18b.
///
/// CF Workers only supports "arraybuffer"; attempts to change it are rejected.
fn ws_binary_type_setter_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _name: v8::Local<v8::Name>,
    _value: v8::Local<v8::Value>,
    _args: v8::PropertyCallbackArguments,
    _retval: v8::ReturnValue<()>,
) {
    if let Some(msg) = v8::String::new(scope, "binaryType is read-only: only arraybuffer supported")
    {
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::{initialize_platform, NanoIsolate};

    fn init_platform() {
        initialize_platform().expect("Failed to initialize V8 platform");
    }

    #[test]
    fn test_websocket_stub_ready_state_and_url() {
        init_platform();
        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
        bind_websocket_pair(ctx_scope, context);

        let code = r#"
            const ws = new WebSocket('ws://test.example');
            ws.readyState === 3 && ws.url === 'ws://test.example' && typeof ws.send === 'function'
        "#;
        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_string, None).expect("compile");
        let result = script.run(ctx_scope).expect("run");
        let result_str = result
            .to_string(ctx_scope)
            .unwrap()
            .to_rust_string_lossy(ctx_scope);
        assert_eq!(
            result_str, "true",
            "WebSocket stub: readyState=3, url set from arg, send callable"
        );
    }

    #[test]
    fn test_websocket_stub_send_throws_on_closed() {
        init_platform();
        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
        bind_websocket_pair(ctx_scope, context);

        let code = r#"
            let threw = false;
            try {
                const ws = new WebSocket('ws://x');
                ws.send('hello');
            } catch (e) {
                threw = e.message.includes('CLOSED');
            }
            threw
        "#;
        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_string, None).expect("compile");
        let result = script.run(ctx_scope).expect("run");
        let result_str = result
            .to_string(ctx_scope)
            .unwrap()
            .to_rust_string_lossy(ctx_scope);
        assert_eq!(
            result_str, "true",
            "send() on CLOSED WebSocket stub must throw"
        );
    }

    #[test]
    fn test_websocket_stub_noop_methods_do_not_throw() {
        init_platform();
        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
        bind_websocket_pair(ctx_scope, context);

        let code = r#"
            let threw = false;
            try {
                const ws = new WebSocket('ws://x');
                ws.close();
                ws.addEventListener('message', () => {});
                ws.removeEventListener('message', () => {});
            } catch (e) {
                threw = true;
            }
            !threw
        "#;
        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_string, None).expect("compile");
        let result = script.run(ctx_scope).expect("run");
        let result_str = result
            .to_string(ctx_scope)
            .unwrap()
            .to_rust_string_lossy(ctx_scope);
        assert_eq!(
            result_str, "true",
            "close/addEventListener/removeEventListener on CLOSED stub must not throw"
        );
    }
}
