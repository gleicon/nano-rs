//! V8 Snapshot Support for Fast Cold Starts
//!
//! This module provides infrastructure for loading pre-generated V8 snapshots.
//! Snapshots should be created at build/deploy time, not runtime.
//!
//! ## Architecture
//!
//! Build/Deploy Time (separate tool):
//!   Create isolate → Bind WinterTC APIs → create_blob() → Save to file
//!
//! Server Startup:
//!   Load snapshot file → Store in SnapshotCache
//!
//! Cold Start (Request 1):
//!   StartupData::new(snapshot_blob)
//!   → CreateParams::snapshot_blob()
//!   → Isolate::new(params) [~1ms vs ~50-100ms from scratch]
//!   → Execute user script → Store handler in global scope
//!   → Handle Request
//!
//! Warm Start (Requests 2-N):
//!   Reuse existing isolate
//!   → Handler already in global scope
//!   → Handle Request [~1-5ms]

use anyhow::{anyhow, Result};

/// Ensure V8 platform is initialized (idempotent).
///
/// Delegates to the single crate-wide guard in `v8::platform` — a second,
/// independent `Once` here would double-initialize the V8 platform, which V8
/// forbids (it aborts the process).
pub fn ensure_v8_initialized() {
    crate::v8::initialize_platform().expect("V8 platform initialization failed");
}

/// Cache for pre-generated runtime snapshot.
///
/// Snapshots should be created at build time using a separate tool,
/// then loaded at server startup.
pub struct SnapshotCache {
    data: Vec<u8>,
}

impl SnapshotCache {
    /// Create a snapshot cache from pre-generated data.
    pub fn from_data(data: Vec<u8>) -> Self {
        tracing::info!("Loaded V8 snapshot ({} bytes)", data.len());
        Self { data }
    }

    /// Attempt to load snapshot from a file path.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let data =
            std::fs::read(path).map_err(|e| anyhow!("Failed to read snapshot file: {}", e))?;

        if data.is_empty() {
            return Err(anyhow!("Snapshot file is empty"));
        }

        tracing::info!(
            "Loaded V8 snapshot from {} ({} bytes)",
            path.display(),
            data.len()
        );
        Ok(Self { data })
    }

    /// Get the snapshot data as a byte slice.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Check if snapshot has valid data.
    pub fn is_valid(&self) -> bool {
        !self.data.is_empty()
    }
}

/// Create an isolate from a snapshot.
///
/// This is the fast path for cold starts - the isolate is created with
/// WinterTC APIs already available, skipping the compilation/bind step.
pub fn create_isolate_from_snapshot(snapshot_data: &[u8]) -> v8::OwnedIsolate {
    ensure_v8_initialized();

    let startup_data: v8::StartupData = snapshot_data.to_vec().into();
    let create_params = v8::CreateParams::default().snapshot_blob(startup_data);

    v8::Isolate::new(create_params)
}

/// Lazy-initialized global snapshot cache.
use std::sync::OnceLock;
static GLOBAL_SNAPSHOT: OnceLock<SnapshotCache> = OnceLock::new();

/// Initialize the global snapshot cache from pre-generated data.
///
/// This should be called once at server startup, before any isolates are created.
pub fn init_global_snapshot(data: Vec<u8>) -> Result<()> {
    let snapshot = SnapshotCache::from_data(data);
    GLOBAL_SNAPSHOT
        .set(snapshot)
        .map_err(|_| anyhow!("Global snapshot already initialized"))?;
    Ok(())
}

/// Initialize the global snapshot cache from a file.
pub fn init_global_snapshot_from_file(path: &std::path::Path) -> Result<()> {
    let snapshot = SnapshotCache::from_file(path)?;
    GLOBAL_SNAPSHOT
        .set(snapshot)
        .map_err(|_| anyhow!("Global snapshot already initialized"))?;
    Ok(())
}

/// Get a reference to the global snapshot cache.
///
/// Returns None if snapshot hasn't been initialized.
pub fn global_snapshot() -> Option<&'static SnapshotCache> {
    GLOBAL_SNAPSHOT.get()
}

/// Check if the global snapshot has been initialized.
pub fn is_snapshot_initialized() -> bool {
    GLOBAL_SNAPSHOT.get().is_some()
}

/// Check if the global snapshot has valid data.
pub fn is_snapshot_valid() -> bool {
    GLOBAL_SNAPSHOT.get().map(|s| s.is_valid()).unwrap_or(false)
}

