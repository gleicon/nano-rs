//! WASM Binary Flow Debugging Test
//!
//! This test traces a WASM binary through the entire system to identify
//! where corruption occurs (VFS → JS → WebAssembly API).
//!
//! V8 v147 API Note: Uses Box::pin + transmute pattern for scope initialization

use nano::v8::initialize_platform;
use nano::v8::NanoIsolate;
use nano::vfs::{DiskBackend, IsolateVfs, VfsBackend, VfsBackendEnum, VfsNamespace, VfsPath};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

/// Helper to format bytes as hex for debugging
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Helper to compare two byte arrays and show differences
fn compare_bytes(name1: &str, bytes1: &[u8], name2: &str, bytes2: &[u8]) {
    println!("\n=== Byte Comparison: {} vs {} ===", name1, name2);
    println!("{} length: {} bytes", name1, bytes1.len());
    println!("{} length: {} bytes", name2, bytes2.len());

    if bytes1.len() != bytes2.len() {
        println!("❌ LENGTH MISMATCH!");
    }

    let min_len = bytes1.len().min(bytes2.len());
    let mut diff_count = 0;

    for i in 0..min_len {
        if bytes1[i] != bytes2[i] {
            if diff_count < 10 {
                // Only show first 10 diffs
                println!(
                    "  Diff at byte {}: {}={:02x} vs {}={:02x}",
                    i, name1, bytes1[i], name2, bytes2[i]
                );
            }
            diff_count += 1;
        }
    }

    if diff_count > 0 {
        println!("  Total differences: {}", diff_count);
    } else if bytes1.len() == bytes2.len() {
        println!("✅ Bytes are identical!");
    }

    println!(
        "{} first 32 bytes: {}",
        name1,
        bytes_to_hex(&bytes1[..min_len.min(32)])
    );
    println!(
        "{} first 32 bytes: {}",
        name2,
        bytes_to_hex(&bytes2[..min_len.min(32)])
    );
}

/// Helper to execute code with V8 v147 scope pattern
fn with_nano_context<F, R>(isolate: &mut NanoIsolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    let isolate_ptr = isolate.isolate();
    v8::scope!(handle_scope, isolate_ptr);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}

/// Test 1: Verify WASM file on disk is valid
#[test]
fn test_1_disk_wasm_is_valid() {
    // Read the WASM file directly from disk
    let wasm_path = PathBuf::from("examples/wasm-test/add.wasm");
    let disk_bytes = fs::read(&wasm_path).expect("Failed to read WASM file");

    println!("\n=== Test 1: Disk WASM File ===");
    println!("File: {:?}", wasm_path);
    println!("Size: {} bytes", disk_bytes.len());
    println!(
        "First 16 bytes (hex): {}",
        bytes_to_hex(&disk_bytes[..16.min(disk_bytes.len())])
    );

    // Verify magic number
    assert_eq!(&disk_bytes[0..4], b"\0asm", "WASM magic number mismatch");
    assert_eq!(disk_bytes[4], 0x01, "WASM version mismatch");

    println!("✅ Disk file is valid WASM ({} bytes)", disk_bytes.len());
}

/// Test 2: Verify VFS disk backend reads file correctly
#[tokio::test]
async fn test_2_vfs_disk_backend_reads_correctly() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let backend = DiskBackend::new(temp_dir.path())
        .await
        .expect("Failed to create backend");

    // Read original file
    let original_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read original");

    // Write to VFS
    let path = VfsPath::new("test::add.wasm").expect("Invalid path");
    VfsBackend::write(&backend, &path, &original_bytes)
        .await
        .expect("Failed to write to VFS");

    // Read back from VFS
    let vfs_bytes = VfsBackend::read(&backend, &path)
        .await
        .expect("Failed to read from VFS");

    println!("\n=== Test 2: VFS Disk Backend ===");
    compare_bytes("Original", &original_bytes, "VFS Read", &vfs_bytes);

    assert_eq!(
        original_bytes.len(),
        vfs_bytes.len(),
        "VFS changed file size!"
    );
    assert_eq!(original_bytes, vfs_bytes, "VFS corrupted the file!");

    println!("✅ VFS disk backend preserves bytes correctly");
}

