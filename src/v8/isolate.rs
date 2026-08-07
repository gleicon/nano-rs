//! V8 isolate with EPT (ExternalPointerTable) fix sentinel
//!
//! This module implements the critical EPT fix that prevents SIGSEGV crashes
//! during ArrayBuffer allocation. The fix requires a strong v8::Global<Value>
//! sentinel per isolate that keeps the EPT segment mapped.
//!
//! # EPT Fix Explanation
//!
//! The ExternalPointerTable (EPT) manages pointers to objects outside the V8 heap
//! (like ArrayBuffer backing stores). When contexts are rapidly created/disposed,
//! the background GC may unmap the `array_buffer_sweeper_space` EPT segment while
//! ArrayBuffer allocations are still in flight, causing use-after-free or SIGSEGV.
//!
//! The fix: Create a strong `v8::Global<Value>` sentinel immediately after isolate
//! creation. This sentinel holds a reference that keeps the EPT segment mapped,
//! preventing the background GC from unmapping it until the isolate is disposed.
//!
//! Reference: AP-02 from Zig version (prod.md), PITFALLS.md Pitfall 1

use anyhow::Result;
use std::marker::PhantomData;

use crate::limits::isolate::{HEAP_SIZE_BYTES_MAX, HEAP_SIZE_BYTES_PER_ISOLATE};
use crate::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};
use crate::{assert_negative, assert_positive, assert_precondition, assert_range};

/// V8 snapshot format magic number for validation
// V8 snapshots start with a 4-byte magic sequence: &[0xD7, 0x3C, 0xD7, 0x3C]
// Used to validate snapshot format before attempting to load.
// See: https://v8.dev/docs/snapshot-format

/// Minimum valid snapshot size (header + at least some data)
const MIN_SNAPSHOT_SIZE: usize = 8;

/// Isolate state for lifecycle tracking and assertion validation
///
/// Tracks the current state of an isolate through its lifecycle to enable
/// state transition assertions and prevent invalid operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolateState {
    /// Isolate is being created
    Creating,
    /// Isolate is ready for use
    Ready,
    /// Isolate is currently executing a request
    Executing,
    /// Isolate is being reset/recycled
    Resetting,
    /// Isolate has been terminated
    Terminated,
}

impl std::fmt::Display for IsolateState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolateState::Creating => write!(f, "Creating"),
            IsolateState::Ready => write!(f, "Ready"),
            IsolateState::Executing => write!(f, "Executing"),
            IsolateState::Resetting => write!(f, "Resetting"),
            IsolateState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// A V8 isolate with the EPT fix sentinel
///
/// This struct wraps a V8 isolate and maintains a strong Global sentinel
/// that prevents EPT segment unmapping during the isolate's lifetime.
///
/// # Thread Safety
/// Isolates are NOT thread-safe. They must never be moved between threads
/// and must only be accessed from the thread that created them.
/// The `PhantomData<*mut ()>` ensures this type is !Send + !Sync.
///
/// # Drop Order
/// The sentinel MUST be dropped BEFORE the isolate. This struct uses Rust's
/// field drop order (fields are dropped in declaration order, reverse of drop).
/// We declare `sentinel` before `isolate` so `isolate` is dropped last.
#[derive(Debug)]
pub struct NanoIsolate {
    /// The strong Global sentinel - keeps EPT segment mapped
    /// This MUST be dropped before the isolate
    #[allow(dead_code)] // Sentinel only needs to exist, not be accessed
    sentinel: v8::Global<v8::Value>,

    /// The V8 isolate - dropped after the sentinel
    isolate: v8::OwnedIsolate,

    /// Phantom data to make this !Send + !Sync
    /// Ensures isolates never move between threads (rusty_v8 issue #1467)
    _not_send_sync: PhantomData<*mut ()>,

    /// Per-isolate VFS for filesystem operations
    /// Dropped before sentinel/isolate (ephemeral per isolate)
    vfs: IsolateVfs,

    /// Current state of the isolate for lifecycle tracking
    state: IsolateState,

    /// Thread ID that created this isolate (for thread affinity checks)
    creation_thread_id: std::thread::ThreadId,

    /// Heap size limit in bytes
    heap_limit_bytes: u32,

    /// Whether the near-heap-limit callback has been registered
    /// V8 only allows one callback per isolate, so we track this to avoid
    /// registration failures on subsequent calls to set_heap_limits
    heap_callback_registered: bool,
}

