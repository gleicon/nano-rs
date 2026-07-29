//! Unified Application Source Types

use crate::sliver::UnpackedSliver;

/// Source of application code for worker initialization.
#[derive(Debug, Clone)]
pub enum AppSource {
    /// Load JS/WASM from a filesystem path.
    Entrypoint { path: String },

    /// Load from a sliver (VFS-first, no heap snapshot).
    Sliver { data: UnpackedSliver },

    /// Static file serving — no V8 isolate created.
    Static { root: String },
}

impl AppSource {
    pub fn entrypoint(path: impl Into<String>) -> Self {
        Self::Entrypoint { path: path.into() }
    }

    pub fn sliver(data: UnpackedSliver) -> Self {
        Self::Sliver { data }
    }

    pub fn static_site(root: impl Into<String>) -> Self {
        Self::Static { root: root.into() }
    }

    pub fn needs_isolate(&self) -> bool {
        matches!(self, Self::Entrypoint { .. } | Self::Sliver { .. })
    }

    pub fn is_entrypoint(&self) -> bool { matches!(self, Self::Entrypoint { .. }) }
    pub fn is_sliver(&self) -> bool { matches!(self, Self::Sliver { .. }) }
    pub fn is_static(&self) -> bool { matches!(self, Self::Static { .. }) }

    pub fn entrypoint_path(&self) -> Option<&str> {
        match self { Self::Entrypoint { path } => Some(path), _ => None }
    }

    pub fn sliver_data(&self) -> Option<&UnpackedSliver> {
        match self { Self::Sliver { data } => Some(data), _ => None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entrypoint_creation() {
        let source = AppSource::entrypoint("./app.js");
        assert!(source.is_entrypoint());
        assert!(!source.is_sliver());
        assert_eq!(source.entrypoint_path(), Some("./app.js"));
    }

    #[test]
    fn test_static_creation() {
        let source = AppSource::static_site("./static");
        assert!(source.is_static());
        assert!(!source.needs_isolate());
    }
}
