//! V8 integration with EPT fix
//!
//! This module handles V8 platform initialization, isolate creation,
//! and the critical EPT (ExternalPointerTable) fix that prevents
//! SIGSEGV crashes during ArrayBuffer allocation.
//!
//! The EPT fix requires creating a strong v8::Global<Value> sentinel
//! per isolate immediately after creation to prevent the background
//! GC from unmapping the array_buffer_sweeper_space segment.
//!
//! Reference: AP-02 from Zig version (prod.md)

pub mod abstractions;
pub mod context;
pub mod isolate;
pub mod module;
pub mod platform;
pub mod script;
pub mod snapshot;

// Re-export key functions and types for convenience
pub use context::create_context;
pub use isolate::NanoIsolate;
pub use module::{is_esm_module, transform_module_code, ModuleLoader, ModuleType};
pub use platform::{initialize_platform, is_initialized, shutdown_platform};
pub use script::execute_script;
pub use snapshot::{
    create_isolate_from_snapshot, create_snapshot_from_nano, ensure_v8_initialized,
    global_snapshot, init_global_snapshot, init_global_snapshot_from_file, is_snapshot_initialized,
    is_snapshot_valid, SnapshotCache,
};
