//! ReadableStream and WritableStream JavaScript implementation for streaming
//!
//! This module provides the ReadableStream API for streaming response bodies
//! from fetch() requests, and WritableStream API for streaming request bodies.
//! It implements backpressure handling and zero-copy data transfer.

use bytes::Bytes;
use std::cell::RefCell;
use std::collections::HashMap;

/// Resource table entry for active streams
#[derive(Debug)]
pub struct StreamResource {
    /// Unique resource ID
    pub rid: u32,
    /// Whether the stream is closed
    pub closed: bool,
}

/// Resource table for tracking active ReadableStreams
pub struct StreamResourceTable {
    resources: RefCell<HashMap<u32, StreamResource>>,
    next_rid: RefCell<u32>,
}

impl StreamResourceTable {
    /// Create a new resource table
    pub fn new() -> Self {
        Self {
            resources: RefCell::new(HashMap::new()),
            next_rid: RefCell::new(1),
        }
    }

    /// Add a new resource and return its ID
    pub fn add(&self) -> u32 {
        let rid = *self.next_rid.borrow();
        *self.next_rid.borrow_mut() += 1;

        let resource = StreamResource { rid, closed: false };
        self.resources.borrow_mut().insert(rid, resource);
        rid
    }

    /// Close a resource by ID
    pub fn close(&self, rid: u32) -> bool {
        if let Some(resource) = self.resources.borrow_mut().get_mut(&rid) {
            resource.closed = true;
            true
        } else {
            false
        }
    }

    /// Check if a resource exists
    pub fn has(&self, rid: u32) -> bool {
        self.resources.borrow().contains_key(&rid)
    }
}

impl Default for StreamResourceTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Bind ReadableStream and related APIs to the global scope
pub fn bind_streams(
    scope: &mut v8::PinnedRef<v8::HandleScope<()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create ReadableStream constructor
    let rs_template = v8::FunctionTemplate::new(&mut ctx_scope, readable_stream_constructor);
    let rs_ctor = rs_template.get_function(&mut ctx_scope).unwrap();

    // Add getReader method to prototype
    if let Some(rs_obj) = rs_ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = rs_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                if let Some(get_reader_fn) =
                    v8::Function::new(&mut ctx_scope, readable_stream_get_reader)
                {
                    let get_reader_key = v8::String::new(&mut ctx_scope, "getReader").unwrap();
                    proto_obj.set(&mut ctx_scope, get_reader_key.into(), get_reader_fn.into());
                }
            }
        }
    }

    let rs_key = v8::String::new(&mut ctx_scope, "ReadableStream").unwrap();
    global.set(&mut ctx_scope, rs_key.into(), rs_ctor.into());

    // Create ReadableStreamDefaultReader constructor
    let reader_template =
        v8::FunctionTemplate::new(&mut ctx_scope, readable_stream_default_reader_constructor);
    let reader_ctor = reader_template.get_function(&mut ctx_scope).unwrap();

    // Add read method to prototype
    if let Some(reader_obj) = reader_ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = reader_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                if let Some(read_fn) = v8::Function::new(&mut ctx_scope, reader_read_callback) {
                    let read_key = v8::String::new(&mut ctx_scope, "read").unwrap();
                    proto_obj.set(&mut ctx_scope, read_key.into(), read_fn.into());
                }
                if let Some(release_fn) =
                    v8::Function::new(&mut ctx_scope, reader_release_lock_callback)
                {
                    let release_key = v8::String::new(&mut ctx_scope, "releaseLock").unwrap();
                    proto_obj.set(&mut ctx_scope, release_key.into(), release_fn.into());
                }
            }
        }
    }

    let reader_key = v8::String::new(&mut ctx_scope, "ReadableStreamDefaultReader").unwrap();
    global.set(&mut ctx_scope, reader_key.into(), reader_ctor.into());

    // Create WritableStream constructor
    let ws_template = v8::FunctionTemplate::new(&mut ctx_scope, writable_stream_constructor);
    let ws_ctor = ws_template.get_function(&mut ctx_scope).unwrap();

    // Add getWriter method to prototype
    if let Some(ws_obj) = ws_ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = ws_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                if let Some(get_writer_fn) =
                    v8::Function::new(&mut ctx_scope, writable_stream_get_writer)
                {
                    let get_writer_key = v8::String::new(&mut ctx_scope, "getWriter").unwrap();
                    proto_obj.set(&mut ctx_scope, get_writer_key.into(), get_writer_fn.into());
                }
            }
        }
    }

    let ws_key = v8::String::new(&mut ctx_scope, "WritableStream").unwrap();
    global.set(&mut ctx_scope, ws_key.into(), ws_ctor.into());

    // Create WritableStreamDefaultWriter constructor
    let writer_template =
        v8::FunctionTemplate::new(&mut ctx_scope, writable_stream_default_writer_constructor);
    let writer_ctor = writer_template.get_function(&mut ctx_scope).unwrap();

    // Add write and close methods to prototype
    if let Some(writer_obj) = writer_ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = writer_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                if let Some(write_fn) = v8::Function::new(&mut ctx_scope, writer_write_callback) {
                    let write_key = v8::String::new(&mut ctx_scope, "write").unwrap();
                    proto_obj.set(&mut ctx_scope, write_key.into(), write_fn.into());
                }
                if let Some(close_fn) = v8::Function::new(&mut ctx_scope, writer_close_callback) {
                    let close_key = v8::String::new(&mut ctx_scope, "close").unwrap();
                    proto_obj.set(&mut ctx_scope, close_key.into(), close_fn.into());
                }
                if let Some(release_fn) =
                    v8::Function::new(&mut ctx_scope, writer_release_lock_callback)
                {
                    let release_key = v8::String::new(&mut ctx_scope, "releaseLock").unwrap();
                    proto_obj.set(&mut ctx_scope, release_key.into(), release_fn.into());
                }
            }
        }
    }

    let writer_key = v8::String::new(&mut ctx_scope, "WritableStreamDefaultWriter").unwrap();
    global.set(&mut ctx_scope, writer_key.into(), writer_ctor.into());

    tracing::debug!("Streams API bindings initialized");
}

