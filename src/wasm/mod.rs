//! WebAssembly support for NANO runtime.
//!
//! WASM runs inside JS handlers via V8's built-in `WebAssembly` global. This
//! module provides the JS-facing bindings and byte validation.
//!
//! - **JavaScript API** (`js_api.rs`): wraps `WebAssembly.compile`/`instantiate`
//!   with limit checks and a per-isolate compiled-module cache (in JS — the v150
//!   synchronous Rust compile API is unusable in `FunctionCallback`s).
//! - **Loader** (`loader.rs`): WASM byte validation.
//!
//! WASM is available to JS handlers, not as a standalone deployable app type
//! (there is no `AppSource::Wasm`).

pub mod error;
pub mod js_api;
pub mod loader;

pub use error::WasmError;
pub use js_api::WebAssemblyAPI;
pub use loader::WasmLoader;