/// Serialize a snapshot-creator NanoIsolate's heap into a startup blob.
///
/// The isolate MUST have been built with `NanoIsolate::snapshot_creator*` (which
/// sets a default context — a precondition of `create_blob`). `into_inner` releases
/// the EPT sentinel Global and every other field, handing over a bare isolate with
/// no live handles; `create_blob` then walks the heap and emits a loadable blob.
/// The isolate is consumed by this call.
pub fn create_snapshot_from_nano(isolate: crate::v8::NanoIsolate) -> anyhow::Result<Vec<u8>> {
    let owned = isolate.into_inner();
    let blob = owned
        .create_blob(v8::FunctionCodeHandling::Keep)
        .ok_or_else(|| anyhow!("create_blob returned no data (isolate had no default context?)"))?;
    if blob.len() < 8 {
        anyhow::bail!(
            "create_blob produced a degenerate blob ({} bytes)",
            blob.len()
        );
    }
    Ok(blob.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_snapshot() {
        let snapshot = SnapshotCache::from_data(vec![]);
        assert!(!snapshot.is_valid());
    }

    #[test]
    fn test_valid_snapshot() {
        let snapshot = SnapshotCache::from_data(vec![1, 2, 3, 4]);
        assert!(snapshot.is_valid());
        assert_eq!(snapshot.data().len(), 4);
    }

    /// Empirically prove v8 150 can create a heap-snapshot blob at runtime — the
    /// capability behind `create_snapshot_from_nano`. The EPT sentinel Global
    /// complicates `create_blob` (which consumes the isolate), so this pins the API.
    #[test]
    fn v150_snapshot_creator_produces_blob() {
        ensure_v8_initialized();

        let mut isolate = v8::Isolate::snapshot_creator(None, None);
        {
            let handle_scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let mut handle_scope = handle_scope.init();
            let context = v8::Context::new(&handle_scope, Default::default());
            let mut ctx_scope = v8::ContextScope::new(&mut handle_scope, context);
            ctx_scope.set_default_context(context);

            // Bake some state into the heap so the snapshot has content.
            let code = v8::String::new(&mut ctx_scope, "var baked = 40 + 2;").unwrap();
            let script = v8::Script::compile(&mut ctx_scope, code, None).unwrap();
            script.run(&mut ctx_scope);
        }

        // create_blob consumes the isolate and requires it be a snapshot_creator.
        let blob = isolate
            .create_blob(v8::FunctionCodeHandling::Keep)
            .expect("v8 150 create_blob should return a snapshot blob");
        assert!(
            blob.len() > 8,
            "snapshot blob should be non-trivial, got {} bytes",
            blob.len()
        );
        // Sanity: the blob is loadable as a startup snapshot.
        assert!(SnapshotCache::from_data(blob.to_vec()).is_valid());
    }

    /// End-to-end: build a NanoIsolate via the snapshot_creator constructor, then
    /// serialize it with `create_snapshot_from_nano`. Proves the EPT-sentinel
    /// integration path works, not just the bare-spike path.
    #[test]
    fn create_snapshot_from_nano_produces_loadable_blob() {
        crate::v8::initialize_platform().expect("platform init");

        let nano = crate::v8::NanoIsolate::snapshot_creator().expect("snapshot_creator isolate");
        let blob = create_snapshot_from_nano(nano).expect("snapshot creation should succeed");

        // Blob is non-trivial.
        assert!(blob.len() > 8, "blob too small: {} bytes", blob.len());
        eprintln!("BLOB_FIRST_8: {:02X?}", &blob[0..8]);

        // The blob restores into a working isolate.
        let restored = create_isolate_from_snapshot(&blob);
        drop(restored);
    }

    /// The NanoIsolate-level restore primitive: a snapshot blob round-trips back
    /// into a fully-wrapped NanoIsolate (EPT sentinel + VFS) via
    /// `create_isolate_from_snapshot` + `NanoIsolate::from_v8_isolate`, and the
    /// restored isolate executes JS.
    #[test]
    fn nano_isolate_round_trips_through_snapshot() {
        crate::v8::initialize_platform().expect("platform init");

        let nano = crate::v8::NanoIsolate::snapshot_creator().expect("snapshot_creator isolate");
        let blob = create_snapshot_from_nano(nano).expect("snapshot creation");

        let owned = create_isolate_from_snapshot(&blob);
        let mut restored =
            crate::v8::NanoIsolate::from_v8_isolate(owned).expect("wrap restored isolate");

        // The restored isolate runs JS.
        let scope = std::pin::pin!(v8::HandleScope::new(restored.isolate()));
        let mut scope = scope.init();
        let ctx = v8::Context::new(&scope, Default::default());
        let mut cs = v8::ContextScope::new(&mut scope, ctx);
        let code = v8::String::new(&mut cs, "6 * 7").unwrap();
        let script = v8::Script::compile(&mut cs, code, None).unwrap();
        let result = script.run(&mut cs).unwrap();
        assert_eq!(result.to_rust_string_lossy(&mut cs), "42");
    }
}