// ============== ReadableStream JavaScript Bindings ==============

fn readable_stream_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Store underlying source if provided
    if args.length() > 0 {
        let source = args.get(0);
        let source_key = v8::String::new(scope, "__source").unwrap();
        this.set(scope, source_key.into(), source.into());
    }

    // Initialize state
    let state_key = v8::String::new(scope, "__state").unwrap();
    let state_val = v8::String::new(scope, "readable").unwrap();
    this.set(scope, state_key.into(), state_val.into());

    retval.set(this.into());
}

fn readable_stream_get_reader(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let global = scope.get_current_context().global(scope);

    // Create reader instance
    let reader_key = v8::String::new(scope, "ReadableStreamDefaultReader").unwrap();
    if let Some(reader_ctor) = global.get(scope, reader_key.into()) {
        if let Some(reader_fn) = reader_ctor.to_object(scope) {
            if let Some(reader_func) = reader_fn
                .cast::<v8::Function>()
                .new_instance(scope, &[this.into()])
            {
                retval.set(reader_func.into());
                return;
            }
        }
    }

    retval.set(v8::null(scope).into());
}

fn readable_stream_default_reader_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Store reference to stream
    if args.length() > 0 {
        let stream = args.get(0);
        let stream_key = v8::String::new(scope, "__stream").unwrap();
        this.set(scope, stream_key.into(), stream.into());
    }

    retval.set(this.into());
}

fn reader_read_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Return { done: true, value: undefined } for basic implementation
    let result = v8::Object::new(scope);
    let done_key = v8::String::new(scope, "done").unwrap();
    let value_key = v8::String::new(scope, "value").unwrap();
    let true_val = v8::Boolean::new(scope, true);
    let undefined_val = v8::undefined(scope);

    result.set(scope, done_key.into(), true_val.into());
    result.set(scope, value_key.into(), undefined_val.into());

    retval.set(result.into());
}

