use std::time::{Duration, Instant};

/// Maximum time to wait for a Promise to resolve (30 seconds)
/// This prevents infinite loops on stuck Promises.
const MAX_PROMISE_WAIT_TIME: Duration = Duration::from_secs(30);

/// Resolve a V8 Promise by pumping the async event loop
///
/// This function resolves Promises by pumping the V8 message loop, allowing
/// async operations like WebAssembly.compile/instantiate and async JavaScript
/// handlers to complete.
///
/// # V147 API Note
/// In v147, we accept ContextScope directly which callers have.
/// ContextScope derefs to the inner HandleScope which we use for API calls.
///
/// # Arguments
///
/// * `scope` - The V8 ContextScope
/// * `promise` - The V8 Promise to resolve
///
/// # Returns
///
/// * `Ok(v8::Local<v8::Value>)` - The resolved value if Fulfilled
/// * `Err(anyhow::Error)` - If Rejected or timed out
///
/// # Example
///
/// ```rust
/// // v147 API pattern
/// let ctx_scope = v8::ContextScope::new(&mut scope, context);
/// let mut ctx_scope = ctx_scope.init();
///
/// // Execute JS that returns a Promise
/// let result = script.run(&mut ctx_scope).unwrap();
///
/// // If it's a Promise, resolve it
/// if result.is_promise() {
///     let promise = result.cast::<v8::Promise>();
///     match resolve_promise_with_async(&mut ctx_scope, promise) {
///         Ok(value) => { /* use resolved value */ }
///         Err(e) => { /* handle error */ }
///     }
/// }
/// ```
pub fn resolve_promise_with_async<'a>(
    scope: &mut v8::ContextScope<'a, 'a, v8::HandleScope<'a, v8::Context>>,
    promise: v8::Local<'a, v8::Promise>,
) -> anyhow::Result<v8::Local<'a, v8::Value>> {
    let start_time = Instant::now();
    let mut iteration_count = 0u32;

    // Loop until promise resolves or times out
    loop {
        // Check for timeout first
        if start_time.elapsed() > MAX_PROMISE_WAIT_TIME {
            return Err(anyhow::anyhow!(
                "Promise resolution timeout after {:?}",
                MAX_PROMISE_WAIT_TIME
            ));
        }

        // CRITICAL FIX: Pump the V8 message loop BEFORE checking promise state
        // WebAssembly.compile/instantiate and other V8 internal async operations
        // require the message loop to be pumped to make progress.
        // This must happen BEFORE checking promise.state() to give V8 a chance
        // to process pending compilation jobs.
        //
        // WASM compilation can require multiple message loop pumps, so we pump
        // multiple times to ensure the compilation completes.
        let platform = v8::V8::get_current_platform();
        for _ in 0..5 {
            // v147 API: pump_message_loop expects &Isolate
            // ContextScope derefs to HandleScope<Context>, which derefs to Isolate
            let isolate: &v8::Isolate = &**scope;
            v8::Platform::pump_message_loop(&platform, isolate, false);
        }

        // Perform microtask checkpoint to execute queued JavaScript microtasks
        // This handles Promise callbacks and other async JavaScript operations
        // v147 API: perform_microtask_checkpoint on ContextScope through DerefMut
        scope.perform_microtask_checkpoint();

        match promise.state() {
            v8::PromiseState::Fulfilled => {
                return Ok(promise.result(scope));
            }
            v8::PromiseState::Rejected => {
                let error = promise.result(scope);
                let error_str = error
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "Promise rejected".to_string());
                return Err(anyhow::anyhow!("Promise rejected: {}", error_str));
            }
            v8::PromiseState::Pending => {
                // Promise is still pending, continue pumping
                iteration_count += 1;

                // Yield to avoid blocking other operations
                // This is important for high-density scenarios with many isolates
                if iteration_count % 10 == 0 {
                    std::thread::yield_now();
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

///
/// Same as `resolve_promise_with_async`, but with shorter timeout
pub fn resolve_promise_with_timeout<'a>(
    scope: &mut v8::ContextScope<'a, 'a, v8::HandleScope<'a, v8::Context>>,
    promise: v8::Local<'a, v8::Promise>,
    timeout_ms: u64,
) -> anyhow::Result<v8::Local<'a, v8::Value>> {
    let start_time = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    let mut iteration_count = 0u32;

    loop {
        // Check for timeout
        if start_time.elapsed() > timeout {
            return Err(anyhow::anyhow!(
                "Promise resolution timeout after {:?}",
                timeout
            ));
        }

        // CRITICAL FIX: Pump the V8 message loop BEFORE checking promise state
        // This allows WebAssembly.compile/instantiate and other internal V8
        // async operations to make progress before we check if they're done.
        // WASM compilation can require multiple pumps, so we pump multiple times.
        let platform = v8::V8::get_current_platform();
        for _ in 0..5 {
            // v147 API: pump_message_loop expects &Isolate
            // ContextScope derefs to HandleScope<Context>, which derefs to Isolate
            let isolate: &v8::Isolate = &**scope;
            v8::Platform::pump_message_loop(&platform, isolate, false);
        }

        // v147 API: perform_microtask_checkpoint on ContextScope through DerefMut
        scope.perform_microtask_checkpoint();

        match promise.state() {
            v8::PromiseState::Fulfilled => {
                return Ok(promise.result(scope));
            }
            v8::PromiseState::Rejected => {
                let error = promise.result(scope);
                let error_str = error
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "Promise rejected".to_string());
                return Err(anyhow::anyhow!("Promise rejected: {}", error_str));
            }
            v8::PromiseState::Pending => {
                iteration_count += 1;

                if iteration_count % 10 == 0 {
                    std::thread::yield_now();
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

/// Check if a value is a Promise and resolve it if so
///
/// This is a convenience function that checks if a value is a Promise
/// and resolves it using the async event loop. If the value is not a Promise,
/// it's returned as-is.
///
/// # V147 API Note
/// In v147, we accept ContextScope directly which callers have.
///
/// # Arguments
///
/// * `scope` - The V8 ContextScope
/// * `value` - The value to check/resolve
///
/// # Returns
///
/// * `Ok(v8::Local<v8::Value>)` - The value (resolved if it was a Promise)
/// * `Err(anyhow::Error)` - If the Promise was rejected or timed out
pub fn resolve_if_promise<'a>(
    scope: &mut v8::ContextScope<'a, 'a, v8::HandleScope<'a, v8::Context>>,
    value: v8::Local<'a, v8::Value>,
) -> anyhow::Result<v8::Local<'a, v8::Value>> {
    if value.is_promise() {
        let promise = value.cast::<v8::Promise>();
        resolve_promise_with_async(scope, promise)
    } else {
        Ok(value)
    }
}
