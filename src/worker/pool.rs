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

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use tracing::{debug, error, info, warn};

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
    vfs_backend: crate::vfs::VfsBackendEnum,
    memory_limit_mb: u32,
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

        let mut workers = Vec::with_capacity(worker_count as usize);

        for id in 0..worker_count {
            let worker_hostname = hostname_for_workers.clone();
            let worker_vfs_backend = vfs_backend_for_workers.clone();
            let worker_source = source_for_workers.clone();
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

                // Extract temp entrypoint override for sliver mode (if any)
                let temp_entrypoint_override: Option<std::path::PathBuf> = match &worker_source {
                    AppSource::Sliver { temp_entrypoint, .. } => temp_entrypoint.clone(),
                    _ => None,
                };

                // Outer loop: one iteration per isolate lifetime.
                'isolate: loop {
                    let namespace = VfsNamespace::from_hostname(&worker_hostname);
                    let vfs = IsolateVfs::new(namespace, worker_vfs_backend.clone());

                    // First isolate: warm-start from snapshot (sliver) or fresh (entrypoint).
                    // Recycled isolates: always fresh.
                    let mut nano = if first_isolate {
                        first_isolate = false;
                        match &worker_source {
                            AppSource::Entrypoint { .. } => {
                                match NanoIsolate::new_with_vfs(vfs) {
                                    Ok(iso) => iso,
                                    Err(e) => { error!("Worker {}: isolate failed: {}", id, e); return; }
                                }
                            }
                            AppSource::Sliver { data, .. } => {
                                if let Err(e) = rt.block_on(data.restore_to_vfs(&vfs)) {
                                    warn!("Worker {}: VFS restore failed: {}", id, e);
                                } else {
                                    debug!("Worker {}: restored {} VFS entries", id, data.vfs_entries.len());
                                }
                                match NanoIsolate::from_snapshot(&data.heap_data, vfs.clone()) {
                                    Ok(iso) => { info!("Worker {}: restored from snapshot", id); iso }
                                    Err(e) => {
                                        warn!("Worker {}: snapshot restore failed ({}), creating fresh", id, e);
                                        match NanoIsolate::new_with_vfs(vfs) {
                                            Ok(iso) => iso,
                                            Err(e) => { error!("Worker {}: isolate failed: {}", id, e); return; }
                                        }
                                    }
                                }
                            }
                            AppSource::Static { .. } => {
                                error!("Worker {}: Static source in unified worker — should not happen", id);
                                return;
                            }
                        }
                    } else {
                        match NanoIsolate::new_with_vfs(vfs) {
                            Ok(iso) => iso,
                            Err(e) => { error!("Worker {}: isolate create failed: {}", id, e); return; }
                        }
                    };

                    if memory_limit_mb > 0 {
                        let bytes = memory_limit_mb as usize * 1024 * 1024;
                        nano.set_heap_limits(bytes / 2, bytes);
                    }

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

                            let task = match task_rx.recv() {
                                Ok(t) => t,
                                Err(_) => { debug!("Worker {}: channel closed", id); break 'isolate; }
                            };

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

                            // Determine entrypoint (sliver may override via temp file)
                            let entrypoint = temp_entrypoint_override
                                .as_ref()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|| task.entrypoint.clone());

                            // Compile + cache handler (once per entrypoint, per isolate lifetime)
                            if !handler_cache.contains_key(&entrypoint) {
                                let code = match crate::data_plane::read_code_cached(&entrypoint) {
                                    Ok(c) => c,
                                    Err(e) => {
                                        let _ = task.response_tx.send(Err(e));
                                        continue 'requests;
                                    }
                                };
                                let transformed = if crate::v8::module::is_esm_module(&code) {
                                    crate::v8::module::transform_module_code(&code)
                                } else {
                                    code.to_string()
                                };

                                let code_v8 = match v8::String::new(&mut ctx_scope, &transformed) {
                                    Some(s) => s,
                                    None => {
                                        let _ = task.response_tx.send(Err(anyhow!("V8 string alloc failed")));
                                        continue 'requests;
                                    }
                                };
                                let script = match v8::Script::compile(&ctx_scope, code_v8, None) {
                                    Some(s) => s,
                                    None => {
                                        let _ = task.response_tx.send(Err(anyhow!("Script compile failed for '{}\'", entrypoint)));
                                        continue 'requests;
                                    }
                                };
                                if script.run(&ctx_scope).is_none() {
                                    let _ = task.response_tx.send(Err(anyhow!("Script execution failed for '{}'", entrypoint)));
                                    continue 'requests;
                                }

                                let global_obj = context.global(&mut ctx_scope);
                                let nano_k = match v8::String::new(&mut ctx_scope, "__nano_user_fetch") {
                                    Some(s) => s,
                                    None => { let _ = task.response_tx.send(Err(anyhow!("V8 OOM allocating key"))); continue 'requests; }
                                };
                                let fetch_k = match v8::String::new(&mut ctx_scope, "fetch") {
                                    Some(s) => s,
                                    None => { let _ = task.response_tx.send(Err(anyhow!("V8 OOM allocating key"))); continue 'requests; }
                                };
                                let handler_val = global_obj.get(&mut ctx_scope, nano_k.into())
                                    .filter(|v| v.is_function())
                                    .or_else(|| global_obj.get(&mut ctx_scope, fetch_k.into()).filter(|v| v.is_function()));

                                match handler_val {
                                    Some(f) => {
                                        let g = v8::Global::new(&**ctx_scope, f.cast::<v8::Function>());
                                        handler_cache.insert(entrypoint.clone(), g);
                                        info!("Worker {}: handler cached for '{}'", id, entrypoint);
                                    }
                                    None => {
                                        let _ = task.response_tx.send(Err(anyhow!(
                                            "No fetch handler found in '{}'. Export a 'fetch' function.",
                                            entrypoint
                                        )));
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
                                                    // SAFETY: iso_ptr valid for this worker's lifetime.
                                                    unsafe { (*iso_ptr).cancel_terminate_execution(); }
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
        }
    }
}

/// Worker pool for sliver-based (snapshot-restored) applications
///
/// This specialized worker pool creates isolates from V8 heap snapshots
/// rather than fresh isolates. It also restores VFS state from the sliver.
///
/// # Design
///
/// - Each worker restores its isolate from the snapshot blob
/// - VFS entries are restored before the worker accepts tasks
/// - Falls back to fresh isolate if snapshot restoration fails
/// - Shares the same dispatch interface as regular WorkerPool
///
/// # Deprecation Notice
///
/// This type is now a thin wrapper around `WorkerPool` for backward compatibility.
/// New code should use `WorkerPool::with_source()` directly with `AppSource::Sliver`.
pub struct SliverWorkerPool {
    /// Inner WorkerPool that handles all execution
    ///
    /// This wraps the unified WorkerPool created with AppSource::Sliver.
    inner: WorkerPool,
    /// Hostname this pool serves (cached for quick access)
    pub hostname: String,
    /// Number of workers (cached for quick access)
    pub worker_count: u32,
    /// Unpacked sliver data (kept for reference/debugging)
    unpacked_sliver: crate::sliver::UnpackedSliver,
    /// Optional temp entrypoint path (kept for reference/debugging)
    temp_entrypoint: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for SliverWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SliverWorkerPool")
            .field("worker_count", &self.worker_count)
            .field("hostname", &self.hostname)
            .field("unpacked_sliver", &self.unpacked_sliver.metadata.hostname)
            .field("temp_entrypoint", &self.temp_entrypoint)
            .finish()
    }
}

impl SliverWorkerPool {
    /// Create a new sliver worker pool with restored isolates
    ///
    /// This now delegates to the unified `WorkerPool::with_source()` constructor
    /// for consistent behavior across all pool types.
    ///
    /// # Arguments
    ///
    /// * `hostname` - Hostname this pool serves (for logging)
    /// * `worker_count` - Number of worker threads to spawn
    /// * `memory_limit_mb` - Memory limit per isolate in MB (0 = no limit)
    /// * `unpacked_sliver` - The unpacked sliver containing snapshot and VFS data
    ///
    /// # Returns
    ///
    /// A new SliverWorkerPool with N workers restored from snapshot
    ///
    /// # Deprecation
    ///
    /// This method now delegates to `WorkerPool::with_source()`. For new code,
    /// use `WorkerPool::with_source(hostname, worker_count, memory_limit_mb, AppSource::sliver(data))`.
    pub fn new(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        unpacked_sliver: crate::sliver::UnpackedSliver,
    ) -> Self {
        Self::with_temp_entrypoint(
            hostname,
            worker_count,
            memory_limit_mb,
            unpacked_sliver,
            None,
        )
    }

    /// Create a new sliver worker pool with a temp entrypoint path
    ///
    /// This variant is used when the sliver VFS has been extracted to a temp
    /// directory, and the JS entrypoint should be read from that location.
    ///
    /// # Deprecation
    ///
    /// This method now delegates to `WorkerPool::with_source()`. For new code,
    /// use `WorkerPool::with_source()` with `AppSource::sliver_with_temp(data, temp)`.
    pub fn with_temp_entrypoint(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        unpacked_sliver: crate::sliver::UnpackedSliver,
        temp_entrypoint: Option<std::path::PathBuf>,
    ) -> Self {
        use crate::worker::AppSource;
        use crate::vfs::MemoryBackend;

        let source = if let Some(temp) = temp_entrypoint.clone() {
            AppSource::sliver_with_temp(unpacked_sliver.clone(), temp)
        } else {
            AppSource::sliver(unpacked_sliver.clone())
        };

        let vfs_backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let inner = WorkerPool::with_source_and_backend(
            hostname.clone(),
            worker_count,
            memory_limit_mb,
            vfs_backend,
            source,
        );

        info!(
            "SliverWorkerPool for {} created with {} workers (delegates to unified WorkerPool)",
            hostname, worker_count
        );

        Self {
            inner,
            hostname: hostname.clone(),
            worker_count,
            unpacked_sliver,
            temp_entrypoint,
        }
    }

    /// Create a new sliver worker pool with a specific VFS backend
    ///
    /// # Deprecation
    ///
    /// This method now delegates to `WorkerPool::with_source_and_backend()`.
    pub fn with_backend(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        vfs_backend: crate::vfs::VfsBackendEnum,
        unpacked_sliver: crate::sliver::UnpackedSliver,
        temp_entrypoint: Option<std::path::PathBuf>,
    ) -> Self {
        use crate::worker::AppSource;

        let source = if let Some(temp) = temp_entrypoint.clone() {
            AppSource::sliver_with_temp(unpacked_sliver.clone(), temp)
        } else {
            AppSource::sliver(unpacked_sliver.clone())
        };

        let inner = WorkerPool::with_source_and_backend(
            hostname.clone(),
            worker_count,
            memory_limit_mb,
            vfs_backend,
            source,
        );

        info!(
            "SliverWorkerPool for {} created with {} workers (custom backend)",
            hostname, worker_count
        );

        Self {
            inner,
            hostname: hostname.clone(),
            worker_count,
            unpacked_sliver,
            temp_entrypoint,
        }
    }

    pub fn dispatch(&self, task: HandlerTask) -> Result<()> {
        self.inner.dispatch(task)
    }

    pub fn shutdown(self) -> Result<()> {
        info!("Shutting down SliverWorkerPool for {}", self.hostname);
        self.inner.shutdown()
    }

    /// Get the number of workers in this pool
    ///
    /// Provided for backward compatibility with code that accessed the field directly.
    pub fn worker_count(&self) -> u32 {
        self.worker_count
    }

    /// Get the hostname this pool serves
    ///
    /// Provided for backward compatibility with code that accessed the field directly.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Access the unpacked sliver data (for debugging/testing)
    pub fn sliver_data(&self) -> &crate::sliver::UnpackedSliver {
        &self.unpacked_sliver
    }

    /// Access the VFS backend (for testing VFS operations)
    pub fn vfs_backend(&self) -> &crate::vfs::VfsBackendEnum {
        &self.inner.vfs_backend
    }
}

impl crate::worker::r#trait::WorkerPool for SliverWorkerPool {
    fn dispatch(&self, task: HandlerTask) -> Result<()> {
        self.inner.dispatch(task)
    }

    fn shutdown(self) -> Result<()> {
        self.inner.shutdown()
    }

    fn worker_count(&self) -> u32 {
        self.worker_count
    }

    fn hostname(&self) -> &str {
        &self.hostname
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoHeaders, NanoRequest, NanoUrl};
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