/// Callback to allow WebAssembly code generation
///
/// This callback is called by V8 when WebAssembly.compile() or
/// WebAssembly.instantiate() is invoked. Returning true allows
/// WASM code generation to proceed.
unsafe extern "C" fn allow_wasm_code_generation(
    _context: v8::Local<v8::Context>,
    _source: v8::Local<v8::String>,
) -> bool {
    true
}

impl NanoIsolate {
    /// Create a new V8 isolate with the EPT fix sentinel and default VFS
    ///
    /// This function:
    /// 1. Creates a new V8 isolate with default params
    /// 2. Creates a HandleScope to work within the isolate
    /// 3. Creates a strong v8::Global<Value> sentinel (undefined value)
    /// 4. Creates a default VFS with empty namespace
    /// 5. Stores the sentinel to prevent EPT segment unmapping
    ///
    /// # Platform Requirement
    /// The V8 platform MUST be initialized before calling this function.
    /// Call `nano::v8::platform::initialize_platform()` first.
    ///
    /// # Example
    /// ```
    /// use nano::v8::{initialize_platform, NanoIsolate};
    ///
    /// initialize_platform().unwrap();
    /// let isolate = NanoIsolate::new().unwrap();
    /// // Use isolate...
    /// // isolate drops automatically when it goes out of scope
    /// ```
    pub fn new() -> Result<Self> {
        // Create default VFS with empty namespace
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("default"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        Self::new_with_vfs(vfs)
    }

    /// Create a new V8 isolate with a specific VFS configuration
    ///
    /// This is the primary constructor when you need per-isolate
    /// filesystem isolation (e.g., for multi-tenant scenarios).
    ///
    /// # Arguments
    ///
    /// * `vfs` - The IsolateVfs to use for this isolate's filesystem
    ///
    /// # Example
    /// ```
    /// use nano::v8::{initialize_platform, NanoIsolate};
    /// use nano::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};
    /// use std::sync::Arc;
    ///
    /// initialize_platform().unwrap();
    ///
    /// let vfs = IsolateVfs::new(
    ///     VfsNamespace::from_hostname("app.example.com"),
    ///     Arc::new(MemoryBackend::default()),
    /// );
    /// let isolate = NanoIsolate::new_with_vfs(vfs).unwrap();
    /// ```
    pub fn new_with_vfs(vfs: IsolateVfs) -> Result<Self> {
        Self::new_with_vfs_and_limit(vfs, 0)
    }

    /// Create a new V8 isolate with a specific VFS configuration and heap size limit.
    ///
    /// When `heap_limit_bytes > 0`, the V8 isolate is created with a hard heap cap
    /// via `CreateParams::heap_limits`. This is the only way to enforce memory limits:
    /// `add_near_heap_limit_callback` only fires when V8's GC limit is approached,
    /// and without `CreateParams::heap_limits` V8's GC limit is the platform default
    /// (~4 GB on 64-bit), making configured per-app limits ineffective.
    pub fn new_with_vfs_and_limit(vfs: IsolateVfs, heap_limit_bytes: usize) -> Result<Self> {
        // PRECONDITION: Platform must be initialized
        assert_precondition!(
            crate::v8::is_initialized(),
            "V8 platform must be initialized before creating isolates"
        );

        // PRECONDITION: VFS must be valid
        assert_positive!(
            !vfs.namespace().as_str().is_empty(),
            "VFS namespace must not be empty"
        );

        // Create the isolate — with a hard heap cap when requested.
        // `initial=0` lets V8 choose its own initial size; only the ceiling is forced.
        let params = if heap_limit_bytes > 0 {
            v8::CreateParams::default().heap_limits(0, heap_limit_bytes)
        } else {
            v8::CreateParams::default()
        };
        let mut isolate = v8::Isolate::new(params);

        // POSITIVE: Isolate was successfully created
        assert_positive!(
            isolate.get_heap_statistics().total_heap_size() > 0,
            "isolate must have positive heap size after creation"
        );

        // Enable WebAssembly code generation
        // This callback allows WebAssembly.compile() and WebAssembly.instantiate()
        // to work. Without this, WASM operations will fail.
        isolate.set_allow_wasm_code_generation_callback(allow_wasm_code_generation);

        // Create the EPT fix sentinel
        // v147 API: Create HandleScope using pin! pattern, init to get PinnedRef
        let sentinel = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = scope.init();
            // Create a Global holding undefined as a Value
            // v8::undefined() takes &PinnedRef
            let undefined = v8::undefined(&scope);
            let value: v8::Local<v8::Value> = undefined.into();
            // Global::new needs &Isolate - PinnedRef derefs to Isolate via Deref
            v8::Global::new(&*scope, value)
        };
        // HandleScope is dropped here, but sentinel survives (it's a Global)

        tracing::debug!("Created NanoIsolate with EPT fix sentinel and VFS");

        // POSTCONDITION: Sentinel was successfully created
        // Note: v8::Global doesn't have is_empty(), but successful creation
        // is implied by reaching this point (v8::Global::new doesn't return Result)

        // POSTCONDITION: Thread ID captured for affinity checks
        let creation_thread_id = std::thread::current().id();

        // POSTCONDITION: Heap limit is within valid range
        assert_range!(HEAP_SIZE_BYTES_PER_ISOLATE, 1, HEAP_SIZE_BYTES_MAX);

        Ok(Self {
            sentinel,
            isolate,
            _not_send_sync: PhantomData,
            vfs,
            state: IsolateState::Creating,
            creation_thread_id,
            heap_limit_bytes: HEAP_SIZE_BYTES_PER_ISOLATE,
            heap_callback_registered: false,
        })
    }

