//! WebSocket thread-local state shared between worker threads and V8 callbacks
//!
//! The `pool.rs` worker loop and the `runtime/websocket.rs` V8 bindings both
//! run on the same OS thread, so thread-locals provide lock-free coordination.

use std::cell::{Cell, RefCell};

// ---------------------------------------------------------------------------
// Thread-local WebSocket connection state
// ---------------------------------------------------------------------------

// Outbound frame sender — cloned from WsChannels.outbound_tx on WS entry.
thread_local! {
    pub(crate) static WS_OUTBOUND: RefCell<Option<std::sync::mpsc::SyncSender<tungstenite::Message>>> =
        RefCell::new(None);
}

// Whether the JS handler called ws.accept() — send() checks this (D-14b).
thread_local! {
    pub(crate) static WS_ACCEPTED: Cell<bool> = Cell::new(false);
}

// JS 'message' event handlers registered via addEventListener('message', fn).
thread_local! {
    pub(crate) static WS_MESSAGE_HANDLERS: RefCell<Vec<v8::Global<v8::Function>>> =
        RefCell::new(Vec::new());
}

// JS 'close' event handlers registered via addEventListener('close', fn).
thread_local! {
    pub(crate) static WS_CLOSE_HANDLERS: RefCell<Vec<v8::Global<v8::Function>>> =
        RefCell::new(Vec::new());
}

// JS 'error' event handlers registered via addEventListener('error', fn).
thread_local! {
    pub(crate) static WS_ERROR_HANDLERS: RefCell<Vec<v8::Global<v8::Function>>> =
        RefCell::new(Vec::new());
}

// The server-side WebSocket object (v8::Object) created by WebSocketPair ctor.
// Used to update the readyState property on state transitions (D-16b).
thread_local! {
    pub(crate) static WS_SERVER_SOCKET: RefCell<Option<v8::Global<v8::Object>>> =
        RefCell::new(None);
}

/// Update the readyState property on the server WebSocket object.
/// No-op if WS_SERVER_SOCKET is None (safe before WebSocketPair is constructed).
pub(crate) fn set_ws_readystate(scope: &mut v8::PinScope<'_, '_>, state: u32) {
    WS_SERVER_SOCKET.with(|cell| {
        let borrow = cell.borrow();
        if let Some(ref global) = *borrow {
            let obj = v8::Local::new(scope, global);
            if let Some(key) = v8::String::new(scope, "readyState") {
                let val = v8::Integer::new_from_unsigned(scope, state);
                obj.set(scope, key.into(), val.into());
            }
        }
    });
}

/// Reset all WS thread-locals to their initial (idle) state.
///
/// Called after the ws_messages loop exits to ensure no stale V8 Globals
/// or channel senders survive isolate recycling (D-10b full context reset).
pub(crate) fn clear_ws_thread_locals() {
    WS_OUTBOUND.with(|cell| *cell.borrow_mut() = None);
    WS_ACCEPTED.with(|cell| cell.set(false));
    WS_MESSAGE_HANDLERS.with(|cell| cell.borrow_mut().clear());
    WS_CLOSE_HANDLERS.with(|cell| cell.borrow_mut().clear());
    WS_ERROR_HANDLERS.with(|cell| cell.borrow_mut().clear());
    WS_SERVER_SOCKET.with(|cell| *cell.borrow_mut() = None);
}
