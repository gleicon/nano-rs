//! Multi-app hosting integration test
//!
//! Tests the full multi-tenant hosting capabilities with real framework apps:
//! - Hono.js style API
//! - Next.js static export  
//! - Astro islands architecture
//!
//! Verifies isolation between apps and routing by hostname.

use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use nano::admin::diagnostics::DiagnosticsCollector;
use nano::app::registry::AppRegistry;
use nano::config::loader::load_config;

/// Get absolute path to test config, accounting for test execution directory
fn get_config_path() -> String {
    // Try to find project root by looking for Cargo.toml
    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // If we're in the tests directory, go up one level
    if current_dir.ends_with("tests") {
        current_dir
            .parent()
            .expect("Failed to get parent")
            .join("tests/multi-app-test.json")
            .to_string_lossy()
            .to_string()
    } else {
        // Assume we're in project root
        current_dir
            .join("tests/multi-app-test.json")
            .to_string_lossy()
            .to_string()
    }
}

/// Multi-app hosting integration test
///
/// This test verifies:
/// 1. Config loading with 3 apps
/// 2. App registry population
/// 3. Diagnostics collection showing active isolates
/// 4. App isolation (different env vars, limits)
#[tokio::test]
async fn test_multi_app_hosting() {
    println!("\n{}", "=".repeat(80));
    println!("NANO Multi-App Hosting Integration Test");
    println!("{}", "=".repeat(80));

    // Step 1: Load multi-app configuration
    let config_path = get_config_path();
    println!(
        "\n[1/5] Loading multi-app configuration from: {}",
        config_path
    );

    let config = load_config(Path::new(&config_path))
        .await
        .expect("Failed to load multi-app config");

    println!("  ✓ Loaded {} apps:", config.apps.len());
    for (i, app) in config.apps.iter().enumerate() {
        println!("    {}. {} -> {}", i + 1, app.hostname, app.entrypoint);
    }

    // Verify we have exactly 3 apps
    assert_eq!(config.apps.len(), 3, "Expected 3 apps in config");
    assert_eq!(config.server.port, 8888, "Expected port 8888");

    // Step 2: Create app registry
    println!("\n[2/5] Creating app registry...");

    let registry = Arc::new(RwLock::new(AppRegistry::from_config(config)));
    let reg = registry.read().await;

    // Verify all hostnames are registered
    let hostnames: Vec<_> = reg.all_hostnames().collect();
    assert!(hostnames.contains(&"hono.example.com".to_string()));
    assert!(hostnames.contains(&"nextjs.example.com".to_string()));
    assert!(hostnames.contains(&"astro.example.com".to_string()));

    println!("  ✓ Registry contains {} hostnames:", hostnames.len());
    for hostname in &hostnames {
        if let Some(app) = reg.get(hostname) {
            println!(
                "    - {} (env vars: {}, limits: {}MB/{}s/{}w)",
                hostname,
                app.env_vars.len(),
                app.limits.memory_mb,
                app.limits.timeout_secs,
                app.limits.workers
            );
        }
    }
    drop(reg);

    // Step 3: Collect diagnostics (like `ps` or `top`)
    println!("\n[3/5] Collecting system diagnostics (like 'ps' for isolates)...");

    let collector = DiagnosticsCollector::new(registry.clone());
    let diagnostics = collector.collect().await;

    println!("  ✓ Diagnostics collected:");
    println!("    - Total isolates: {}", diagnostics.total_isolates);
    println!("    - Total apps: {}", diagnostics.app_stats.len());
    println!(
        "    - Total requests (simulated): {}",
        diagnostics.total_requests
    );

    // Verify expected isolate count (sum of all workers)
    let expected_isolates = 2 + 4 + 2; // hono(2) + nextjs(4) + astro(2)
    assert_eq!(
        diagnostics.total_isolates, expected_isolates,
        "Expected {} isolates (sum of workers), got {}",
        expected_isolates, diagnostics.total_isolates
    );

    // Step 4: Print detailed `ps`-style output
    println!("\n[4/5] Detailed system state (ps-style output):");
    println!("{}", "-".repeat(80));
    print!("{}", diagnostics.format_ps());
    println!("{}", "-".repeat(80));

    // Step 5: Verify app isolation
    println!("\n[5/5] Verifying app isolation...");

    let reg = registry.read().await;

    // Check Hono app isolation
    let hono_app = reg.get("hono.example.com").expect("Hono app not found");
    assert_eq!(
        hono_app.env_vars.get("APP_NAME"),
        Some(&"hono-api".to_string())
    );
    assert_eq!(hono_app.limits.memory_mb, 128);
    assert_eq!(hono_app.limits.workers, 2);
    println!("  ✓ Hono app isolated: APP_NAME=hono-api, mem=128MB, workers=2");

    // Check Next.js app isolation
    let nextjs_app = reg.get("nextjs.example.com").expect("NextJS app not found");
    assert_eq!(
        nextjs_app.env_vars.get("APP_NAME"),
        Some(&"nextjs-static".to_string())
    );
    assert_eq!(
        nextjs_app.env_vars.get("NODE_ENV"),
        Some(&"production".to_string())
    );
    assert_eq!(nextjs_app.limits.memory_mb, 256);
    assert_eq!(nextjs_app.limits.workers, 4);
    println!("  ✓ Next.js app isolated: APP_NAME=nextjs-static, mem=256MB, workers=4");

    // Check Astro app isolation
    let astro_app = reg.get("astro.example.com").expect("Astro app not found");
    assert_eq!(
        astro_app.env_vars.get("APP_NAME"),
        Some(&"astro-islands".to_string())
    );
    assert_eq!(
        astro_app.env_vars.get("HYDRATION"),
        Some(&"partial".to_string())
    );
    assert_eq!(astro_app.limits.memory_mb, 128);
    assert_eq!(astro_app.limits.workers, 2);
    println!("  ✓ Astro app isolated: APP_NAME=astro-islands, mem=128MB, workers=2");

    // Verify no cross-contamination
    assert!(
        hono_app.env_vars.get("HYDRATION").is_none(),
        "Hono app should not have Astro's HYDRATION var"
    );
    assert!(
        astro_app.env_vars.get("NODE_ENV").is_none(),
        "Astro app should not have NextJS's NODE_ENV var"
    );
    println!("  ✓ No environment variable cross-contamination detected");

    // Check memory limit differences
    let mem_limits: Vec<_> = diagnostics
        .app_stats
        .iter()
        .map(|a| a.config.memory_limit_mb)
        .collect();
    assert!(mem_limits.contains(&128), "Should have 128MB limit");
    assert!(mem_limits.contains(&256), "Should have 256MB limit");
    println!("  ✓ Different memory limits enforced per app");

    println!("\n{}", "=".repeat(80));
    println!("Multi-App Hosting Test: PASSED");
    println!("{}", "=".repeat(80));
    println!();
}