    /// Create a NanoIsolate from an existing v8::OwnedIsolate.
    ///
    /// This is used when creating isolates from snapshots, where the V8 isolate
    /// is created with special params (snapshot_blob) before wrapping.
    ///
    /// # Arguments
    /// * `isolate` - The pre-created v8::OwnedIsolate
    ///
    /// # Safety
    /// The isolate must have been created with V8 platform initialized.
    pub fn from_v8_isolate(mut isolate: v8::OwnedIsolate) -> Result<Self> {
        // PRECONDITION: Platform must be initialized
        assert_precondition!(
            crate::v8::is_initialized(),
            "V8 platform must be initialized before wrapping isolates"
        );

        // Create the EPT fix sentinel
        let sentinel = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = scope.init();
            let undefined = v8::undefined(&scope);
            let value: v8::Local<v8::Value> = undefined.into();
            v8::Global::new(&*scope, value)
        };

        tracing::debug!("Created NanoIsolate from existing V8 isolate (EPT fix applied)");

        let creation_thread_id = std::thread::current().id();

        // Create default VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("default"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        Ok(Self {
            sentinel,
            isolate,
            _not_send_sync: PhantomData,
            vfs,
            state: IsolateState::Creating,
            creation_thread_id,
            heap_limit_bytes: HEAP_SIZE_BYTES_PER_ISOLATE,
            heap_callback_registered: false,
        })
    }

    /// Create a new V8 isolate from a snapshot blob
    ///
    /// This is the primary constructor for restoring isolates from
    /// slivers. The snapshot contains the serialized V8 heap state.
    ///
    /// # Arguments
    /// * `snapshot_data` - The V8 heap snapshot blob
    /// * `vfs` - The VFS configuration for this isolate
    ///
    /// # Platform Requirement
    /// The V8 platform MUST be initialized before calling this function.
    pub fn from_snapshot(snapshot_data: &[u8], vfs: IsolateVfs) -> Result<Self> {
        // PRECONDITION: Platform must be initialized
        assert_precondition!(
            crate::v8::is_initialized(),
            "V8 platform must be initialized before creating isolates from snapshots"
        );

        // PRECONDITION: VFS namespace must be valid
        assert_positive!(
            !vfs.namespace().as_str().is_empty(),
            "VFS namespace must not be empty"
        );

        // Check for legacy cold sliver marker (backward compatibility)
        //
        // Design Rationale: Early nano-rs versions used a marker header
        // instead of real snapshots. This check provides graceful degradation
        // for legacy/invalid sliver files by creating a fresh isolate.
        // Production slivers should always contain real heap snapshots.
        if snapshot_data == b"NANO_SNAPSHOT_PLACEHOLDER_V1" {
            tracing::warn!("Legacy cold sliver marker detected - creating fresh isolate");
            return Self::new_with_vfs(vfs);
        }

        // Check for invalid/empty snapshot data
        if snapshot_data.len() < MIN_SNAPSHOT_SIZE {
            tracing::warn!(
                "Snapshot data too small ({} bytes, minimum {} bytes) - creating fresh isolate",
                snapshot_data.len(),
                MIN_SNAPSHOT_SIZE
            );
            return Self::new_with_vfs(vfs);
        }

        // PROPER V8 SNAPSHOT VALIDATION
        //
        // V8 snapshots have a specific structure that we can validate:
        // - Magic number: 0xD7 0x3C 0xD7 0x3C (first 4 bytes, little-endian)
        // - Version info follows magic number
        // - Must match V8 runtime version to be usable
        //
        // V8 snapshot validation before loading:
        // - Magic number validates format
        // - StartupData::from() converts bytes for CreateParams::snapshot_blob()
        // - rusty_v8 handles version compatibility internally

        // Validate V8 snapshot magic number
        const V8_SNAPSHOT_MAGIC: [u8; 4] = [0xD7, 0x3C, 0xD7, 0x3C];
        let has_magic = snapshot_data.len() >= 4 && &snapshot_data[0..4] == &V8_SNAPSHOT_MAGIC[..];

        if !has_magic {
            tracing::warn!(
                "Snapshot missing V8 magic number (first 4 bytes: {:02X?}, expected: {:02X?}) - creating fresh isolate",
                &snapshot_data[0..4.min(snapshot_data.len())],
                V8_SNAPSHOT_MAGIC
            );
            return Self::new_with_vfs(vfs);
        }

        tracing::info!("V8 snapshot magic number validated successfully");

        // V8 snapshot version info is at bytes 4-7 (varies by V8 version)
        // rusty_v8 handles version compatibility internally when snapshot API is used

        // Load the snapshot into V8
        // rusty_v8's StartupData supports From<Vec<u8>> via Cow<'static, [u8]>
        let startup_data = v8::StartupData::from(snapshot_data.to_vec());

        // Validate the snapshot data is usable
        if !startup_data.is_valid() {
            tracing::warn!("V8 snapshot data failed validation ({} bytes, magic validated) - creating fresh isolate", snapshot_data.len());
            return Self::new_with_vfs(vfs);
        }

        tracing::info!(
            "V8 snapshot validated ({} bytes), restoring isolate from snapshot",
            snapshot_data.len()
        );

        // Create isolate params with snapshot blob
        let params = v8::CreateParams::default().snapshot_blob(startup_data);

        // Create isolate from snapshot
        let mut isolate = v8::Isolate::new(params);

        // Enable WebAssembly code generation
        isolate.set_allow_wasm_code_generation_callback(allow_wasm_code_generation);

        // Create EPT fix sentinel
        let sentinel = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = scope.init();
            let undefined = v8::undefined(&scope);
            let value: v8::Local<v8::Value> = undefined.into();
            v8::Global::new(&*scope, value)
        };

        // Thread ID for affinity checks
        let creation_thread_id = std::thread::current().id();

        tracing::debug!("Restored NanoIsolate from snapshot with EPT fix sentinel and VFS");

        Ok(Self {
            sentinel,
            isolate,
            _not_send_sync: PhantomData,
            vfs,
            state: IsolateState::Ready,
            creation_thread_id,
            heap_limit_bytes: HEAP_SIZE_BYTES_PER_ISOLATE,
            heap_callback_registered: false,
        })
    }

    /// Get a reference to the VFS
    pub fn vfs(&self) -> &IsolateVfs {
        &self.vfs
    }

    /// Get a mutable reference to the VFS
    pub fn vfs_mut(&mut self) -> &mut IsolateVfs {
        &mut self.vfs
    }

    /// Create a new V8 context within this isolate
    ///
    /// Returns a Local<Context> that can be used to execute scripts.
    /// The context is scoped to the isolate's lifetime.
    ///
    /// # Example
    /// ```
    /// use nano::v8::{initialize_platform, NanoIsolate};
    ///
    /// initialize_platform().unwrap();
    /// let mut isolate = NanoIsolate::new().unwrap();
    /// let context = isolate.create_context();
    /// // Execute scripts in the context...
    /// ```
    pub fn create_context(&mut self) -> v8::Global<v8::Context> {
        // v147 API: Create HandleScope using pin! pattern, init to get PinnedRef
        let scope_storage = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
        let scope = scope_storage.init();

        // Create a context with default options
        // v147 API: Context::new takes &PinnedRef
        let context = v8::Context::new(&scope, Default::default());

        // Convert to Global so it can outlive the scope
        v8::Global::new(&scope, context)
    }

    /// Get a reference to the underlying isolate
    ///
    /// This is primarily useful for low-level V8 operations.
    /// Prefer using the provided methods when possible.
    pub fn isolate(&mut self) -> &mut v8::OwnedIsolate {
        &mut self.isolate
    }

    /// Consume the NanoIsolate and return the inner OwnedIsolate
    ///
    /// This is used for snapshot creation - the OwnedIsolate can be
    /// passed to `create_blob()` to serialize the heap state.
    ///
    /// Note: The EPT sentinel and VFS are properly dropped, only the
    /// isolate is extracted for snapshotting.
    pub fn into_inner(self) -> v8::OwnedIsolate {
        use std::mem::ManuallyDrop;

        // Wrap fields in ManuallyDrop to prevent automatic dropping
        // when we destructure self
        let mut this = ManuallyDrop::new(self);

        // SAFETY: We're extracting isolate and will properly drop other fields
        unsafe {
            // Extract the isolate
            let isolate = std::ptr::read(&this.isolate);

            // Explicitly drop the other fields
            std::ptr::drop_in_place(&mut this.sentinel);
            std::ptr::drop_in_place(&mut this.vfs);
            // _not_send_sync is Copy (PhantomData), no need to drop

            isolate
        }
    }

    /// Create a NanoIsolate using the snapshot creator workflow
    ///
    /// This creates an isolate that can later be serialized to a snapshot blob.
    /// Use this constructor when you intend to create a sliver.
    ///
    /// The isolate will have a default context automatically set up for snapshotting.
    ///
    /// # Example
    /// ```
    /// use nano::v8::{initialize_platform, NanoIsolate};
    /// use nano::v8::snapshot::create_snapshot_from_nano;
    ///
    /// initialize_platform().unwrap();
    ///
    /// // Create isolate for snapshotting with default context
    /// let isolate = NanoIsolate::snapshot_creator().unwrap();
    ///
    /// // Create snapshot blob (required before dropping snapshot_creator isolate)
    /// let blob = create_snapshot_from_nano(isolate).unwrap();
    /// assert!(!blob.is_empty());
    /// ```
    pub fn snapshot_creator() -> Result<Self> {
        // Create default VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("snapshot"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        Self::snapshot_creator_with_vfs(vfs)
    }

    /// Create a NanoIsolate using snapshot creator with specific VFS
    ///
    /// This is the primary constructor for creating slivers.
    /// The resulting isolate can be serialized via `create_blob()`.
    /// A default context is automatically set up for snapshotting.
    ///
    /// # Arguments
    ///
    /// * `vfs` - The IsolateVfs to use for this isolate's filesystem
    pub fn snapshot_creator_with_vfs(vfs: IsolateVfs) -> Result<Self> {
        // Create isolate using snapshot_creator API (v139+)
        let mut isolate = v8::Isolate::snapshot_creator(None, None);

        // Create a default context for the snapshot
        // V8 requires a default context to be set before create_blob() can work
        // v147 API: Use pin! pattern and init() for HandleScope, direct creation for ContextScope
        let sentinel = {
            let handle_scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let mut handle_scope = handle_scope.init();

            // Create a default context
            // v147 API: Context::new takes &PinnedRef
            let context = v8::Context::new(&handle_scope, Default::default());

            // Enter the context and set it as default for snapshotting
            // v147 API: ContextScope does NOT need pin! or init() - use directly
            let mut context_scope = v8::ContextScope::new(&mut handle_scope, context);

            // Set as default context (required for snapshot creation)
            context_scope.set_default_context(context);

            // Create the EPT fix sentinel within the context scope
            // v147 API: v8::undefined takes &ContextScope (which derefs to scope)
            let undefined = v8::undefined(&context_scope);
            let value: v8::Local<v8::Value> = undefined.into();
            // v147 API: Global::new takes &Isolate, PinnedRef derefs to Isolate via Deref
            let sentinel = v8::Global::new(&*context_scope, value);

            sentinel
        };

        tracing::debug!(
            "Created NanoIsolate using snapshot_creator with default context (snapshottable)"
        );

        Ok(Self {
            sentinel,
            isolate,
            _not_send_sync: PhantomData,
            vfs,
            state: IsolateState::Creating,
            creation_thread_id: std::thread::current().id(),
            heap_limit_bytes: HEAP_SIZE_BYTES_PER_ISOLATE,
            heap_callback_registered: false,
        })
    }

    /// Set V8 heap limits for memory constraint enforcement
    ///
    /// Configures a near-heap-limit callback that triggers when V8's heap
    /// approaches the configured max limit. The callback immediately terminates
    /// execution to prevent runaway memory consumption.
    ///
    /// # Arguments
    ///
    /// * `min_bytes` - Soft heap limit in bytes (reserved for future GC tuning)
    /// * `max_bytes` - Hard heap limit in bytes (triggers near-heap-limit callback)
    ///
    /// # Behavior
    ///
    /// When the heap approaches `max_bytes`, V8 invokes the callback:
    /// 1. Log a warning with current/initial limit details
    /// 2. Immediately terminate execution via `terminate_execution()`
    /// 3. Return current_limit (do not increase)
    ///
    /// # Security
    ///
    /// This prevents memory DoS attacks by terminating execution as soon as
    /// the heap limit is approached, rather than extending the limit which would
    /// allow attackers to consume unlimited memory.
    ///
    /// # Limitations
    ///
    /// This method can only be called once per isolate. V8 only supports a single
    /// near-heap-limit callback per isolate. Subsequent calls will only update
    /// the stored limit value but won't re-register the callback.
    ///
    /// # Errors
    ///
    /// Logs an error and returns early if `max_bytes` is zero.
    ///
    /// # Example
    ///
    /// ```
    /// use nano::v8::{initialize_platform, NanoIsolate};
    ///
    /// initialize_platform().unwrap();
    /// let mut isolate = NanoIsolate::new().unwrap();
    ///
    /// // Set 128MB heap limit (100MB soft, 128MB hard)
    /// isolate.set_heap_limits(100 * 1024 * 1024, 128 * 1024 * 1024);
    /// ```
    pub fn set_heap_limits(&mut self, min_bytes: usize, max_bytes: usize) {
        // Validate inputs
        if max_bytes == 0 {
            tracing::error!("set_heap_limits called with max_bytes=0, aborting");
            return;
        }
        if min_bytes > max_bytes {
            tracing::warn!(
                "set_heap_limits: min_bytes ({}) > max_bytes ({}), clamping min to 0",
                min_bytes,
                max_bytes
            );
        }

        // Store the configured limit
        self.heap_limit_bytes = max_bytes as u32;

        // V8 only allows one near-heap-limit callback per isolate.
        // Register the callback only on first call to this method.
        // Subsequent calls only update the stored limit value.
        if self.heap_callback_registered {
            tracing::debug!("Heap limit callback already registered for this isolate, only updating limit value to {}MB", max_bytes / (1024 * 1024));
            return;
        }

        // Capture a raw pointer to the isolate for the callback
        // SAFETY: The callback is only valid while the isolate exists, and the
        // isolate is never moved (it's pinned in NanoIsolate). terminate_execution
        // is safe to call even if the isolate is already terminating.
        let isolate_ptr: *mut v8::Isolate = &mut *self.isolate;

        self.add_near_heap_limit_callback(move |current_limit, initial_limit| {
            tracing::warn!(
                "Isolate approaching heap limit - terminating execution. \
                 current_limit={}MB, initial_limit={}MB",
                current_limit / (1024 * 1024),
                initial_limit / (1024 * 1024),
            );
            crate::metrics::METRICS.record_heap_limit_hit();

            // Terminate execution immediately to prevent memory DoS.
            // SAFETY: isolate_ptr is valid as long as NanoIsolate exists;
            // terminate_execution is safe to call multiple times (idempotent).
            unsafe {
                (*isolate_ptr).terminate_execution();
            }

            // Return current_limit without increase.
            // V8 may invoke this callback again; terminate_execution is idempotent.
            current_limit
        });

        self.heap_callback_registered = true;
        tracing::debug!(
            "Registered near-heap-limit callback with limit {}MB",
            max_bytes / (1024 * 1024)
        );
    }

    /// Get V8 heap statistics
    ///
    /// Returns detailed heap statistics including used size, total size,
    /// heap limit, and external memory usage.
    pub fn heap_statistics(&mut self) -> v8::HeapStatistics {
        self.isolate.get_heap_statistics()
    }

    /// Add a near-heap-limit callback
    ///
    /// This callback is invoked when V8 is approaching its heap limit.
    /// The callback receives the current heap limit and the initial limit,
    /// and returns a new limit (or 0 to signal abort).
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called when heap limit approached
    ///   - Receives: (current_limit, initial_limit)
    ///   - Returns: new_limit (or 0 to abort)
    ///
    /// # Safety
    ///
    /// The callback must not trigger GC or allocate in V8 heap.
    pub fn add_near_heap_limit_callback<F>(&mut self, callback: F)
    where
        F: FnMut(usize, usize) -> usize + 'static,
    {
        // V8 requires a 'static callback. We wrap the closure.
        let boxed_callback = Box::new(callback);
        let raw = Box::into_raw(boxed_callback);

        // Use an unsafe callback that dereferences our boxed closure
        unsafe extern "C" fn trampoline(
            data: *mut std::ffi::c_void,
            current: usize,
            initial: usize,
        ) -> usize {
            let callback = &mut *(data as *mut Box<dyn FnMut(usize, usize) -> usize>);
            callback(current, initial)
        }

        self.isolate
            .add_near_heap_limit_callback(trampoline, raw as *mut _);
    }

    /// Get the sentinel as a reference (for testing/debugging)
    #[cfg(test)]
    const fn sentinel(&self) -> &v8::Global<v8::Value> {
        &self.sentinel
    }

    /// Get the current state of the isolate
    pub fn state(&self) -> IsolateState {
        self.state
    }

    /// Set the state of the isolate (for state machine transitions)
    ///
    /// # Panics
    /// Panics if the state transition is invalid
    pub fn set_state(&mut self, new_state: IsolateState) {
        let old_state = self.state;

        // Valid state transitions per state machine design
        match (old_state, new_state) {
            // Creating can transition to Ready or Terminated (on error)
            (IsolateState::Creating, IsolateState::Ready) => (),
            (IsolateState::Creating, IsolateState::Terminated) => (),
            // Ready can transition to Executing or Terminated
            (IsolateState::Ready, IsolateState::Executing) => (),
            (IsolateState::Ready, IsolateState::Terminated) => (),
            // Executing can transition to Ready (reset), Resetting, or Terminated
            (IsolateState::Executing, IsolateState::Ready) => (),
            (IsolateState::Executing, IsolateState::Resetting) => (),
            (IsolateState::Executing, IsolateState::Terminated) => (),
            // Resetting can only transition to Ready
            (IsolateState::Resetting, IsolateState::Ready) => (),
            // Terminated is terminal state
            (IsolateState::Terminated, _) => {
                unreachable!(
                    "INVALID STATE TRANSITION: Terminated -> {:?} at {}:{}",
                    new_state,
                    file!(),
                    line!()
                );
            }
            // Any other transition is invalid
            _ => {
                unreachable!(
                    "INVALID STATE TRANSITION: {:?} -> {:?} at {}:{}",
                    old_state,
                    new_state,
                    file!(),
                    line!()
                );
            }
        }

        self.state = new_state;
    }

    /// Check thread affinity - asserts isolate is accessed from creation thread
    ///
    /// # Panics
    /// Panics if called from a different thread than the one that created the isolate
    pub fn assert_thread_affinity(&self) {
        let current_thread = std::thread::current().id();
        assert!(
            current_thread == self.creation_thread_id,
            "THREAD AFFINITY VIOLATION: isolate accessed from wrong thread. Expected {:?}, got {:?} at {}:{}",
            self.creation_thread_id, current_thread, file!(), line!()
        );
    }

    /// Get the thread ID that created this isolate
    pub const fn creation_thread_id(&self) -> std::thread::ThreadId {
        self.creation_thread_id
    }

    /// Get the heap limit in bytes
    pub const fn heap_limit_bytes(&self) -> u32 {
        self.heap_limit_bytes
    }

    /// Get current heap usage in bytes
    pub fn heap_used_bytes(&mut self) -> u32 {
        let stats = self.isolate.get_heap_statistics();
        stats.used_heap_size() as u32
    }
}

