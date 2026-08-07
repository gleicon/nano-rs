//! Performance measurements: isolate creation, handler compilation, request latency.
//!
//! Measures each phase of the request pipeline:
//! - Isolate creation time (per-recycle cost)
//! - Script compile + first-run (once per entrypoint per isolate)
//! - Handler call via cached Global<Function> (the hot path)
//! - WorkerPool end-to-end latency (includes channel overhead)
//!
//! Target: <10ms per request steady-state (handler already cached).

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::v8::{initialize_platform, NanoIsolate};
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};
use nano::worker::pool::WorkerPool;
use nano::worker::HandlerTask;

use std::sync::Arc;
use std::time::{Duration, Instant};

fn init_v8() {
    let _ = initialize_platform();
}

fn make_vfs() -> IsolateVfs {
    IsolateVfs::new(
        VfsNamespace::from_hostname("perf.test"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    )
}

fn make_backend() -> VfsBackendEnum {
    VfsBackendEnum::Memory(Arc::new(MemoryBackend::default()))
}

fn make_get_request() -> NanoRequest {
    let url = NanoUrl::parse("http://perf.test/").unwrap();
    NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None)
}

fn write_js(code: &str) -> String {
    let dir = std::env::temp_dir().join("nano_perf_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("handler.js");
    std::fs::write(&path, code).unwrap();
    path.to_string_lossy().to_string()
}

// ─── Phase 1: Isolate creation cost ──────────────────────────────────────────

#[test]
fn perf_isolate_creation_10x() {
    init_v8();
    let mut times = Vec::with_capacity(10);
    for _ in 0..10 {
        let t = Instant::now();
        let _iso = NanoIsolate::new_with_vfs(make_vfs()).expect("isolate failed");
        times.push(t.elapsed());
    }
    let avg = times.iter().sum::<Duration>() / times.len() as u32;
    let min = *times.iter().min().unwrap();
    let max = *times.iter().max().unwrap();
    println!(
        "\n[PERF] isolate_creation (10x): avg={:.1}ms  min={:.1}ms  max={:.1}ms",
        avg.as_secs_f64() * 1e3,
        min.as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3
    );
    // Just report — creation is one-time on recycle, not per-request
}

// ─── Phase 2: Script compile + first-run in persistent scope ─────────────────

#[test]
fn perf_script_compile_in_persistent_scope() {
    init_v8();
    let code = r#"function __nano_user_fetch(r){ return {status:200,headers:{},body:"ok"}; }"#;

    let mut nano = NanoIsolate::new_with_vfs(make_vfs()).expect("isolate failed");
    let scope_pin = std::pin::pin!(v8::HandleScope::new(nano.isolate()));
    let mut scope = scope_pin.init();
    let context = v8::Context::new(&scope, Default::default());
    nano::runtime::apis::RuntimeAPIs::bind_all(&mut scope, context);
    let mut ctx_scope = v8::ContextScope::new(&mut scope, context);

    let t_compile = Instant::now();
    let code_v8 = v8::String::new(&mut ctx_scope, code).unwrap();
    let script = v8::Script::compile(&ctx_scope, code_v8, None).expect("compile failed");
    let compile_ms = t_compile.elapsed().as_secs_f64() * 1e3;

    let t_run = Instant::now();
    script.run(&ctx_scope).expect("run failed");
    let run_ms = t_run.elapsed().as_secs_f64() * 1e3;

    println!(
        "\n[PERF] script_phases: compile={:.2}ms  first_run={:.2}ms",
        compile_ms, run_ms
    );
}

// ─── Phase 3: Handler call via cached Global<Function> — the hot path ─────────

#[test]
fn perf_handler_call_100x() {
    init_v8();
    let code = r#"function __nano_user_fetch(r){ return {status:200,headers:{},body:"ok"}; }"#;

    let mut nano = NanoIsolate::new_with_vfs(make_vfs()).expect("isolate failed");
    let scope_pin = std::pin::pin!(v8::HandleScope::new(nano.isolate()));
    let mut scope = scope_pin.init();
    let context = v8::Context::new(&scope, Default::default());
    nano::runtime::apis::RuntimeAPIs::bind_all(&mut scope, context);
    let mut ctx_scope = v8::ContextScope::new(&mut scope, context);

    // Compile once
    let code_v8 = v8::String::new(&mut ctx_scope, code).unwrap();
    v8::Script::compile(&ctx_scope, code_v8, None)
        .unwrap()
        .run(&ctx_scope)
        .unwrap();

    // Cache handler as Global<Function>
    let global_obj = context.global(&mut ctx_scope);
    let key = v8::String::new(&mut ctx_scope, "__nano_user_fetch").unwrap();
    let handler_val = global_obj.get(&mut ctx_scope, key.into()).unwrap();
    assert!(handler_val.is_function(), "handler must be function");
    let handler_g = v8::Global::new(&**ctx_scope, handler_val.cast::<v8::Function>());

    // Dummy arg
    let dummy = v8::String::new(&mut ctx_scope, "req").unwrap();

    // Warm up 5
    for _ in 0..5 {
        let h = v8::Local::new(&mut ctx_scope, &handler_g);
        let recv = context.global(&mut ctx_scope);
        let result = h.call(&mut ctx_scope, recv.into(), &[dummy.into()]);
        assert!(
            result.is_some(),
            "warm-up call returned None (JS exception)"
        );
    }

    // Measure 100 calls
    let mut times = Vec::with_capacity(100);
    for _ in 0..100 {
        let t = Instant::now();
        let h = v8::Local::new(&mut ctx_scope, &handler_g);
        let recv = context.global(&mut ctx_scope);
        let result = h.call(&mut ctx_scope, recv.into(), &[dummy.into()]);
        times.push(t.elapsed());
        assert!(result.is_some(), "handler call returned None");
    }

    let avg = times.iter().sum::<Duration>() / times.len() as u32;
    let min = *times.iter().min().unwrap();
    let mut sorted = times.clone();
    sorted.sort();
    let p95 = sorted[94];
    let max = *times.iter().max().unwrap();

    println!("\n[PERF] handler_call/Global<Function> (100x): avg={:.3}ms  min={:.3}ms  p95={:.3}ms  max={:.3}ms",
        avg.as_secs_f64()*1e3, min.as_secs_f64()*1e3,
        p95.as_secs_f64()*1e3, max.as_secs_f64()*1e3);

    assert!(
        avg.as_millis() < 10,
        "Hot-path handler call too slow: avg={:?} (target <10ms)",
        avg
    );
}

// ─── Phase 4: WorkerPool end-to-end latency (includes channel + JS response) ──

#[test]
fn perf_workerpool_e2e_50x() {
    init_v8();
    nano::data_plane::init_code_cache();

    // Sync (non-async) handler — avoids Promise resolution overhead
    let entrypoint = write_js(
        r#"
export default {
    fetch(request) {
        return new Response("hello", { status: 200 });
    }
};
"#,
    );

    let pool = WorkerPool::with_backend("perf.test".to_string(), 2, 0, make_backend());

    // Warmup: first 3 requests compile the handler — will be slow
    for i in 0..3 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = HandlerTask::new(entrypoint.clone(), make_get_request(), tx);
        pool.dispatch(task).unwrap();
        let resp = rx.blocking_recv().expect("timeout on warmup");
        assert!(resp.is_ok(), "warmup {} failed: {:?}", i, resp.err());
        assert_eq!(resp.unwrap().status(), 200, "warmup {} wrong status", i);
    }

    // Measure 50 requests
    let mut times = Vec::with_capacity(50);
    for i in 0..50 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = HandlerTask::new(entrypoint.clone(), make_get_request(), tx);
        let t = Instant::now();
        pool.dispatch(task).unwrap();
        let resp = rx.blocking_recv().expect(&format!("req {} timeout", i));
        let elapsed = t.elapsed();

        // Validate response
        assert!(resp.is_ok(), "req {} error: {:?}", i, resp.err());
        let r = resp.unwrap();
        assert_eq!(r.status(), 200, "req {} wrong status: {}", i, r.status());
        // Body must be non-empty string "hello"
        // Body bytes must contain "hello"
        if let Some(b) = r.body() {
            assert!(b.windows(5).any(|w| w == b"hello"), "req {} wrong body", i);
        }

        times.push(elapsed);
    }

    let avg = times.iter().sum::<Duration>() / times.len() as u32;
    let min = *times.iter().min().unwrap();
    let mut sorted = times.clone();
    sorted.sort();
    let p50 = sorted[24];
    let p95 = sorted[47];
    let max = *times.iter().max().unwrap();

    println!("\n[PERF] WorkerPool e2e (50 requests, 2 workers, handler cached):");
    println!(
        "  avg={:.2}ms  min={:.2}ms  p50={:.2}ms  p95={:.2}ms  max={:.2}ms",
        avg.as_secs_f64() * 1e3,
        min.as_secs_f64() * 1e3,
        p50.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3
    );

    // Steady-state: skip first 3 (compilation) already done in warmup
    let ss_avg = times.iter().sum::<Duration>() / times.len() as u32;
    println!(
        "  steady-state avg: {:.2}ms  [target: <10ms]",
        ss_avg.as_secs_f64() * 1e3
    );
}