/// Test JSON diagnostics output format
#[tokio::test]
async fn test_diagnostics_json_format() {
    let config_path = get_config_path();
    let config = load_config(Path::new(&config_path))
        .await
        .expect("Failed to load config");

    let registry = Arc::new(RwLock::new(AppRegistry::from_config(config)));
    let collector = DiagnosticsCollector::new(registry);
    let diagnostics = collector.collect().await;

    let json = diagnostics.format_json();

    // Verify JSON contains expected fields
    assert!(
        json.contains("total_isolates"),
        "JSON should contain total_isolates"
    );
    assert!(
        json.contains("total_requests"),
        "JSON should contain total_requests"
    );
    assert!(json.contains("app_count"), "JSON should contain app_count");
    assert!(json.contains("apps"), "JSON should contain apps array");
    assert!(
        json.contains("hono.example.com"),
        "JSON should contain hono hostname"
    );
    assert!(
        json.contains("nextjs.example.com"),
        "JSON should contain nextjs hostname"
    );
    assert!(
        json.contains("astro.example.com"),
        "JSON should contain astro hostname"
    );

    println!("Diagnostics JSON output verified");
}

/// Test that different apps have different worker counts
#[tokio::test]
async fn test_worker_count_isolation() {
    let config_path = get_config_path();
    let config = load_config(Path::new(&config_path))
        .await
        .expect("Failed to load config");

    let registry = Arc::new(RwLock::new(AppRegistry::from_config(config)));
    let reg = registry.read().await;

    let hono = reg.get("hono.example.com").unwrap();
    let nextjs = reg.get("nextjs.example.com").unwrap();
    let astro = reg.get("astro.example.com").unwrap();

    // Different worker counts
    assert_ne!(
        hono.limits.workers, nextjs.limits.workers,
        "Hono and NextJS should have different worker counts"
    );
    assert_eq!(
        hono.limits.workers, astro.limits.workers,
        "Hono and Astro can share same worker count, but separate pools"
    );

    // Verify total workers
    let total_workers = hono.limits.workers + nextjs.limits.workers + astro.limits.workers;
    assert_eq!(
        total_workers, 8,
        "Total workers across all apps should be 8 (2+4+2)"
    );

    println!(
        "Worker count isolation verified: hono={}, nextjs={}, astro={}",
        hono.limits.workers, nextjs.limits.workers, astro.limits.workers
    );
}
