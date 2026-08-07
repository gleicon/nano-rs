//! CRUD Operations Integration Tests
//!
//! Tests Create, Read, Update, Delete operations via HTTP handlers.
//! These tests verify full REST API functionality.

use tokio::sync::oneshot;

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::v8::initialize_platform;
use nano::worker::{HandlerTask, WorkQueue};

/// Test CREATE operation - POST request creates a resource
#[tokio::test]
async fn test_crud_create() {
    let _ = initialize_platform();

    let mut queue = WorkQueue::new(1);
    let hostname = "crud-test.local";

    // Handler with in-memory storage
    let js_code = r#"
        const storage = new Map();
        let nextId = 1;
        
        export default {
            async fetch(request) {
                const url = new URL(request.url);
                
                if (request.method === 'POST' && url.pathname === '/items') {
                    const body = await request.json();
                    const id = nextId++;
                    const item = { id, ...body, created: Date.now() };
                    storage.set(id, item);
                    
                    return new Response(JSON.stringify(item), {
                        status: 201,
                        headers: { 'Content-Type': 'application/json' }
                    });
                }
                
                return new Response('Not Found', { status: 404 });
            }
        };
    "#;

    // Write JS to temp file
    let temp_dir = std::env::temp_dir();
    let entrypoint_path = temp_dir.join(format!("crud_{}.js", hostname.replace(".", "_")));
    std::fs::write(&entrypoint_path, js_code).unwrap();

    // Create request
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/items", hostname)).unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");

    let request = NanoRequest::new(
        "POST".to_string(),
        url,
        headers,
        Some(r#"{"name":"Test Item","value":42}"#.as_bytes().to_vec().into()),
    );

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_create_001".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();

    assert_eq!(response.status(), 201, "CREATE should return 201 Created");

    let body = response
        .body()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    assert!(
        body.contains("id"),
        "Response should contain created item with id"
    );
    assert!(
        body.contains("Test Item"),
        "Response should contain item name"
    );
    assert!(body.contains("42"), "Response should contain item value");

    println!("✅ CRUD CREATE test passed!");
}

/// Test READ operation - GET request retrieves resources
#[tokio::test]
async fn test_crud_read() {
    let _ = initialize_platform();

    let mut queue = WorkQueue::new(1);
    let hostname = "crud-read-test.local";

    let js_code = r#"
        const items = new Map([
            [1, { id: 1, name: 'Item 1', value: 100 }],
            [2, { id: 2, name: 'Item 2', value: 200 }]
        ]);
        
        export default {
            async fetch(request) {
                const url = new URL(request.url);
                const match = url.pathname.match(/\/items\/(\d+)/);
                
                if (request.method === 'GET' && url.pathname === '/items') {
                    // List all
                    const all = Array.from(items.values());
                    return Response.json(all);
                }
                
                if (request.method === 'GET' && match) {
                    // Get one
                    const id = parseInt(match[1]);
                    const item = items.get(id);
                    
                    if (item) {
                        return Response.json(item);
                    }
                    return new Response('Not Found', { status: 404 });
                }
                
                return new Response('Not Found', { status: 404 });
            }
        };
    "#;

    // Write JS to temp file
    let temp_dir = std::env::temp_dir();
    let entrypoint_path = temp_dir.join(format!("crud_{}.js", hostname.replace(".", "_")));
    std::fs::write(&entrypoint_path, js_code).unwrap();

    // Test READ ALL
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/items", hostname)).unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_read_all".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();

    assert_eq!(response.status(), 200);
    let body = response
        .body()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    assert!(body.contains("Item 1"));
    assert!(body.contains("Item 2"));

    // Test READ ONE
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/items/1", hostname)).unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_read_one".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();

    assert_eq!(response.status(), 200);
    let body = response
        .body()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    assert!(body.contains("Item 1"));
    assert!(!body.contains("Item 2")); // Should only have one item

    println!("✅ CRUD READ test passed!");
}

/// Test UPDATE operation - PUT/PATCH modifies resources
#[tokio::test]
async fn test_crud_update() {
    let _ = initialize_platform();

    let mut queue = WorkQueue::new(1);
    let hostname = "crud-update-test.local";

    let js_code = r#"
        const items = new Map([
            [1, { id: 1, name: 'Original', value: 100 }]
        ]);
        
        export default {
            async fetch(request) {
                const url = new URL(request.url);
                const match = url.pathname.match(/\/items\/(\d+)/);
                
                if (request.method === 'PUT' && match) {
                    const id = parseInt(match[1]);
                    const existing = items.get(id);
                    
                    if (!existing) {
                        return new Response('Not Found', { status: 404 });
                    }
                    
                    const body = await request.json();
                    const updated = { ...existing, ...body, id, updated: Date.now() };
                    items.set(id, updated);
                    
                    return Response.json(updated);
                }
                
                return new Response('Not Found', { status: 404 });
            }
        };
    "#;

    // Write JS to temp file
    let temp_dir = std::env::temp_dir();
    let entrypoint_path = temp_dir.join(format!("crud_{}.js", hostname.replace(".", "_")));
    std::fs::write(&entrypoint_path, js_code).unwrap();

    // Test UPDATE
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/items/1", hostname)).unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");

    let request = NanoRequest::new(
        "PUT".to_string(),
        url,
        headers,
        Some(r#"{"name":"Updated","value":999}"#.as_bytes().to_vec().into()),
    );

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_update".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();

    assert_eq!(response.status(), 200);
    let body = response
        .body()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    assert!(
        body.contains("Updated"),
        "Response should contain updated name"
    );
    assert!(
        body.contains("999"),
        "Response should contain updated value"
    );
    assert!(
        body.contains("updated"),
        "Response should have updated timestamp"
    );

    println!("✅ CRUD UPDATE test passed!");
}

