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

pub mod handler;
pub mod apis;
pub mod console_api;
pub mod text_codec_api;
pub mod url_api;
pub mod buffer_api;
pub mod fetch;
pub mod stream;
pub mod crypto;
pub mod vfs_bindings;
pub mod fs_polyfill;
pub mod request;
pub mod async_support;
pub mod timers;
pub mod websocket;
pub mod subtle_v8;
pub mod web_apis;
pub mod kv;

// Re-export handler types for convenience
pub use handler::{HandlerContext, execute_handler, execute_handler_with_context};

// Re-export APIs for handler
pub use apis::RuntimeAPIs;
pub use fetch::{bind_fetch, FetchState};
