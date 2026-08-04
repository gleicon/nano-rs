//! Worker pool implementation with thread-local isolate ownership
//!
//! This module provides the WorkerPool that manages N worker threads,
//! each owning a V8 isolate. Tasks are dispatched via MPSC channels
//! and responses are returned via oneshot channels.

use crate::v8::{initialize_platform, NanoIsolate};
use crate::worker::oom::OomMonitorBuilder;
use crate::worker::HandlerTask;
use crate::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};
use base64::Engine as _;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{anyhow, Result};

pub use crate::data_plane::with_worker_runtime;
pub use crate::worker::sliver_pool::SliverWorkerPool;

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use tracing::{debug, error, info, warn};

/// Read source code from VFS first, fall back to disk.
/// VFS path like `/index.js` is tried against the isolate's VFS;
/// on miss (or non-VFS entrypoints like absolute disk paths), falls back to `read_code_cached`.
fn read_code_vfs_or_disk(entrypoint: &str, vfs: &IsolateVfs) -> Result<std::sync::Arc<str>> {
    let vfs_result = crate::data_plane::with_worker_runtime(|h| {
        h.block_on(vfs.read(entrypoint))
    });
    if let Some(Ok(bytes)) = vfs_result {
        if let Ok(s) = String::from_utf8(bytes) {
            return Ok(s.into());
        }
    }
    crate::data_plane::read_code_cached(entrypoint)
}

fn compile_esm_handler(
    ctx_scope: &mut v8::ContextScope<'_, '_, v8::HandleScope<'_, v8::Context>>,
    entrypoint: &str,
    code: &str,
    vfs: IsolateVfs,
) -> Result<v8::Global<v8::Function>> {
    use crate::v8::module::{ModuleLoader, set_current_loader, module_resolve_callback};
    let ep_v8 = v8::String::new(ctx_scope, entrypoint)
        .ok_or_else(|| anyhow!("OOM: module origin"))?;
    let origin = v8::ScriptOrigin::new(
        ctx_scope, ep_v8.into(), 0, 0, true, -1, None, false, false, true, None,
    );
    let code_v8 = v8::String::new(ctx_scope, code)
        .ok_or_else(|| anyhow!("OOM: module source"))?;
    let mut esm_source = v8::script_compiler::Source::new(code_v8, Some(&origin));
    let esm_module = v8::script_compiler::compile_module(ctx_scope, &mut esm_source)
        .ok_or_else(|| anyhow!("ESM compile failed: {}", entrypoint))?;

    let mut loader = ModuleLoader::new(vfs);
    // SAFETY: loader lives until instantiate_module returns.
    unsafe { set_current_loader(Some(&mut loader as *mut _)); }
    let inst_ok = esm_module.instantiate_module(ctx_scope, module_resolve_callback).is_some();
    unsafe { set_current_loader(None); }
    if !inst_ok { return Err(anyhow!("ESM instantiate failed: {}", entrypoint)); }

    esm_module.evaluate(ctx_scope)
        .ok_or_else(|| anyhow!("ESM evaluate failed: {}", entrypoint))?;

    let ns = esm_module.get_module_namespace().to_object(ctx_scope)
        .ok_or_else(|| anyhow!("ESM namespace not object: {}", entrypoint))?;

    // Try `export function fetch` first, then `export default { fetch }`.
    let fk = v8::String::new(ctx_scope, "fetch");
    let fk_val = fk.and_then(|k| ns.get(ctx_scope, k.into())).filter(|v| v.is_function());
    let handler_val = match fk_val {
        Some(v) => v,
        None => {
            let dk = v8::String::new(ctx_scope, "default");
            let default_obj = dk.and_then(|k| ns.get(ctx_scope, k.into()))
                .and_then(|d| d.to_object(ctx_scope));
            let fk2 = v8::String::new(ctx_scope, "fetch");
            match default_obj.and_then(|o| fk2.and_then(|k| o.get(ctx_scope, k.into()))).filter(|v| v.is_function()) {
                Some(v) => v,
                None => return Err(anyhow!(
                    "No 'fetch' export in '{}'. Use: export function fetch(req){{...}}",
                    entrypoint
                )),
            }
        }
    };
    Ok(v8::Global::new(ctx_scope, handler_val.cast::<v8::Function>()))
}

/// WinterTC addEventListener shim — injected before every classic handler.
///
/// The Service Worker / WinterTC pattern uses `addEventListener("fetch", fn)`
/// where `fn(event)` receives a FetchEvent with `event.request` and
/// `event.respondWith(response)`. But pool.rs calls handlers as `fn(request)`
/// and reads the return value as the Response.
///
/// The wrapper bridges both conventions: it builds a fake FetchEvent, calls the
/// user callback, and returns whatever was passed to `respondWith`.
const WINTERTC_SHIM: &str = "var __nano_user_fetch;\
\nglobalThis.addEventListener = function(type, fn) {\
\n  if (type === 'fetch') {\
\n    globalThis.__nano_user_fetch = function(request) {\
\n      var captured;\
\n      var event = {\
\n        request: request,\
\n        respondWith: function(r) { captured = r; }\
\n      };\
\n      fn(event);\
\n      return captured;\
\n    };\
\n  }\
\n};\n";

