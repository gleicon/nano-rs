//! JavaScript runtime APIs
//!
//! This module implements the WinterTC-compatible JavaScript APIs:
//! - fetch() handler interface
//! - console.log/warn/error
//! - TextEncoder/TextDecoder
//! - setTimeout/setInterval
//! - AbortController/AbortSignal
//! - structuredClone()
//! - crypto.getRandomValues()
//! - performance.now()
//! - Blob and FormData
//! - DOMException
//!
//! These APIs bridge between JavaScript execution in V8 and the Rust
//! runtime, providing the standard WinterTC environment for edge functions.

pub mod apis;
pub mod async_support;
pub mod buffer_api;
pub mod console_api;
pub mod crypto;
pub mod fetch;
pub mod fs_polyfill;
pub mod handler;
pub mod kv;
pub mod request;
pub mod stream;
pub mod subtle_v8;
pub mod text_codec_api;
pub mod timers;
pub mod url_api;
pub mod v8_helpers;
pub mod vfs_bindings;
pub mod web_apis;
pub mod websocket;

// Re-export handler types for convenience
pub use handler::{execute_handler, execute_handler_with_context, HandlerContext};

// Re-export APIs for handler
pub use apis::RuntimeAPIs;
pub use fetch::{bind_fetch, FetchState};
