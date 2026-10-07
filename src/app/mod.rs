//! Application management module
//!
//! Provides application registry, hot-reload, and graceful drain functionality
//! for multi-app hosting scenarios.

pub mod drain;
pub mod registry;
pub mod reload;

pub use drain::{DrainHandle, RequestDrain};
pub use registry::AppRegistry;
pub use reload::{reload_config, ConfigDiff, ReloadError};