/// Test 3: Verify IsolateVfs preserves bytes
#[tokio::test]
async fn test_3_isolate_vfs_preserves_bytes() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let backend = DiskBackend::new(temp_dir.path())
        .await
        .expect("Failed to create backend");

    // Create IsolateVfs with Disk backend
    let namespace = VfsNamespace::from_hostname("wasm-test.example.com");
    let isolate_vfs = IsolateVfs::new(namespace, VfsBackendEnum::Disk(Arc::new(backend)));

    // Read original
    let original_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read original");

    // Write via IsolateVfs
    isolate_vfs
        .write("/add.wasm", &original_bytes)
        .await
        .expect("Failed to write via IsolateVfs");

    // Read back via IsolateVfs
    let isolate_bytes = isolate_vfs
        .read("/add.wasm")
        .await
        .expect("Failed to read via IsolateVfs");

    println!("\n=== Test 3: IsolateVfs Layer ===");
    compare_bytes("Original", &original_bytes, "IsolateVfs", &isolate_bytes);

    assert_eq!(
        original_bytes.len(),
        isolate_bytes.len(),
        "IsolateVfs changed file size!"
    );
    assert_eq!(
        original_bytes, isolate_bytes,
        "IsolateVfs corrupted the file!"
    );

    println!("✅ IsolateVfs preserves bytes correctly");
}

/// Test 4: Check what happens when bytes pass through V8 bindings
#[test]
fn test_4_v8_bindings_byte_preservation() {
    let _ = initialize_platform();

    // Read original WASM
    let original_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read WASM");

    // Create isolate
    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    with_nano_context(&mut isolate, |context_scope, _context| {
        // Create a Uint8Array from the bytes in V8
        let ab = v8::ArrayBuffer::new(context_scope, original_bytes.len());
        let store = ab.get_backing_store();
        for (i, byte) in original_bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(*byte);
            }
        }
        let uint8array = v8::Uint8Array::new(context_scope, ab, 0, original_bytes.len())
            .expect("Failed to create Uint8Array");

        // Read the bytes back out
        let mut read_back = Vec::with_capacity(original_bytes.len());
        for i in 0..original_bytes.len() {
            if let Some(val) = uint8array.get_index(context_scope, i as u32) {
                if let Some(num) = val.to_integer(context_scope) {
                    read_back.push(num.value() as u8);
                }
            }
        }

        println!("\n=== Test 4: V8 Uint8Array Round-trip ===");
        compare_bytes("Original", &original_bytes, "V8 Round-trip", &read_back);

        assert_eq!(
            original_bytes.len(),
            read_back.len(),
            "V8 changed byte count!"
        );
        assert_eq!(original_bytes, read_back, "V8 corrupted bytes!");

        println!("✅ V8 Uint8Array round-trip preserves bytes");
    });
}

/// Test 5: Check if the issue is with how we create Uint8Array in bindings (byte by byte)
#[test]
fn test_5_vfs_bindings_byte_creation_pattern() {
    let _ = initialize_platform();

    let original_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read WASM");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    with_nano_context(&mut isolate, |context_scope, _context| {
        // Simulate exactly what vfs_bindings.rs does
        let ab = v8::ArrayBuffer::new(context_scope, original_bytes.len());
        let store = ab.get_backing_store();
        for (i, byte) in original_bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(*byte);
            }
        }

        // Create Uint8Array the same way
        let uint8array = v8::Uint8Array::new(context_scope, ab, 0, original_bytes.len())
            .expect("Failed to create Uint8Array");

        // Read back the bytes the same way wasm_validate_callback does
        let len = uint8array.byte_length();
        let mut read_back = Vec::with_capacity(len);
        for i in 0..len {
            if let Some(val) = uint8array.get_index(context_scope, i as u32) {
                if let Some(num) = val.to_integer(context_scope) {
                    read_back.push(num.value() as u8);
                }
            }
        }

        println!("\n=== Test 5: VFS Bindings Byte Creation Pattern ===");
        println!("Original size: {}", original_bytes.len());
        println!("Read back size: {}", read_back.len());
        println!("ArrayBuffer size: {}", ab.byte_length());
        println!("Uint8Array size: {}", len);

        // Check if any bytes were lost during creation
        if len != original_bytes.len() {
            println!("❌ Size mismatch during Uint8Array creation!");
        }

        if read_back.len() != original_bytes.len() {
            println!("❌ Bytes lost during read-back!");
        }

        compare_bytes(
            "Original",
            &original_bytes,
            "Via bindings pattern",
            &read_back,
        );

        // Critical: Check if first 4 bytes are correct (magic number)
        println!("\nMagic number check:");
        println!(
            "Original: {:02x} {:02x} {:02x} {:02x}",
            original_bytes[0], original_bytes[1], original_bytes[2], original_bytes[3]
        );
        if read_back.len() >= 4 {
            println!(
                "Read back: {:02x} {:02x} {:02x} {:02x}",
                read_back[0], read_back[1], read_back[2], read_back[3]
            );
        }
    });
}