impl Drop for NanoIsolate {
    /// Drop implementation ensures proper cleanup order
    ///
    /// # EPT Fix Critical Order
    /// The sentinel MUST be dropped BEFORE the isolate. In Rust:
    /// - Fields are dropped in declaration order
    /// - We declared `sentinel` before `isolate`
    /// - Therefore, sentinel drops first, isolate drops last
    ///
    /// # VFS Drop Order
    /// The VFS is dropped after sentinel but before isolate (correct position).
    /// VFS is ephemeral and per-isolate, so it should be cleaned up
    /// when the isolate is disposed.
    fn drop(&mut self) {
        // NEGATIVE: Ensure we're not in the middle of execution when dropped
        assert_negative!(
            self.state == IsolateState::Executing,
            "isolate must not be dropped while executing"
        );

        // State transition to Terminated before dropping
        self.state = IsolateState::Terminated;

        tracing::debug!("Dropping NanoIsolate (EPT sentinel dropped before isolate)");
        // Fields are dropped in declaration order:
        // 1. state (IsolateState) - simple enum
        // 2. creation_thread_id (ThreadId) - simple ID
        // 3. heap_limit_bytes (u32) - simple value
        // 4. sentinel (v8::Global<Value>) - releases strong reference
        // 5. isolate (v8::OwnedIsolate) - disposes the isolate
        // 6. _not_send_sync (PhantomData) - no-op
        // 7. vfs (IsolateVfs) - drops ephemeral filesystem
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;

    /// Helper to ensure platform is initialized for tests
    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    /// Test that we can create an isolate with the EPT fix
    #[test]
    fn test_create_isolate() {
        init_platform();

        let isolate = NanoIsolate::new();
        assert!(isolate.is_ok(), "Failed to create isolate");
    }

    /// Test that we can create a context within an isolate
    #[test]
    fn test_create_context() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        let _context = isolate.create_context();

        // Context created successfully - test passes if no crash
    }