/// Test DELETE operation - DELETE removes resources
/// Note: Each request runs in a fresh context, so we test DELETE in isolation
#[tokio::test]
async fn test_crud_delete() {
    let _ = initialize_platform();

    let mut queue = WorkQueue::new(1);
    let hostname = "crud-delete-test.local";

    let js_code = r#"
        const items = new Map([
            [1, { id: 1, name: 'To Delete' }]
        ]);

        export default {
            async fetch(request) {
                const url = new URL(request.url);
                const match = url.pathname.match(/\/items\/(\d+)/);

                if (request.method === 'DELETE' && match) {
                    const id = parseInt(match[1]);

                    if (items.has(id)) {
                        items.delete(id);
                        return new Response(null, { status: 204 });
                    }

                    return new Response('Not Found', { status: 404 });
                }

                return new Response('Not Found', { status: 404 });
            }
        };
    "#;

    // Write JS to temp file
    let temp_dir = std::env::temp_dir();
    let entrypoint_path = temp_dir.join(format!("crud_{}.js", hostname.replace(".", "_")));
    std::fs::write(&entrypoint_path, js_code).unwrap();

    // Test DELETE existing item -> 204
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/items/1", hostname)).unwrap();
    let request = NanoRequest::new("DELETE".to_string(), url, NanoHeaders::new(), None);

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_delete".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();
    assert_eq!(
        response.status(),
        204,
        "DELETE should return 204 No Content"
    );

    // Test DELETE non-existent item -> 404 (each request has fresh context)
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/items/99", hostname)).unwrap();
    let request = NanoRequest::new("DELETE".to_string(), url, NanoHeaders::new(), None);

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_delete_404".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();
    assert_eq!(
        response.status(),
        404,
        "DELETE non-existent should return 404"
    );

    println!("✅ CRUD DELETE test passed!");
}

/// Test full CRUD cycle in one handler
/// Note: Context resets between requests, so all operations run in a single request
#[tokio::test]
async fn test_crud_full_cycle() {
    let _ = initialize_platform();

    let mut queue = WorkQueue::new(1);
    let hostname = "crud-full-test.local";

    let js_code = r#"
        const items = new Map();
        let nextId = 1;

        export default {
            async fetch(request) {
                const url = new URL(request.url);

                // Internal full-cycle test endpoint
                if (request.method === 'POST' && url.pathname === '/test-cycle') {
                    // CREATE
                    const id = nextId++;
                    const item = { id, name: 'Test', value: 42, created: Date.now() };
                    items.set(id, item);

                    // READ
                    const readItem = items.get(id);
                    if (!readItem) {
                        return Response.json({ error: 'read failed' }, { status: 500 });
                    }

                    // UPDATE
                    const updated = { ...readItem, name: 'Updated', updated: Date.now() };
                    items.set(id, updated);

                    // VERIFY UPDATE
                    const verifyItem = items.get(id);
                    if (verifyItem.name !== 'Updated') {
                        return Response.json({ error: 'verify failed' }, { status: 500 });
                    }

                    // DELETE
                    items.delete(id);

                    // VERIFY DELETE
                    const count = items.size;

                    return Response.json({
                        created: item,
                        read: readItem,
                        updated: verifyItem,
                        finalCount: count,
                        passed: true
                    });
                }

                return new Response('Not Found', { status: 404 });
            }
        };
    "#;

    // Write JS to temp file
    let temp_dir = std::env::temp_dir();
    let entrypoint_path = temp_dir.join(format!("crud_{}.js", hostname.replace(".", "_")));
    std::fs::write(&entrypoint_path, js_code).unwrap();

    // Single request runs all CRUD operations
    let (tx, rx) = oneshot::channel();
    let url = NanoUrl::parse(&format!("http://{}/test-cycle", hostname)).unwrap();

    let request = NanoRequest::new("POST".to_string(), url, NanoHeaders::new(), None);

    let task = HandlerTask::new_with_request_id(
        entrypoint_path.to_str().unwrap().to_string(),
        request,
        tx,
        hostname.to_string(),
        "req_crud_full_cycle".to_string(),
    );

    queue.dispatch(hostname, task).await.unwrap();
    let response = rx.await.unwrap().unwrap();
    assert_eq!(response.status(), 200, "Full cycle should succeed");

    let body = response
        .body()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    assert!(
        body.contains("\"passed\":true"),
        "Full cycle should report passed=true"
    );
    assert!(
        body.contains("\"finalCount\":0"),
        "Final count should be 0 after delete"
    );
    assert!(
        body.contains("\"name\":\"Updated\""),
        "Updated name should be present"
    );

    println!("✅ CRUD FULL CYCLE test passed!");
}