fn compile_classic_handler(
    ctx_scope: &mut v8::ContextScope<'_, '_, v8::HandleScope<'_, v8::Context>>,
    entrypoint: &str,
    code: &str,
    context: v8::Local<'_, v8::Context>,
    cache_key: &str,
) -> Result<v8::Global<v8::Function>> {
    let shimmed = format!("{}{}", WINTERTC_SHIM, code);
    let code_v8 = v8::String::new(ctx_scope, &shimmed)
        .ok_or_else(|| anyhow!("V8 string alloc failed"))?;

    let unbound = if let Some(cached_bytes) = crate::data_plane::get_bytecode_cache(cache_key) {
        let cached_data = v8::script_compiler::CachedData::new(&cached_bytes);
        let mut source = v8::script_compiler::Source::new_with_cached_data(code_v8, None, cached_data);
        v8::script_compiler::compile_unbound_script(
            ctx_scope, &mut source,
            v8::script_compiler::CompileOptions::ConsumeCodeCache,
            v8::script_compiler::NoCacheReason::NoReason,
        )
    } else {
        let mut source = v8::script_compiler::Source::new(code_v8, None);
        let unbound = v8::script_compiler::compile_unbound_script(
            ctx_scope, &mut source,
            v8::script_compiler::CompileOptions::NoCompileOptions,
            v8::script_compiler::NoCacheReason::NoReason,
        );
        if let Some(ref u) = unbound {
            if let Some(cache) = u.create_code_cache() {
                let bytes: std::sync::Arc<[u8]> = (&**cache).into();
                crate::data_plane::set_bytecode_cache(cache_key, bytes);
            }
        }
        unbound
    };

    let script = unbound
        .ok_or_else(|| anyhow!("Script compile failed for '{}'", entrypoint))?
        .bind_to_current_context(ctx_scope);
    script.run(ctx_scope)
        .ok_or_else(|| anyhow!("Script execution failed for '{}'", entrypoint))?;

    let global_obj = context.global(ctx_scope);
    let nano_k = v8::String::new(ctx_scope, "__nano_user_fetch")
        .ok_or_else(|| anyhow!("V8 OOM allocating key"))?;
    let fetch_k = v8::String::new(ctx_scope, "fetch")
        .ok_or_else(|| anyhow!("V8 OOM allocating key"))?;
    global_obj.get(ctx_scope, nano_k.into())
        .filter(|v| v.is_function())
        .or_else(|| global_obj.get(ctx_scope, fetch_k.into()).filter(|v| v.is_function()))
        .map(|f| v8::Global::new(ctx_scope, f.cast::<v8::Function>()))
        .ok_or_else(|| anyhow!(
            "No fetch handler found in '{}'. Export a 'fetch' function.",
            entrypoint
        ))
}

/// Dropping closes the channel, signaling the worker to exit.
#[derive(Debug)]
pub struct WorkerHandle {
    pub id: u32,
    thread: Option<JoinHandle<()>>,
    task_tx: mpsc::Sender<HandlerTask>,
}

impl WorkerHandle {
    pub fn send(&self, task: HandlerTask) -> Result<()> {
        self.task_tx
            .send(task)
            .map_err(|_| anyhow!("Worker {} channel closed", self.id))
    }

    fn take_thread(&mut self) -> Option<JoinHandle<()>> {
        self.thread.take()
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        debug!("WorkerHandle {} dropped", self.id);
    }
}

/// Legacy pool — prefer [`SliverWorkerPool`], [`EntrypointWorkerPool`], or [`WorkQueue`] for new code.
///
/// Each worker owns one V8 isolate (thread-local). Tasks dispatched via MPSC, round-robin.
pub struct WorkerPool {
    workers: Vec<WorkerHandle>,
    pub worker_count: u32,
    pub hostname: String,
    next_worker: AtomicU32,
    pub(crate) vfs_backend: crate::vfs::VfsBackendEnum,
    memory_limit_mb: u32,
    #[allow(dead_code)]
    env_vars: std::collections::HashMap<String, String>,
}

impl std::fmt::Debug for WorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerPool")
            .field("workers", &self.workers.len())
            .field("worker_count", &self.worker_count)
            .field("hostname", &self.hostname)
            .field("next_worker", &self.next_worker)
            .field("vfs_backend", &"<dyn VfsBackend>")
            .field("memory_limit_mb", &self.memory_limit_mb)
            .finish()
    }
}

impl WorkerPool {
    /// Create a pool with an explicit VFS backend.
    ///
    /// Uses an `Entrypoint` source — the actual JS file is resolved from
    /// `task.entrypoint` at dispatch time, not from the placeholder here.
    pub fn with_backend(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        vfs_backend: crate::vfs::VfsBackendEnum,
    ) -> Self {
        Self::with_source_and_backend(
            hostname,
            worker_count,
            memory_limit_mb,
            vfs_backend,
            crate::worker::AppSource::entrypoint("index.js"),
        )
    }

    /// # Panics
    ///
    /// Panics if the V8 platform is not initialized.
    pub fn new(hostname: String, worker_count: u32, memory_limit_mb: u32) -> Self {
        // "index.js" placeholder — actual entrypoint comes from task.entrypoint at dispatch time.
        Self::with_source_and_backend(
            hostname,
            worker_count,
            memory_limit_mb,
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
            crate::worker::AppSource::entrypoint("index.js"),
        )
    }

    /// Get a reference to the shared VFS backend
    ///
    /// This is useful for testing and administrative operations
    /// that need to inspect or modify the filesystem.
    pub fn vfs_backend(&self) -> &crate::vfs::VfsBackendEnum {
        &self.vfs_backend
    }

    /// Dispatch a task to a worker
    pub fn dispatch(&self, task: HandlerTask) -> Result<()> {
        let worker_idx = self.next_worker.fetch_add(1, Ordering::SeqCst) % self.worker_count;
        let worker_idx = worker_idx as usize;

        self.workers[worker_idx]
            .send(task)
            .map_err(|e| anyhow!("Failed to dispatch to worker {}: {}", worker_idx, e))
    }

    pub fn shutdown(mut self) -> Result<()> {
        info!("Shutting down WorkerPool for {}", self.hostname);

        let mut handles: Vec<_> = self
            .workers
            .drain(..)
            .map(|mut w| (w.id, w.take_thread()))
            .collect();

        for (id, handle) in handles.drain(..) {
            if let Some(h) = handle {
                debug!("Waiting for worker {} to exit", id);
                match h.join() {
                    Ok(_) => debug!("Worker {} exited cleanly", id),
                    Err(_) => warn!("Worker {} panicked during shutdown", id),
                }
            }
        }

        info!("WorkerPool for {} shut down complete", self.hostname);
        Ok(())
    }