fn reader_release_lock_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    retval.set(v8::undefined(scope).into());
}

// ============== WritableStream JavaScript Bindings ==============

fn writable_stream_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Store underlying sink if provided
    if args.length() > 0 {
        let sink = args.get(0);
        let sink_key = v8::String::new(scope, "__sink").unwrap();
        this.set(scope, sink_key.into(), sink.into());
    }

    retval.set(this.into());
}

fn writable_stream_get_writer(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let global = scope.get_current_context().global(scope);

    // Create writer instance
    let writer_key = v8::String::new(scope, "WritableStreamDefaultWriter").unwrap();
    if let Some(writer_ctor) = global.get(scope, writer_key.into()) {
        if let Some(writer_fn) = writer_ctor.to_object(scope) {
            if let Some(writer_func) = writer_fn
                .cast::<v8::Function>()
                .new_instance(scope, &[this.into()])
            {
                retval.set(writer_func.into());
                return;
            }
        }
    }

    retval.set(v8::null(scope).into());
}

fn writable_stream_default_writer_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Store reference to stream
    if args.length() > 0 {
        let stream = args.get(0);
        let stream_key = v8::String::new(scope, "__stream").unwrap();
        this.set(scope, stream_key.into(), stream.into());
    }

    retval.set(this.into());
}

fn writer_write_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Return Promise.resolve() for basic implementation
    let global = scope.get_current_context().global(scope);
    let promise_key = v8::String::new(scope, "Promise").unwrap();
    let undefined_val = v8::undefined(scope);

    if let Some(promise_ctor) = global.get(scope, promise_key.into()) {
        if let Some(promise_fn) = promise_ctor.to_object(scope) {
            if let Some(promise_func) = promise_fn
                .cast::<v8::Function>()
                .new_instance(scope, &[undefined_val.into()])
            {
                retval.set(promise_func.into());
                return;
            }
        }
    }

    retval.set(undefined_val.into());
}

fn writer_close_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Return Promise.resolve() for basic implementation
    let global = scope.get_current_context().global(scope);
    let promise_key = v8::String::new(scope, "Promise").unwrap();
    let undefined_val = v8::undefined(scope);

    if let Some(promise_ctor) = global.get(scope, promise_key.into()) {
        if let Some(promise_fn) = promise_ctor.to_object(scope) {
            if let Some(promise_func) = promise_fn
                .cast::<v8::Function>()
                .new_instance(scope, &[undefined_val.into()])
            {
                retval.set(promise_func.into());
                return;
            }
        }
    }

    retval.set(undefined_val.into());
}

fn writer_release_lock_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    retval.set(v8::undefined(scope).into());
}

/// UnderlyingSink trait for Rust-side data consumption
///
/// Implement this trait to receive data written to a WritableStream.
/// The sink can apply backpressure by not returning until data is processed.
pub trait UnderlyingSink: Send {
    /// Called when the stream is constructed
    fn start(&mut self) -> Result<(), anyhow::Error> {
        Ok(())
    }

    /// Called when a chunk is written to the stream
    /// Returns a future that resolves when the chunk has been processed
    fn write(
        &mut self,
        chunk: Bytes,
    ) -> impl std::future::Future<Output = Result<(), anyhow::Error>> + Send;

    /// Called when the stream is closed
    fn close(&mut self) -> impl std::future::Future<Output = Result<(), anyhow::Error>> + Send {
        async { Ok(()) }
    }

    /// Called when the stream is aborted
    fn abort(
        &mut self,
        _reason: Option<String>,
    ) -> impl std::future::Future<Output = Result<(), anyhow::Error>> + Send {
        async { Ok(()) }
    }
}

#[path = "stream_impl.rs"]
pub mod stream_impl;
pub use stream_impl::{WritableStream, WritableStreamDefaultWriter, WriteError, WriteResult};