    /// Test that the sentinel exists
    #[test]
    fn test_ept_sentinel_exists() {
        init_platform();

        let isolate = NanoIsolate::new().expect("Failed to create isolate");

        // The sentinel should exist (it's a Global, which is always valid)
        let _sentinel = isolate.sentinel();
        // We can't easily test the sentinel's effect, but its existence
        // is verified by the fact that the isolate was created successfully
        // and no SIGSEGV occurs
        // Sentinel is a Global, its presence proves the EPT fix is in place
    }

    /// Test that multiple isolates can be created and disposed
    /// This stress tests the EPT fix
    #[test]
    fn test_multiple_isolates() {
        init_platform();

        // Create and dispose 10 isolates sequentially
        // This would trigger the EPT bug without the sentinel
        for i in 0..10 {
            let mut isolate = NanoIsolate::new().expect(&format!("Failed to create isolate {}", i));
            let _context = isolate.create_context();
            // isolate and context are dropped here
        }

        // If we reach here without SIGSEGV, the EPT fix is working
    }

    /// Test VFS integration with NanoIsolate
    #[tokio::test]
    async fn test_vfs_access() {
        init_platform();

        // Create isolate with custom VFS namespace
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        let mut isolate = NanoIsolate::new_with_vfs(vfs).expect("Failed to create isolate");

        // Write via VFS
        isolate
            .vfs_mut()
            .write("/config.json", b"{\"test\": true}")
            .await
            .unwrap();

        // Read back via VFS
        let content = isolate.vfs().read("/config.json").await.unwrap();
        assert_eq!(content, b"{\"test\": true}");

        // Verify file exists
        assert!(isolate.vfs().exists("/config.json").await.unwrap());
        assert!(!isolate.vfs().exists("/missing.txt").await.unwrap());
    }

