//! Direct WASM test - verifies bytes integrity through VFS to WebAssembly.compile()
//!
//! This test creates a minimal valid WASM binary, writes it to VFS,
//! reads it back, and attempts WebAssembly compilation.

use nano::v8::{initialize_platform, NanoIsolate};
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};
use std::sync::Arc;

/// Valid WASM binary that exports an "add" function
/// Wat: (module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))
const WASM_BYTES: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, // magic: \0asm
    0x01, 0x00, 0x00, 0x00, // version: 1
    // Type section
    0x01, 0x07, 0x01, // section id 1, size 7, 1 type
    0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f, // func type: (i32, i32) -> i32
    // Function section
    0x03, 0x02, 0x01, 0x00, // section id 3, size 2, 1 func, type 0
    // Export section
    0x07, 0x07, 0x01, // section id 7, size 7, 1 export
    0x03, 0x61, 0x64, 0x64, // name: "add" (3 bytes)
    0x00, // export kind function
    0x00, // function index 0
    // Code section
    0x0a, 0x09, 0x01, // section id 10, size 9, 1 function
    0x07, 0x00, // body size 7, 0 locals
    0x20, 0x00, // local.get 0
    0x20, 0x01, // local.get 1
    0x6a, // i32.add
    0x0b, // end
];

#[test]
fn test_wasm_bytes_validity() {
    // Verify our WASM bytes are valid by checking magic number
    assert_eq!(&WASM_BYTES[0..4], &[0x00, 0x61, 0x73, 0x6d]);
    assert_eq!(WASM_BYTES[4], 0x01);

    // Verify length (actual bytes)
    println!("✓ WASM bytes valid: {} bytes", WASM_BYTES.len());
}

#[test]
fn test_wasm_compile_direct() {
    let _ = initialize_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let isolate_ptr = isolate.isolate();

    // Create VFS and write WASM file
    let vfs = IsolateVfs::new(
        VfsNamespace::from_hostname("test"),
        VfsBackendEnum::memory(MemoryBackend::default()),
    );

    // Write WASM bytes to VFS
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/add.wasm", WASM_BYTES)
            .await
            .expect("Failed to write WASM to VFS");
    });

    // Set up VFS context
    let vfs_ref = Arc::new(vfs);
    let vfs_ref_clone = vfs_ref.clone();
    nano::runtime::vfs_bindings::set_current_vfs(Some(vfs_ref));

    // Create V8 scope
    let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate_ptr));
    let mut scope = scope_storage.init();
    let context = v8::Context::new(&mut scope, Default::default());
    let mut ctx_scope = v8::ContextScope::new(&mut scope, context);

    // Read file via VFS
    let file_bytes = rt.block_on(async {
        vfs_ref_clone
            .read("/add.wasm")
            .await
            .expect("Failed to read WASM from VFS")
    });

    println!("✓ Read {} bytes from VFS", file_bytes.len());

    // Verify bytes match
    assert_eq!(file_bytes.len(), WASM_BYTES.len());
    assert_eq!(file_bytes.as_ref() as &[u8], WASM_BYTES);
    println!("✓ Bytes match original");

    // Create Uint8Array in V8
    let ab = v8::ArrayBuffer::new(&mut ctx_scope, file_bytes.len());
    let store = ab.get_backing_store();
    for (i, byte) in file_bytes.iter().enumerate() {
        if let Some(cell) = store.get(i) {
            cell.set(*byte);
        }
    }

    let uint8array = v8::Uint8Array::new(&mut ctx_scope, ab, 0, file_bytes.len())
        .expect("Failed to create Uint8Array");

    println!("✓ Created Uint8Array with {} bytes", file_bytes.len());

    // DEBUG: Print hex dump of bytes in the Uint8Array
    println!("DEBUG: First 50 bytes in hex:");
    let bytes_hex: Vec<String> = file_bytes
        .iter()
        .take(50)
        .map(|b| format!("{:02x}", b))
        .collect();
    println!("{}", bytes_hex.join(" "));

    // Get WebAssembly object
    let global = context.global(&mut ctx_scope);
    let wasm_key = v8::String::new(&mut ctx_scope, "WebAssembly").unwrap();
    let wasm_val = global
        .get(&mut ctx_scope, wasm_key.into())
        .expect("WebAssembly not found");
    let wasm_obj = wasm_val.to_object(&mut ctx_scope).unwrap();

    // Get WebAssembly.compile
    let compile_key = v8::String::new(&mut ctx_scope, "compile").unwrap();
    let compile_fn = wasm_obj
        .get(&mut ctx_scope, compile_key.into())
        .expect("WebAssembly.compile not found")
        .cast::<v8::Function>();

    println!("✓ Got WebAssembly.compile function");

    // Try to compile - this is where the failure happens
    let result = compile_fn.call(&mut ctx_scope, wasm_val.into(), &[uint8array.into()]);

    match result {
        Some(val) => {
            if val.is_promise() {
                println!("✓ WebAssembly.compile returned a Promise");
                let promise = val.cast::<v8::Promise>();

                // Pump message loop to resolve promise
                let platform = v8::V8::get_current_platform();
                for _ in 0..10 {
                    let isolate_ref: &v8::Isolate = &*ctx_scope;
                    v8::Platform::pump_message_loop(&platform, isolate_ref, false);
                    ctx_scope.perform_microtask_checkpoint();

                    match promise.state() {
                        v8::PromiseState::Fulfilled => {
                            println!("✓ Promise resolved successfully!");
                            let module = promise.result(&mut ctx_scope);
                            assert!(!module.is_undefined());
                            println!("✓ WASM module compiled successfully");
                            return;
                        }
                        v8::PromiseState::Rejected => {
                            let error = promise.result(&mut ctx_scope);
                            let error_str = error
                                .to_string(&mut ctx_scope)
                                .map(|s| s.to_rust_string_lossy(&mut ctx_scope))
                                .unwrap_or_else(|| "Unknown error".to_string());
                            panic!("✗ WebAssembly.compile rejected: {}", error_str);
                        }
                        v8::PromiseState::Pending => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
                panic!("✗ Promise did not resolve within timeout");
            } else {
                panic!("✗ WebAssembly.compile did not return a Promise");
            }
        }
        None => {
            panic!("✗ WebAssembly.compile returned None (exception thrown)");
        }
    }
}