/// Test 6: Test reading bytes from V8 TypedArray directly (simulating WebAssembly.validate input)
#[test]
fn test_6_v8_typedarray_byte_extraction() {
    let _ = initialize_platform();

    let original_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read WASM");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    with_nano_context(&mut isolate, |context_scope, _context| {
        // Create Uint8Array
        let ab = v8::ArrayBuffer::new(context_scope, original_bytes.len());
        let store = ab.get_backing_store();
        for (i, byte) in original_bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(*byte);
            }
        }
        let uint8array = v8::Uint8Array::new(context_scope, ab, 0, original_bytes.len())
            .expect("Failed to create Uint8Array");

        // Now simulate exactly what wasm_validate_callback does in js_api.rs
        let buffer: v8::Local<v8::Value> = uint8array.into();

        // Extract bytes using the same logic as wasm_validate_callback
        let extracted_bytes = if buffer.is_array_buffer() {
            let ab = buffer.cast::<v8::ArrayBuffer>();
            let store = ab.get_backing_store();
            let len = ab.byte_length();
            let mut vec = Vec::with_capacity(len);
            for i in 0..len {
                if let Some(cell) = store.get(i) {
                    vec.push(cell.get());
                }
            }
            vec
        } else if buffer.is_uint8_array() {
            let ta = buffer.cast::<v8::Uint8Array>();
            let len = ta.byte_length();
            let mut vec = Vec::with_capacity(len);
            for i in 0..len {
                if let Some(val) = ta.get_index(context_scope, i as u32) {
                    if let Some(num) = val.to_integer(context_scope) {
                        vec.push(num.value() as u8);
                    }
                }
            }
            vec
        } else {
            vec![]
        };

        println!("\n=== Test 6: V8 TypedArray Byte Extraction (like WebAssembly.validate) ===");
        println!("Original length: {}", original_bytes.len());
        println!("Extracted length: {}", extracted_bytes.len());

        compare_bytes("Original", &original_bytes, "Extracted", &extracted_bytes);

        // The key question: does this extraction work correctly?
        if extracted_bytes.len() != original_bytes.len() {
            println!("❌ CRITICAL: Byte extraction lost data!");
            println!(
                "   Expected {} bytes, got {}",
                original_bytes.len(),
                extracted_bytes.len()
            );
        }

        // Check if magic number is preserved
        if extracted_bytes.len() >= 4 {
            if &extracted_bytes[0..4] == b"\0asm" {
                println!("✅ Magic number preserved");
            } else {
                println!("❌ Magic number CORRUPTED!");
                println!(
                    "   Got: {:02x} {:02x} {:02x} {:02x}",
                    extracted_bytes[0], extracted_bytes[1], extracted_bytes[2], extracted_bytes[3]
                );
            }
        }
    });
}

