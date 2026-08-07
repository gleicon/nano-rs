//! NANO Edge Runtime - Multi-tenant JavaScript edge runtime
//!
//! A Rust-based edge runtime using rusty_v8 to execute JavaScript in isolated
//! V8 contexts. Supports multi-tenancy with thread-local isolates and context
//! reset between requests for fast cold starts.

// The V8 crate's PinnedRef API accepts &HandleScope (not &mut) for most
// operations, but historical call sites pass &mut scope. Suppress crate-wide
// to avoid 300+ mechanical changes that are correct but noisy.
#![allow(clippy::unnecessary_mut_passed)]
// Pre-existing lint debt across 40+ files; tracked here rather than scattered
// as per-site annotations. These are style/idiom warnings, not bugs.
#![allow(
    clippy::borrow_deref_ref,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::collapsible_str_replace,
    clippy::derivable_impls,
    clippy::doc_overindented_list_items,
    clippy::double_ended_iterator_last,
    clippy::empty_line_after_doc_comments,
    clippy::explicit_auto_deref,
    clippy::implicit_saturating_sub,
    clippy::inherent_to_string,
    clippy::large_enum_variant,
    clippy::let_and_return,
    clippy::let_unit_value,
    clippy::manual_div_ceil,
    clippy::manual_is_multiple_of,
    clippy::manual_map,
    clippy::manual_range_contains,
    clippy::map_identity,
    clippy::missing_const_for_thread_local,
    clippy::missing_transmute_annotations,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_range_loop,
    clippy::needless_return,
    clippy::new_without_default,
    clippy::op_ref,
    clippy::question_mark,
    clippy::redundant_closure,
    clippy::should_implement_trait,
    clippy::single_match,
    clippy::to_string_in_format_args,
    clippy::type_complexity,
    clippy::unnecessary_cast,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_map_or,
    clippy::unnecessary_sort_by,
    clippy::unnecessary_unwrap,
    clippy::unwrap_or_default,
    clippy::useless_conversion
)]

use anyhow::Result;

pub mod admin;
pub mod app;
pub mod assertions;
pub mod config;
pub mod control_plane;
pub mod data_plane;
pub mod http;
pub mod limits;
pub mod logging;
pub mod metrics;
pub mod runtime;
pub mod signal;
pub mod sliver;
pub mod v8;
pub mod vfs;
pub mod wasm;
pub mod worker;

/// Library entry point — stub used by tests and binary glue.
///
/// ## Integration boundary
///
/// External systems (PaaS layers, orchestrators) integrate with nano-rs via the
/// **admin HTTP API** (`POST /admin/apps`, `reload`, `scale`, etc.) over the Unix
/// socket or TCP address configured at startup. This is the only supported
/// integration surface.
///
/// The `[lib]` crate target exists for internal test infrastructure. The module
/// tree (`worker`, `runtime`, `wasm`, `vfs`, etc.) is `pub` for test access, but
/// its API is **not stable** and may change between versions without notice.
///
/// ## Future embedding (not yet designed)
///
/// A `NanoRuntime::builder()` embedding API is a potential future path, but
/// requires a design doc covering public API surface, lifecycle contract
/// (start/reload/shutdown), and config model. Do not implement without that design.
pub fn run() -> Result<()> {
    tracing::info!("NANO runtime initialized");
    Ok(())
}
