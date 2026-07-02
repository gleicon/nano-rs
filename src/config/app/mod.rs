//! Application configuration — thin re-export module
//!
//! Types are defined in `types.rs` and validation functions in `validation.rs`.

mod types;
mod validation;

pub use types::*;
pub use validation::{validate_config, validate_nano_config};