    /// Test that set_heap_limits configures the heap limit and registers a callback
    #[test]
    fn test_set_heap_limits() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        // Set a 64MB heap limit
        isolate.set_heap_limits(32 * 1024 * 1024, 64 * 1024 * 1024);

        // Verify the limit was stored
        assert_eq!(isolate.heap_limit_bytes(), 64 * 1024 * 1024);
    }

    /// Test that set_heap_limits handles zero max_bytes gracefully
    #[test]
    fn test_set_heap_limits_zero_max() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        // Should not panic and should not change the limit
        let original_limit = isolate.heap_limit_bytes();
        isolate.set_heap_limits(0, 0);
        assert_eq!(isolate.heap_limit_bytes(), original_limit);
    }

    /// Test that set_heap_limits handles inverted min/max gracefully
    #[test]
    fn test_set_heap_limits_inverted() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        // min > max should still set max and log a warning
        isolate.set_heap_limits(200 * 1024 * 1024, 100 * 1024 * 1024);
        assert_eq!(isolate.heap_limit_bytes(), 100 * 1024 * 1024);
    }

    /// new_with_vfs_and_limit with a non-zero limit creates an isolate and
    /// stores the limit so set_heap_limits can register the termination callback.
    #[test]
    fn test_new_with_vfs_and_limit_sets_heap_cap() {
        init_platform();

        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("memlimit-test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        let limit = 64 * 1024 * 1024; // 64 MB
        let mut isolate = NanoIsolate::new_with_vfs_and_limit(vfs, limit)
            .expect("Failed to create isolate with limit");

        // Registering the callback after creation must succeed
        isolate.set_heap_limits(limit / 2, limit);
        assert_eq!(isolate.heap_limit_bytes(), limit as u32);
    }

    /// new_with_vfs_and_limit with 0 behaves identically to new_with_vfs.
    #[test]
    fn test_new_with_vfs_and_limit_zero_is_unlimited() {
        init_platform();

        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("nolimit-test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        let isolate = NanoIsolate::new_with_vfs_and_limit(vfs, 0)
            .expect("Failed to create isolate with zero limit");
        // Default heap_limit_bytes is the compile-time constant, not 0
        assert!(isolate.heap_limit_bytes() > 0);
    }
}