    /// Get the number of workers in this pool
    pub fn worker_count(&self) -> u32 {
        self.worker_count
    }

    /// # Panics
    ///
    /// Panics if V8 platform is not initialized or worker_count is 0.
    pub fn with_source(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        source: crate::worker::AppSource,
    ) -> Self {
        use crate::vfs::MemoryBackend;
        use crate::worker::AppSource;

        let vfs_backend = match &source {
            AppSource::Entrypoint { path } => {
                let path_obj = std::path::Path::new(path);
                let base_dir = path_obj
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let base_dir_for_thread = base_dir.clone();
                let base_dir_for_error = base_dir.clone();
                // DiskBackend::new is async; block on it from a spawned thread.
                let backend_result = std::thread::spawn(move || {
                    match tokio::runtime::Runtime::new() {
                        Ok(rt) => rt.block_on(async {
                            crate::vfs::DiskBackend::new(&base_dir_for_thread).await
                        }),
                        Err(e) => Err(crate::vfs::VfsError::IoError(format!("Failed to create tokio runtime: {}", e)))
                    }
                }).join();

                match backend_result {
                    Ok(Ok(disk_backend)) => {
                        tracing::info!(
                            "Created DiskBackend for entrypoint app at hostname: {}, base_dir: {:?}",
                            hostname,
                            base_dir
                        );
                        crate::vfs::VfsBackendEnum::disk(disk_backend)
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            "Failed to create DiskBackend for entrypoint app at {:?}, falling back to MemoryBackend: {}",
                            base_dir_for_error,
                            e
                        );
                        crate::vfs::VfsBackendEnum::memory(MemoryBackend::default())
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Thread panic creating DiskBackend for entrypoint app at {:?}, falling back to MemoryBackend: {:?}",
                            base_dir_for_error,
                            e
                        );
                        crate::vfs::VfsBackendEnum::memory(MemoryBackend::default())
                    }
                }
            }
            AppSource::Sliver { .. } => {
                tracing::debug!("Using MemoryBackend for sliver app at hostname: {}", hostname);
                crate::vfs::VfsBackendEnum::memory(MemoryBackend::default())
            }
            AppSource::Static { .. } => {
                panic!("Static sources should not create WorkerPool - use StaticPool instead");
            }
        };

        Self::with_source_and_backend(hostname, worker_count, memory_limit_mb, vfs_backend, source)
    }

    /// Create a pool with an explicit VFS backend and app source.
    ///
    /// This is the primary constructor. All other constructors delegate here.
    ///
    /// # Panics
    ///
    /// Panics if the V8 platform is not initialized and initialization fails,
    /// or if `worker_count` is 0.
    pub fn with_source_and_backend(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        vfs_backend: crate::vfs::VfsBackendEnum,
        source: crate::worker::AppSource,
    ) -> Self {
        Self::with_source_backend_and_env(hostname, worker_count, memory_limit_mb, vfs_backend, source, std::collections::HashMap::new())
    }

    pub fn with_source_backend_and_env(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        vfs_backend: crate::vfs::VfsBackendEnum,
        source: crate::worker::AppSource,
        env_vars: std::collections::HashMap<String, String>,
    ) -> Self {
        use crate::worker::AppSource;

        if !crate::v8::is_initialized() {
            initialize_platform().expect("Failed to initialize V8 platform");
        }

        assert!(worker_count > 0, "Worker count must be at least 1");

        if source.is_static() {
            panic!("Static sources should not create WorkerPool - use StaticPool instead");
        }

        let hostname_for_workers = hostname.clone();
        let vfs_backend_for_workers = vfs_backend.clone();
        let source_for_workers = source.clone();
        let env_vars_for_workers = env_vars.clone();

        let mut workers = Vec::with_capacity(worker_count as usize);

        for id in 0..worker_count {
            let worker_hostname = hostname_for_workers.clone();
            let worker_vfs_backend = vfs_backend_for_workers.clone();
            let worker_source = source_for_workers.clone();
            let worker_env_vars = env_vars_for_workers.clone();
            let (task_tx, task_rx) = mpsc::channel::<HandlerTask>();

            // Spawn unified worker thread with persistent V8 scope lifecycle.
            let thread = thread::spawn(move || {
                info!("UnifiedWorker {} starting for {}", id, worker_hostname);

                let rt = match tokio::runtime::Runtime::new() {
                    Ok(r) => r,
                    Err(e) => { error!("Worker {}: tokio runtime failed: {}", id, e); return; }
                };
                crate::data_plane::set_worker_runtime(rt.handle().clone());

                let oom_monitor = if memory_limit_mb > 0 {
                    Some(
                        OomMonitorBuilder::new(format!("worker_{}_{}", worker_hostname, id))
                            .with_limit_mb(memory_limit_mb)
                            .for_hostname(&worker_hostname)
                            .build(),
                    )
                } else {
                    None
                };

                const MAX_REQUESTS_PER_ISOLATE: u32 = 10_000;
                let mut first_isolate = true;

                // Outer loop: one iteration per isolate lifetime.
                'isolate: loop {
                    let namespace = VfsNamespace::from_hostname(&worker_hostname);
                    let vfs = IsolateVfs::new(namespace, worker_vfs_backend.clone());

                    let heap_limit_bytes = if memory_limit_mb > 0 {
                        memory_limit_mb as usize * 1024 * 1024
                    } else {
                        0
                    };

                    let mut nano = match &worker_source {
                        AppSource::Sliver { data } if first_isolate => {
                            first_isolate = false;
                            if let Err(e) = rt.block_on(data.restore_to_vfs(&vfs)) {
                                warn!("Worker {}: VFS restore failed: {}", id, e);
                            } else {
                                debug!("Worker {}: restored {} VFS entries", id, data.vfs_entries.len());
                            }
                            if data.bytecode_matches_v8() {
                                if let Some(ref bc) = data.bytecode {
                                    let entrypoint = data.entrypoint();
                                    let cache_key = format!("{}::{}", worker_hostname, entrypoint);
                                    let bytes: std::sync::Arc<[u8]> = bc.as_slice().into();
                                    crate::data_plane::set_bytecode_cache(&cache_key, bytes);
                                    debug!("Worker {}: sliver bytecode pre-loaded for '{}'", id, entrypoint);
                                }
                            }
                            match NanoIsolate::new_with_vfs_and_limit(vfs, heap_limit_bytes) {
                                Ok(iso) => iso,
                                Err(e) => { error!("Worker {}: isolate failed: {}", id, e); return; }
                            }
                        }
                        AppSource::Entrypoint { .. } if first_isolate => {
                            first_isolate = false;
                            match NanoIsolate::new_with_vfs_and_limit(vfs, heap_limit_bytes) {
                                Ok(iso) => iso,
                                Err(e) => { error!("Worker {}: isolate failed: {}", id, e); return; }
                            }
                        }
                        AppSource::Static { .. } => {
                            error!("Worker {}: Static source in unified worker — should not happen", id);
                            return;
                        }
                        _ => {
                            // For sliver: VFS is re-populated each isolate cycle.
                            if let AppSource::Sliver { data } = &worker_source {
                                if let Err(e) = rt.block_on(data.restore_to_vfs(&vfs)) {
                                    warn!("Worker {}: VFS restore on recycle failed: {}", id, e);
                                }
                            }
                            match NanoIsolate::new_with_vfs_and_limit(vfs, heap_limit_bytes) {
                                Ok(iso) => iso,
                                Err(e) => { error!("Worker {}: isolate create failed: {}", id, e); return; }
                            }
                        }
                    };

                    // Register the near-heap-limit callback so V8 terminates execution
                    // (rather than OOM-crashing the process) when the isolate hits its ceiling.
                    // This works in tandem with CreateParams::heap_limits set above.
                    if heap_limit_bytes > 0 {
                        nano.set_heap_limits(heap_limit_bytes / 2, heap_limit_bytes);
                    }

                    // Expose VFS to Nano.fs.* callbacks via thread-local.
                    // Must be set per-isolate-lifetime; the inner request loop
                    // reuses the same isolate (and same VFS) for all requests.
                    let vfs_clone = nano.vfs().clone();
                    let vfs_arc = std::sync::Arc::new(vfs_clone.clone());
                    crate::runtime::vfs_bindings::set_current_vfs(Some(vfs_arc));
                    // Expose app env vars as Nano.env frozen object.
                    crate::runtime::vfs_bindings::set_current_env(worker_env_vars.clone());

                    // Raw pointer for CPU timeout guards.
                    // SAFETY: nano lives for the entire scope block below.
                    let iso_ptr: *mut v8::Isolate = &mut **nano.isolate();

                    // === PERSISTENT SCOPE BLOCK ===
                    {
                        let scope_pin = std::pin::pin!(v8::HandleScope::new(nano.isolate()));
                        let mut scope = scope_pin.init();
                        let context = v8::Context::new(&scope, Default::default());
                        // Security: block eval() and new Function() — matches Cloudflare Workers.
                        context.set_allow_generation_from_strings(false);
                        crate::runtime::apis::RuntimeAPIs::bind_all(&mut scope, context);
                        let mut ctx_scope = v8::ContextScope::new(&mut scope, context);

                        let mut handler_cache: std::collections::HashMap<
                            String, v8::Global<v8::Function>
                        > = std::collections::HashMap::new();

                        let mut served: u32 = 0;
                        let isolate_id = format!("{}:{}", worker_hostname, id);

                        'requests: loop {
                            if served >= MAX_REQUESTS_PER_ISOLATE {
                                info!("Worker {}: recycling isolate after {} requests", id, served);
                                break 'requests;
                            }

                            // Signal V8 GC scheduler that we're idle while waiting.
                            // SAFETY: iso_ptr is valid for the duration of the scope block.
                            unsafe { (*iso_ptr).set_idle(true); }
                            let task = match task_rx.recv() {
                                Ok(t) => t,
                                Err(_) => { debug!("Worker {}: channel closed", id); break 'isolate; }
                            };
                            unsafe { (*iso_ptr).set_idle(false); }

                            // OOM pre-check
                            if let Some(ref mon) = oom_monitor {
                                // SAFETY: iso_ptr valid for scope block duration
                                let iso_ref: &mut v8::Isolate = unsafe { &mut *iso_ptr };
                                if let Err(oom) = mon.check(iso_ref) {
                                    mon.log_oom_event(&oom, &task.request_id);
                                    let _ = task.response_tx.send(Ok(mon.create_oom_response(&oom)));
                                    break 'requests;
                                }
                            }

                            let t0 = std::time::Instant::now();
                            let request_id = task.request_id.clone();

                            let entrypoint = match &worker_source {
                                AppSource::Sliver { data } => data.entrypoint(),
                                _ => task.entrypoint.clone(),
                            };

                            // Compile + cache handler (once per entrypoint, per isolate lifetime)
                            if !handler_cache.contains_key(&entrypoint) {
                                let code = match read_code_vfs_or_disk(&entrypoint, &vfs_clone) {
                                    Ok(c) => c,
                                    Err(e) => {
                                        let _ = task.response_tx.send(Err(e));
                                        continue 'requests;
                                    }
                                };

                                let is_esm = crate::v8::module::is_esm_module(&code);
                                let cache_key = format!("{}::{}", worker_hostname, entrypoint);
                                let handler_result = if is_esm {
                                    compile_esm_handler(&mut ctx_scope, &entrypoint, &code, vfs_clone.clone())
                                } else {
                                    compile_classic_handler(&mut ctx_scope, &entrypoint, &code, context, &cache_key)
                                };
                                match handler_result {
                                    Ok(g) => {
                                        handler_cache.insert(entrypoint.clone(), g);
                                        info!("Worker {}: {} handler cached for '{}'", id,
                                              if is_esm { "ESM" } else { "classic" }, entrypoint);
                                    }
                                    Err(e) => {
                                        let _ = task.response_tx.send(Err(e));
                                        continue 'requests;
                                    }
                                }
                            }

                            // --- WebSocket mode (D-01 pin-a-worker, D-10b isolate recycle) ---
                            if let Some(ws_channels) = task.ws {
                                use crate::worker::tenant_pool::{
                                    WS_OUTBOUND, WS_ACCEPTED, WS_MESSAGE_HANDLERS,
                                    WS_CLOSE_HANDLERS, WS_ERROR_HANDLERS,
                                    set_ws_readystate, clear_ws_thread_locals,
                                };
                                WS_OUTBOUND.with(|tx| *tx.borrow_mut() = Some(ws_channels.outbound_tx.clone()));
                                WS_ACCEPTED.with(|a| a.set(false));
                                WS_MESSAGE_HANDLERS.with(|h| h.borrow_mut().clear());
                                WS_CLOSE_HANDLERS.with(|h| h.borrow_mut().clear());
                                WS_ERROR_HANDLERS.with(|h| h.borrow_mut().clear());

                                                if let Some(handler_g) = handler_cache.get(&entrypoint) {
                                    let gobj = context.global(&mut ctx_scope);
                                    let hlocal = v8::Local::new(&mut ctx_scope, handler_g);
                                    if let Some(url_str) = v8::String::new(&mut ctx_scope, &task.request.url().href()) {
                                        let tc_s = v8::TryCatch::new(&mut *ctx_scope);
                                        let tc_pin = std::pin::pin!(tc_s);
                                        let tc = tc_pin.init();
                                        let _ = hlocal.call(&tc, gobj.into(), &[url_str.into()]);
                                    }
                                }

                                set_ws_readystate(&mut ctx_scope, 1);
                                let _ = task.response_tx; // 101 already sent by router

                                info!("Worker {}: entering ws_messages loop for '{}'", id, entrypoint);
                                let idle_dur = std::time::Duration::from_millis(30_000);

                                // Expands to: OOM check → log → send Close(Error/OOM) → break $label.
                                macro_rules! ws_oom_break {
                                    ($label:lifetime) => {
                                        if let Some(ref mon) = oom_monitor {
                                            // SAFETY: iso_ptr valid for scope block duration
                                            let iso_ref: &mut v8::Isolate = unsafe { &mut *iso_ptr };
                                            if let Err(oom) = mon.check(iso_ref) {
                                                mon.log_oom_event(&oom, &task.request_id);
                                                let _ = ws_channels.outbound_tx.send(tungstenite::Message::Close(Some(
                                                    tungstenite::protocol::CloseFrame {
                                                        code: tungstenite::protocol::frame::coding::CloseCode::Error,
                                                        reason: std::borrow::Cow::Borrowed("OOM"),
                                                    }
                                                )));
                                                break $label;
                                            }
                                        }
                                    };
                                }

                                // Expands to: call every handler in $handlers with $event as argument.
                                macro_rules! ws_dispatch {
                                    ($handlers:expr, $event:expr) => {{
                                        let gobj = context.global(&mut ctx_scope);
                                        $handlers.with(|cell| {
                                            for hg in cell.borrow().iter() {
                                                let hl = v8::Local::new(&mut ctx_scope, hg);
                                                let tc_s = v8::TryCatch::new(&mut *ctx_scope);
                                                let tc_pin = std::pin::pin!(tc_s);
                                                let tc = tc_pin.init();
                                                let _ = hl.call(&tc, gobj.into(), &[$event.into()]);
                                            }
                                        });
                                    }};
                                }

                                'ws_messages: loop {
                                    match ws_channels.inbound_rx.recv_timeout(idle_dur) {
                                        Ok(tungstenite::Message::Text(s)) => {
                                            ws_oom_break!('ws_messages);
                                            // SAFETY: iso_ptr valid for isolate lifetime; V8 terminate_execution is thread-safe
                                            let _cg = if task.cpu_time_limit_ms > 0 { Some(crate::data_plane::CpuTimeoutGuard::new(unsafe { &mut *iso_ptr }, task.cpu_time_limit_ms)) } else { None };
                                            let event = v8::Object::new(&mut ctx_scope);
                                            if let (Some(tk), Some(tv), Some(dk), Some(dv)) = (
                                                v8::String::new(&mut ctx_scope, "type"),
                                                v8::String::new(&mut ctx_scope, "message"),
                                                v8::String::new(&mut ctx_scope, "data"),
                                                v8::String::new(&mut ctx_scope, s.as_str()),
                                            ) {
                                                event.set(&mut ctx_scope, tk.into(), tv.into());
                                                event.set(&mut ctx_scope, dk.into(), dv.into());
                                                ws_dispatch!(WS_MESSAGE_HANDLERS, event);
                                            }
                                        }
                                        Ok(tungstenite::Message::Binary(b)) => {
                                            ws_oom_break!('ws_messages);
                                            // SAFETY: iso_ptr valid for isolate lifetime; V8 terminate_execution is thread-safe
                                            let _cg = if task.cpu_time_limit_ms > 0 { Some(crate::data_plane::CpuTimeoutGuard::new(unsafe { &mut *iso_ptr }, task.cpu_time_limit_ms)) } else { None };
                                            let ab_store = v8::ArrayBuffer::new_backing_store_from_vec(b);
                                            let shared = ab_store.make_shared();
                                            let ab = v8::ArrayBuffer::with_backing_store(&mut ctx_scope, &shared);
                                            let event = v8::Object::new(&mut ctx_scope);
                                            if let (Some(tk), Some(tv), Some(dk)) = (
                                                v8::String::new(&mut ctx_scope, "type"),
                                                v8::String::new(&mut ctx_scope, "message"),
                                                v8::String::new(&mut ctx_scope, "data"),
                                            ) {
                                                event.set(&mut ctx_scope, tk.into(), tv.into());
                                                event.set(&mut ctx_scope, dk.into(), ab.into());
                                                ws_dispatch!(WS_MESSAGE_HANDLERS, event);
                                            }
                                        }
                                        Ok(tungstenite::Message::Close(frame)) => {
                                            set_ws_readystate(&mut ctx_scope, 3);
                                            let (code_val, reason_str) = frame
                                                .map(|f| (u16::from(f.code), f.reason.into_owned()))
                                                .unwrap_or((1000, String::new()));
                                            let close_event = v8::Object::new(&mut ctx_scope);
                                            if let (Some(tyk), Some(tyv), Some(ck), Some(rk), Some(rv), Some(wck)) = (
                                                v8::String::new(&mut ctx_scope, "type"),
                                                v8::String::new(&mut ctx_scope, "close"),
                                                v8::String::new(&mut ctx_scope, "code"),
                                                v8::String::new(&mut ctx_scope, "reason"),
                                                v8::String::new(&mut ctx_scope, &reason_str),
                                                v8::String::new(&mut ctx_scope, "wasClean"),
                                            ) {
                                                let code_int = v8::Integer::new(&mut ctx_scope, code_val as i32);
                                                let was_clean = v8::Boolean::new(&mut ctx_scope, true);
                                                close_event.set(&mut ctx_scope, tyk.into(), tyv.into());
                                                close_event.set(&mut ctx_scope, ck.into(), code_int.into());
                                                close_event.set(&mut ctx_scope, rk.into(), rv.into());
                                                close_event.set(&mut ctx_scope, wck.into(), was_clean.into());
                                                ws_dispatch!(WS_CLOSE_HANDLERS, close_event);
                                            }
                                            break 'ws_messages;
                                        }
                                        Ok(tungstenite::Message::Ping(_)) | Ok(tungstenite::Message::Pong(_)) => continue 'ws_messages,
                                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => { info!("Worker {}: WS idle timeout", id); break 'ws_messages; }
                                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                            set_ws_readystate(&mut ctx_scope, 3);
                                            let error_event = v8::Object::new(&mut ctx_scope);
                                            if let (Some(tyk), Some(tyv)) = (
                                                v8::String::new(&mut ctx_scope, "type"),
                                                v8::String::new(&mut ctx_scope, "error"),
                                            ) {
                                                error_event.set(&mut ctx_scope, tyk.into(), tyv.into());
                                                ws_dispatch!(WS_ERROR_HANDLERS, error_event);
                                            }
                                            break 'ws_messages;
                                        }
                                        Ok(_) => continue 'ws_messages,
                                    }
                                }
                                clear_ws_thread_locals();
                                break 'requests; // D-10b: fresh isolate per WS connection
                            }
                            // Clear any stale termination flag from a previous request
                            // before arming the new timeout. The previous guard's Drop already
                            // called cancel_terminate_execution(), but this is cheap insurance.
                            unsafe { (*iso_ptr).cancel_terminate_execution(); }

                            // CPU timeout guard
                            let _timeout = if task.cpu_time_limit_ms > 0 {
                                // SAFETY: iso_ptr is valid for this isolate's lifetime.
                                // CpuTimeoutGuard calls terminate_execution() from a timer thread,
                                // which V8 documents as safe to call from any thread.
                                let iso_ref: &mut v8::Isolate = unsafe { &mut *iso_ptr };
                                Some(crate::data_plane::CpuTimeoutGuard::new(iso_ref, task.cpu_time_limit_ms))
                            } else {
                                None
                            };

                            // Execute handler using persistent context
                            // handler_cache.get is infallible: just inserted above if missing.
                            let handler_g = handler_cache.get(&entrypoint)
                                .expect("handler must be cached: just inserted in block above");
                            let global_obj = context.global(&mut ctx_scope);
                            let handler_local = v8::Local::new(&mut ctx_scope, handler_g);

                            let result: anyhow::Result<crate::http::NanoResponse> = (|| {
                                let url_str = v8::String::new(&mut ctx_scope, &task.request.url().href())
                                    .ok_or_else(|| anyhow!("URL string alloc failed"))?;
                                let opts = v8::Object::new(&mut ctx_scope);

                                let mk = v8::String::new(&mut ctx_scope, "method").ok_or_else(|| anyhow!("method key"))?;
                                let mv = v8::String::new(&mut ctx_scope, task.request.method()).ok_or_else(|| anyhow!("method val"))?;
                                opts.set(&mut ctx_scope, mk.into(), mv.into());

                                let hk = v8::String::new(&mut ctx_scope, "headers").ok_or_else(|| anyhow!("headers key"))?;
                                let hck = v8::String::new(&mut ctx_scope, "Headers").ok_or_else(|| anyhow!("Headers key"))?;
                                let hctor = global_obj.get(&mut ctx_scope, hck.into())
                                    .filter(|v| v.is_function())
                                    .ok_or_else(|| anyhow!("Headers constructor not found"))?
                                    .cast::<v8::Function>();
                                let hinit = v8::Object::new(&mut ctx_scope);
                                for (name, vals) in task.request.headers().entries() {
                                    let val = vals.join(", ");
                                    if let (Some(k), Some(v)) = (
                                        v8::String::new(&mut ctx_scope, name),
                                        v8::String::new(&mut ctx_scope, &val),
                                    ) {
                                        hinit.set(&mut ctx_scope, k.into(), v.into());
                                    }
                                }
                                let hobj = hctor.new_instance(&mut ctx_scope, &[hinit.into()])
                                    .ok_or_else(|| anyhow!("Headers instantiation failed"))?;
                                opts.set(&mut ctx_scope, hk.into(), hobj.into());

                                if let Some(body) = task.request.body() {
                                    let bk = v8::String::new(&mut ctx_scope, "body").ok_or_else(|| anyhow!("body key"))?;
                                    let encoded = base64::engine::general_purpose::STANDARD.encode(body);
                                    let bv = v8::String::new(&mut ctx_scope, &encoded).ok_or_else(|| anyhow!("body val"))?;
                                    opts.set(&mut ctx_scope, bk.into(), bv.into());
                                }

                                let rck = v8::String::new(&mut ctx_scope, "Request").ok_or_else(|| anyhow!("Request key"))?;
                                let rctor = global_obj.get(&mut ctx_scope, rck.into())
                                    .filter(|v| v.is_function())
                                    .ok_or_else(|| anyhow!("Request constructor not found"))?
                                    .cast::<v8::Function>();
                                let js_req = rctor.new_instance(&mut ctx_scope, &[url_str.into(), opts.into()])
                                    .ok_or_else(|| anyhow!("Request instantiation failed"))?;

                                // TryCatch intercepts any JS exception thrown by the handler.
                                // Dropping tc at closure exit clears the pending exception from
                                // the isolate, preventing isolate poisoning across requests.
                                // Must pin-and-init like HandleScope — TryCatch::new returns ScopeStorage.
                                let tc_storage = v8::TryCatch::new(&mut *ctx_scope);
                                let tc_pin = std::pin::pin!(tc_storage);
                                let mut tc = tc_pin.init();

                                // Clear any stale interval state from a previous request.
                                crate::runtime::apis::clear_pending_intervals();
                                crate::runtime::apis::clear_pending_timeouts();

                                let call_result = handler_local.call(&tc, global_obj.into(), &[js_req.into()]);

                                let resolved = match call_result {
                                    None => {
                                        let msg = tc.exception()
                                            .and_then(|e| e.to_string(&tc))
                                            .map(|s| s.to_rust_string_lossy(&tc))
                                            .unwrap_or_else(|| "unknown JS exception".to_string());
                                        return Err(anyhow!("JS exception: {}", msg));
                                    }
                                    Some(v) if v.is_promise() => {
                                        let promise = v.cast::<v8::Promise>();
                                        let platform = v8::V8::get_current_platform();
                                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                                        loop {
                                            for _ in 0..5 {
                                                // SAFETY: pump_message_loop requires &Isolate.
                                                // iso_ptr is valid and pinned to this thread.
                                                let iso: &v8::Isolate = unsafe { &*iso_ptr };
                                                v8::Platform::pump_message_loop(&platform, iso, false);
                                            }
                                            tc.perform_microtask_checkpoint();
                                            match promise.state() {
                                                v8::PromiseState::Fulfilled => break promise.result(&tc),
                                                v8::PromiseState::Rejected => {
                                                    let err = promise.result(&tc);
                                                    let msg = err.to_string(&tc)
                                                        .map(|s| s.to_rust_string_lossy(&tc))
                                                        .unwrap_or_else(|| "Promise rejected".to_string());
                                                    return Err(anyhow!("Promise rejected: {}", msg));
                                                }
                                                v8::PromiseState::Pending => {
                                                    if std::time::Instant::now() > deadline {
                                                        return Err(anyhow!("Async handler timed out after 30s"));
                                                    }
                                                    // Detect CPU timeout that fired while WASM or async code
                                                    // ran inside perform_microtask_checkpoint. TerminateExecution
                                                    // interrupts WASM JIT loops but does not automatically reject
                                                    // the outer async Promise — it stays Pending. Check both the
                                                    // timer flag and the TryCatch termination flag.
                                                    if crate::data_plane::is_cpu_termination_requested() || tc.has_terminated() {
                                                        return Err(anyhow!("CPU timeout"));
                                                    }
                                                    crate::runtime::apis::fire_pending_intervals(&mut *tc);
                                                    crate::runtime::apis::fire_pending_timeouts(&mut *tc);
                                                    std::thread::sleep(std::time::Duration::from_millis(1));
                                                }
                                            }
                                        }
                                    }
                                    Some(v) => v,
                                };

                                let obj = resolved.to_object(&tc)
                                    .ok_or_else(|| anyhow!("Handler response is not an object"))?;

                                let sk = v8::String::new(&tc, "status").ok_or_else(|| anyhow!("status key"))?;
                                let status = obj.get(&tc, sk.into())
                                    .and_then(|v| v.to_integer(&tc))
                                    .map(|i| i.value() as u16)
                                    .unwrap_or(200);

                                let mut response = crate::http::NanoResponse::with_status(status);

                                let h2k = v8::String::new(&tc, "headers").ok_or_else(|| anyhow!("headers key"))?;
                                if let Some(hval) = obj.get(&tc, h2k.into()) {
                                    if let Some(hobj) = hval.to_object(&tc) {
                                        let ik = v8::String::new(&tc, "__headers__").ok_or_else(|| anyhow!("__headers__ key"))?;
                                        let hsrc = hobj.get(&tc, ik.into())
                                            .and_then(|v| v.to_object(&tc))
                                            .unwrap_or(hobj);
                                        if let Some(names) = hsrc.get_own_property_names(&tc, Default::default()) {
                                            for i in 0..names.length() {
                                                if let Some(key) = names.get_index(&tc, i) {
                                                    if let Some(ks) = key.to_string(&tc) {
                                                        let k = ks.to_rust_string_lossy(&tc);
                                                        if k.starts_with("__") || matches!(k.as_str(), "set" | "get" | "forEach") {
                                                            continue;
                                                        }
                                                        if let Some(val) = hsrc.get(&tc, key.into()) {
                                                            if !val.is_function() {
                                                                if let Some(vs) = val.to_string(&tc) {
                                                                    response = response.with_header(&k, &vs.to_rust_string_lossy(&tc));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                let b2k = v8::String::new(&tc, "body").ok_or_else(|| anyhow!("body key"))?;
                                if let Some(bval) = obj.get(&tc, b2k.into()) {
                                    if !bval.is_null() && !bval.is_undefined() {
                                        if let Some(bs) = bval.to_string(&tc) {
                                            response = response.with_body(bs.to_rust_string_lossy(&tc));
                                        }
                                    }
                                }

                                Ok(response)
                            })();

                            let duration_ms = t0.elapsed().as_millis() as u64;
                            let status_code = match &result {
                                Ok(r) => r.status(),
                                Err(_) => 500,
                            };
                            tracing::info!(
                                request_id = %request_id,
                                worker_id = id,
                                isolate_id = %isolate_id,
                                status = status_code,
                                duration_ms = duration_ms,
                                "Worker {} request {} → {} in {}ms",
                                id, request_id, status_code, duration_ms
                            );

                            let result = result.map(|mut r| {
                                r.set_worker_id(id);
                                r.set_isolate_id(isolate_id.clone());
                                r
                            });
                            let _ = task.response_tx.send(result);
                            served += 1;
                        }
                        // ctx_scope + scope drop here
                    }
                    // nano drops here

                    info!("Worker {}: isolate recycled, creating fresh", id);
                } // 'isolate loop

                info!("Worker {} exiting", id);
            });

            workers.push(WorkerHandle {
                id,
                thread: Some(thread),
                task_tx,
            });
        }

        WorkerPool {
            workers,
            worker_count,
            hostname,
            next_worker: AtomicU32::new(0),
            vfs_backend,
            memory_limit_mb,
            env_vars,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoHeaders, NanoRequest, NanoUrl};
    use crate::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};
    use crate::worker::HandlerTask;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;
    use tokio::sync::oneshot;

    fn init_platform() {
        if !crate::v8::is_initialized() {
            crate::v8::initialize_platform().expect("Failed to initialize V8 platform");
        }
    }

    fn make_vfs() -> IsolateVfs {
        IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            VfsBackendEnum::memory(MemoryBackend::default()),
        )
    }

    #[test]
    fn test_compile_classic_handler_named_fetch() {
        init_platform();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope_pin = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let mut scope = scope_pin.init();
        let context = v8::Context::new(&scope, Default::default());
        let mut ctx_scope = v8::ContextScope::new(&mut scope, context);
        let result = compile_classic_handler(
            &mut ctx_scope, "/handler.js",
            "function fetch(req) { return { status: 200 }; }",
            context,
            "test::/handler.js",
        );
        assert!(result.is_ok(), "named fetch should compile: {:?}", result);
    }

    #[test]
    fn test_compile_classic_handler_no_fetch_errors() {
        init_platform();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope_pin = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let mut scope = scope_pin.init();
        let context = v8::Context::new(&scope, Default::default());
        let mut ctx_scope = v8::ContextScope::new(&mut scope, context);
        let result = compile_classic_handler(&mut ctx_scope, "/handler.js", "var x = 1;", context, "test::/handler.js");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No fetch handler"));
    }

    #[test]
    fn test_compile_esm_handler_named_export() {
        init_platform();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope_pin = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let mut scope = scope_pin.init();
        let context = v8::Context::new(&scope, Default::default());
        let mut ctx_scope = v8::ContextScope::new(&mut scope, context);
        let result = compile_esm_handler(
            &mut ctx_scope, "/handler.js",
            "export function fetch(req) { return { status: 200 }; }",
            make_vfs(),
        );
        assert!(result.is_ok(), "named ESM fetch should compile: {:?}", result);
    }

    #[test]
    fn test_compile_esm_handler_default_export() {
        init_platform();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope_pin = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let mut scope = scope_pin.init();
        let context = v8::Context::new(&scope, Default::default());
        let mut ctx_scope = v8::ContextScope::new(&mut scope, context);
        let result = compile_esm_handler(
            &mut ctx_scope, "/handler.js",
            "export default { fetch(req) { return { status: 200 }; } }",
            make_vfs(),
        );
        assert!(result.is_ok(), "default ESM export should compile: {:?}", result);
    }

    #[test]
    fn test_compile_esm_handler_no_fetch_errors() {
        init_platform();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope_pin = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let mut scope = scope_pin.init();
        let context = v8::Context::new(&scope, Default::default());
        let mut ctx_scope = v8::ContextScope::new(&mut scope, context);
        let result = compile_esm_handler(
            &mut ctx_scope, "/handler.js",
            "export const x = 1;",
            make_vfs(),
        );
        assert!(result.is_err(), "ESM without fetch should fail");
    }

    // Accesses private `next_worker` field — cannot move to tests/.
    #[test]
    fn test_round_robin_dispatch() {
        init_platform();
        let pool = WorkerPool::new("test.example.com".to_string(), 3, 0);

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let js_path = temp_dir.path().join("test.js");
        let mut f = fs::File::create(&js_path).expect("Failed to create test file");
        f.write_all(b"function fetch(request) { return { status: 200, headers: {}, body: \"\" }; }")
            .expect("Failed to write test code");
        let entrypoint = js_path.to_string_lossy().to_string();

        assert_eq!(pool.next_worker.load(Ordering::SeqCst), 0);

        for _ in 0..6 {
            let url = NanoUrl::parse("http://test/").unwrap();
            let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);
            let (tx, rx) = oneshot::channel();
            pool.dispatch(HandlerTask::new(entrypoint.clone(), request, tx)).expect("Dispatch failed");
            let _ = rx.blocking_recv();
        }

        assert_eq!(pool.next_worker.load(Ordering::SeqCst), 6, "counter must advance once per dispatch");

        pool.shutdown().expect("Shutdown failed");
    }
}