/// Test 7: Use V8's native WasmModuleObject::compile() API end-to-end.
///
/// Exercises the full native path: compile WASM bytes to a module via
/// `v8::WasmModuleObject::compile`, construct a `WebAssembly.Instance` from it
/// with `new_instance` (it's a constructor), and call the exported `add(5, 3)`,
/// asserting the result is 8. The `WebAssembly.compile()` JS API uses this same
/// native machinery internally.
#[test]
fn test_7_webassembly_compile() {
    let _ = initialize_platform();

    let wasm_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read WASM");
    println!("\n=== Test 7: WasmModuleObject::compile() Direct API Test ===");
    println!("WASM size: {} bytes", wasm_bytes.len());
    println!("First 16 bytes: {}", bytes_to_hex(&wasm_bytes[..16]));

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let isolate_ptr = isolate.isolate();

    // Create HandleScope using the scope! macro for PinScope.
    v8::scope!(handle_scope, isolate_ptr);
    let context = v8::Context::new(handle_scope, Default::default());

    // Create context scope early for WASM operations
    // WasmModuleObject::compile needs the context scope
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    println!("Calling v8::WasmModuleObject::compile()...");
    match v8::WasmModuleObject::compile(ctx_scope, &wasm_bytes) {
        Some(module_obj) => {
            println!("✅ Native WASM compilation succeeded!");
            println!("   Module object type: {:?}", module_obj.type_repr());

            // Get the compiled module for caching/serialization
            let compiled_module = module_obj.get_compiled_module();
            let wire_bytes = compiled_module.get_wire_bytes_ref();
            println!("   Compiled module wire bytes: {} bytes", wire_bytes.len());

            // ctx_scope is already created above - use it for JS operations
            // Get WebAssembly.Instance constructor
            let global = context.global(ctx_scope);
            let wasm_key = v8::String::new(ctx_scope, "WebAssembly").unwrap();
            let wasm_val = global
                .get(ctx_scope, wasm_key.into())
                .expect("WebAssembly not found");

            let wasm_global = wasm_val.to_object(ctx_scope).unwrap();
            let instance_key = v8::String::new(ctx_scope, "Instance").unwrap();
            let instance_ctor_val = wasm_global
                .get(ctx_scope, instance_key.into())
                .expect("WebAssembly.Instance not found");
            let instance_ctor = instance_ctor_val.cast::<v8::Function>();

            // Create empty imports object
            let imports_obj = v8::Object::new(ctx_scope);

            // WebAssembly.Instance is a constructor — it must be invoked with `new`,
            // so use new_instance(), not call(). Calling it as a plain function throws
            // "TypeError: WebAssembly.Instance must be invoked with 'new'".
            let instance_result =
                instance_ctor.new_instance(ctx_scope, &[module_obj.into(), imports_obj.into()]);

            match instance_result {
                Some(instance) => {
                    println!("✅ WebAssembly.Instance created via new_instance()");
                    let exports_key = v8::String::new(ctx_scope, "exports").unwrap();
                    if let Some(exports) = instance.get(ctx_scope, exports_key.into()) {
                        let exports_obj = exports.to_object(ctx_scope).unwrap();
                        let add_key = v8::String::new(ctx_scope, "add").unwrap();
                        if let Some(add_fn) = exports_obj.get(ctx_scope, add_key.into()) {
                            if add_fn.is_function() {
                                let add = add_fn.cast::<v8::Function>();
                                let five = v8::Integer::new(ctx_scope, 5);
                                let three = v8::Integer::new(ctx_scope, 3);
                                match add.call(
                                    ctx_scope,
                                    exports.into(),
                                    &[five.into(), three.into()],
                                ) {
                                    Some(result) => {
                                        let result_i32 = result
                                            .to_integer(ctx_scope)
                                            .map(|i| i.value() as i32)
                                            .unwrap_or(-1);
                                        println!("   add(5, 3) = {}", result_i32);
                                        assert_eq!(
                                            result_i32, 8,
                                            "WASM add function should return 8"
                                        );
                                        println!("✅ Full WASM execution works!");
                                    }
                                    None => {
                                        println!("❌ Failed to call add function");
                                        panic!("Failed to call add function");
                                    }
                                }
                            } else {
                                panic!("add export is not a function");
                            }
                        } else {
                            panic!("add export not found");
                        }
                    } else {
                        panic!("exports not found");
                    }
                }
                None => {
                    println!("❌ Failed to create WebAssembly.Instance");
                    panic!("Failed to create WebAssembly.Instance");
                }
            }
        }
        None => {
            println!("❌ Native WASM compilation failed (returned None)");
            panic!("WasmModuleObject::compile() failed - WASM bytes may be invalid or V8 WASM not enabled");
        }
    }
}

/// Test 8: Verify WebAssembly.compile() JS API works after native compile
#[test]
fn test_8_js_webassembly_api() {
    let _ = initialize_platform();

    let wasm_bytes = fs::read("examples/wasm-test/add.wasm").expect("Failed to read WASM");
    println!("\n=== Test 8: WebAssembly JS API Test ===");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    with_nano_context(&mut isolate, |context_scope, context| {
        // Test WebAssembly.validate()
        let global = context.global(context_scope);
        let wasm_key = v8::String::new(context_scope, "WebAssembly").unwrap();
        let wasm_val = global
            .get(context_scope, wasm_key.into())
            .expect("WebAssembly not found");

        if wasm_val.is_undefined() {
            panic!("WebAssembly is undefined!");
        }

        let wasm_obj = wasm_val
            .to_object(context_scope)
            .expect("WebAssembly is not an object");

        // Test WebAssembly.validate()
        let validate_key = v8::String::new(context_scope, "validate").unwrap();
        let validate_fn = wasm_obj
            .get(context_scope, validate_key.into())
            .expect("WebAssembly.validate not found")
            .cast::<v8::Function>();

        // Create Uint8Array for testing
        let ab = v8::ArrayBuffer::new(context_scope, wasm_bytes.len());
        let store = ab.get_backing_store();
        for (i, byte) in wasm_bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(*byte);
            }
        }
        let uint8array = v8::Uint8Array::new(context_scope, ab, 0, wasm_bytes.len())
            .expect("Failed to create Uint8Array");

        let validate_result =
            validate_fn.call(context_scope, wasm_obj.into(), &[uint8array.into()]);
        match validate_result {
            Some(val) => {
                let is_valid = val.is_true();
                println!("WebAssembly.validate() returned: {}", is_valid);
                if is_valid {
                    println!("✅ WebAssembly.validate() says WASM is valid!");
                } else {
                    println!("❌ WebAssembly.validate() says WASM is invalid");
                }
            }
            None => {
                println!("❌ WebAssembly.validate() threw exception");
            }
        }
    });
}
