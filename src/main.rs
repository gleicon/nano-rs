use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

mod cli;

/// NANO Edge Runtime - Multi-tenant JavaScript edge runtime
#[derive(Debug, Parser)]
#[command(name = "nano-rs")]
#[command(about = "Multi-tenant JavaScript edge runtime (Rust + rusty_v8)")]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (v8-crate: 152.2.0)"))]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the NANO HTTP server (default behavior)
    Run {
        /// Configuration file path
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,

        /// Sliver file to run directly (conflicts with --config)
        #[arg(long, value_name = "FILE", conflicts_with = "config")]
        sliver: Option<PathBuf>,

        /// Number of workers when using --sliver
        #[arg(short, long, default_value = "4")]
        workers: usize,

        /// Serve static files to any hostname (disables strict multi-tenancy)
        /// By default, NANO returns 404 for requests with wrong Host header.
        /// Use --static for local development to serve VFS files regardless of Host.
        #[arg(long)]
        static_files: bool,

        /// Override the hostname from the sliver metadata
        /// Useful when running behind a proxy or with different DNS
        #[arg(long, value_name = "HOST")]
        hostname: Option<String>,
    },

    /// Sliver management commands (create, list, and delete app bundles)
    #[command(subcommand)]
    Sliver(cli::SliverCommand),

    /// Show detailed version information including V8 versions
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured JSON logging with env-filter support
    nano::logging::init_logging();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run {
            config,
            sliver,
            workers,
            static_files,
            hostname,
        }) => {
            if let Some(sliver_path) = sliver {
                // Run from sliver file
                run_from_sliver(sliver_path, workers, static_files, hostname).await
            } else if let Some(config_path) = config {
                // Run with config file
                run_server_with_config(config_path).await
            } else {
                // Default behavior: run the server
                run_server().await
            }
        }
        None => {
            // Default behavior: run the server
            run_server().await
        }
        Some(Commands::Sliver(sliver_cmd)) => {
            // Execute sliver command
            handle_sliver_command(sliver_cmd).await
        }
        Some(Commands::Version) => {
            // Initialize V8 platform to get the actual engine version
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();

            let v8_engine_version = v8::V8::get_version();
            let v8_crate_version = "152.2.0"; // From Cargo.lock
            let app_version = env!("CARGO_PKG_VERSION");

            println!(
                "nano-rs {} (v8: {}, v8-crate: {})",
                app_version, v8_engine_version, v8_crate_version
            );
            Ok(())
        }
    }
}

/// Run the NANO HTTP server with graceful shutdown
async fn run_server() -> Result<()> {
    tracing::info!("NANO Edge Runtime starting...");

    // Initialize V8 platform (once per process)
    nano::v8::platform::initialize_platform().context("Failed to initialize V8 platform")?;
    tracing::info!("V8 platform initialized");

    // Set up graceful shutdown with signal handling
    let drain = nano::app::drain::RequestDrain::new();
    let shutdown_config = nano::signal::ShutdownConfig::default();
    let drain_timeout_secs = shutdown_config.drain_timeout_secs;
    let (shutdown, mut shutdown_rx) = nano::signal::setup_shutdown(shutdown_config, drain);
    tracing::info!(
        "Graceful shutdown initialized (drain timeout: {}s)",
        drain_timeout_secs
    );

    // Shared registry (metadata) and HTTP router (live routing table).
    // Both admin API and HTTP server hold a clone of the same Arc so admin
    // API writes are immediately visible to the HTTP server.
    let registry = Arc::new(RwLock::new(nano::app::registry::AppRegistry::default()));
    let shared_router = Arc::new(RwLock::new(nano::http::router::VirtualHostRouter::default()));

    // Start HTTP server with the shared router.
    // from_env() reads NANO_HOST and NANO_PORT; falls back to 127.0.0.1:8080.
    let config = nano::http::ServerConfig::from_env()?;
    tracing::info!("Starting HTTP server on {}", config.socket_addr()?);

    let shutdown_state = shutdown.state().clone();
    let router_for_http = shared_router.clone();
    let server_handle = tokio::spawn(async move {
        nano::http::start_server_with_shared_router(router_for_http, config, shutdown_state).await
    });

    // Start Admin API server (optional)
    let admin_api_key = std::env::var("NANO_ADMIN_API_KEY").unwrap_or_default();
    let admin_host = std::env::var("NANO_ADMIN_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let unix_socket_path = std::env::var("NANO_ADMIN_UNIX_SOCKET").ok();

    let admin_handle = if !admin_api_key.is_empty() {
        let admin_config =
            nano::admin::server::AdminConfig::new(admin_api_key).with_host(admin_host);
        let admin_state =
            nano::admin::server::AdminState::new(registry.clone(), shared_router.clone());

        match nano::admin::server::start_admin_server(admin_config, admin_state).await {
            Ok(admin_server) => {
                tracing::info!("Admin API server started on {}", admin_server.local_addr);
                Some(admin_server)
            }
            Err(e) => {
                tracing::error!("Failed to start Admin API server: {}", e);
                None
            }
        }
    } else {
        tracing::info!("Admin API server not started (NANO_ADMIN_API_KEY not set)");
        None
    };

    // Start Unix socket admin server (optional)
    let unix_socket_handle = if let Some(socket_path) = unix_socket_path {
        let unix_config = nano::admin::unix_socket::UnixSocketConfig::new(socket_path);
        // Auth object passed for API compat but not used; socket is gated by filesystem permissions
        let unix_auth = std::sync::Arc::new(nano::admin::auth::AdminAuth::new(""));
        let unix_state =
            nano::admin::server::AdminState::new(registry.clone(), shared_router.clone());

        match nano::admin::unix_socket::start_unix_socket_server(unix_config, unix_state, unix_auth)
            .await
        {
            Ok(unix_server) => {
                tracing::info!(
                    "Unix socket admin server started at {}",
                    unix_server.socket_path().display()
                );
                Some(unix_server)
            }
            Err(e) => {
                tracing::error!("Failed to start Unix socket admin server: {}", e);
                None
            }
        }
    } else {
        tracing::info!("Unix socket admin server not started (NANO_ADMIN_UNIX_SOCKET not set)");
        None
    };

    // Wait for shutdown signal
    tracing::info!("Waiting for shutdown signal (SIGTERM or Ctrl+C)...");
    let _ = shutdown_rx.recv().await;
    tracing::info!("Shutdown signal received, initiating graceful shutdown...");

    // Perform graceful shutdown
    shutdown.shutdown().await;

    // Shut down admin servers
    if let Some(admin_server) = admin_handle {
        tracing::info!("Shutting down Admin API server...");
        admin_server.shutdown().await;
    }

    if let Some(unix_server) = unix_socket_handle {
        tracing::info!("Shutting down Unix socket admin server...");
        unix_server.shutdown().await;
    }

    // Wait for main server
    match server_handle.await {
        Ok(result) => {
            if let Err(e) = result {
                tracing::error!("Server error during shutdown: {}", e);
            }
        }
        Err(e) => {
            tracing::error!("Server task panicked: {}", e);
        }
    }

    tracing::info!("NANO Edge Runtime shutdown complete");
    Ok(())
}

/// Run server from a sliver file
///
/// Loads the sliver, extracts the hostname and snapshot, creates a
/// SliverWorkerPool, and starts the HTTP server.
///
/// # Arguments
///
/// * `sliver_path` - Path to the sliver file
/// * `workers` - Number of worker threads
/// * `static_files` - If true, serve VFS to any hostname (dev mode).
///                    If false, only serve to exact hostname match (strict multi-tenancy).
/// * `hostname_override` - Optional override for the hostname from sliver metadata
async fn run_from_sliver(
    sliver_path: PathBuf,
    workers: usize,
    static_files: bool,
    hostname_override: Option<String>,
) -> Result<()> {
    tracing::info!("Starting NANO from sliver: {}", sliver_path.display());

    // Validate sliver file exists
    if !sliver_path.exists() {
        anyhow::bail!("Sliver file not found: {}", sliver_path.display());
    }

    // Initialize V8 platform
    nano::v8::platform::initialize_platform().context("Failed to initialize V8 platform")?;
    tracing::info!("V8 platform initialized");

    // Read and unpack sliver
    let sliver_data = std::fs::read(&sliver_path)
        .with_context(|| format!("Failed to read sliver file: {}", sliver_path.display()))?;

    let unpacked = nano::sliver::unpack_sliver(&sliver_data)
        .with_context(|| format!("Failed to unpack sliver: {}", sliver_path.display()))?;

    tracing::info!(
        "Unpacked sliver for {}: {} VFS entries, bytecode: {}",
        unpacked.metadata.hostname,
        unpacked.vfs_entries.len(),
        unpacked.bytecode.is_some()
    );

    let vfs_file_count = unpacked.vfs_entries.len();

    // Get hostname from sliver or use override
    let sliver_hostname = unpacked.metadata.hostname.clone();
    let hostname = hostname_override.unwrap_or_else(|| sliver_hostname.clone());

    if hostname != sliver_hostname {
        tracing::info!(
            "Overriding sliver hostname: '{}' -> '{}'",
            sliver_hostname,
            hostname
        );
    }

    let js_entrypoint = nano::sliver::SliverExtractor::resolve_entrypoint(&unpacked);
    tracing::info!("Sliver entrypoint: {}", js_entrypoint);

    // Create app registry with sliver data
    let mut registry = nano::app::registry::AppRegistry::default();
    let registered_hostname = registry
        .register_from_sliver(&sliver_path, None)
        .with_context(|| format!("Failed to register sliver: {}", sliver_path.display()))?;

    // Use our hostname (override or original) instead of registry's
    let _ = registered_hostname;

    let _registry = Arc::new(RwLock::new(registry));
    tracing::info!("Sliver hostname: '{}' (expected in Host header)", hostname);

    let pool_slot =
        nano::worker::SliverPoolSlot::new(hostname.clone(), workers as u32, 0, unpacked);

    tracing::info!(
        "Created SliverPoolSlot with {} workers for {}",
        workers,
        hostname
    );

    // Hot-swap on SIGHUP: re-read the sliver file and blue-green swap in a fresh,
    // warm pool without changing the hostname. In-flight requests finish on the
    // old pool; new requests route to the new one; the old pool drains and exits.
    #[cfg(unix)]
    {
        let slot = Arc::clone(&pool_slot);
        let reload_path = sliver_path.clone();
        tokio::spawn(async move {
            let mut hup =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("SIGHUP hot-reload unavailable: {}", e);
                        return;
                    }
                };
            while hup.recv().await.is_some() {
                tracing::info!(
                    "SIGHUP received — reloading sliver from {}",
                    reload_path.display()
                );
                match std::fs::read(&reload_path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        nano::sliver::unpack_sliver(&bytes).map_err(|e| e.to_string())
                    }) {
                    Ok(unpacked) => {
                        slot.hotswap_and_drain(unpacked, std::time::Duration::from_secs(30));
                        tracing::info!("Sliver hot-swapped; previous pool draining (30s)");
                        println!("  Hot-swapped sliver from {}", reload_path.display());
                    }
                    Err(e) => {
                        tracing::error!("SIGHUP reload failed, keeping current pool: {}", e);
                        println!("  Hot-reload failed (kept current version): {}", e);
                    }
                }
            }
        });
    }

    // Set up graceful shutdown
    let drain = nano::app::drain::RequestDrain::new();
    let (shutdown, mut shutdown_rx) =
        nano::signal::setup_shutdown(nano::signal::ShutdownConfig::default(), drain);
    tracing::info!("Graceful shutdown initialized");

    // Get server address
    let config = nano::http::ServerConfig::default();
    let socket_addr = config.socket_addr()?;

    // Print startup banner to console (not just tracing)
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    if static_files {
        println!("║      NANO Edge Runtime - Sliver Mode (WinterTC/Permissive) ║");
    } else {
        println!("║    NANO Edge Runtime - Sliver Mode (WinterTC/Multi-Tenant) ║");
    }
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();
    println!("  Sliver:     {}", sliver_path.display());
    println!("  Hostname:   {}", hostname);
    println!("  Address:    http://{}", socket_addr);
    println!("  Workers:    {}", workers);
    println!("  JS Entry:   {}", js_entrypoint);
    println!("  VFS Files:  {}", vfs_file_count);
    if static_files {
        println!("  Mode:       Permissive (--static) - requests to any host");
    } else {
        println!("  Mode:       Strict - 404 for wrong Host header");
    }
    println!();
    println!("  ALL requests route through JavaScript (WinterTC fetch handler)");
    println!("  Static files must be served by your JS code via nano.vfs.read()");
    println!("  Entrypoint: {}", js_entrypoint);
    println!("  Ready to accept connections...");
    println!("  Press Ctrl+C to stop");
    println!();

    tracing::info!(
        "Starting HTTP server on {} for sliver app {}",
        socket_addr,
        hostname
    );

    // Subscribe to shutdown signal for the server
    let mut server_shutdown_rx = shutdown.subscribe();
    // Clone the Arc for the server task - we'll keep one reference for shutdown
    let pool_slot_clone = Arc::clone(&pool_slot);
    // Share the shutdown drain so each sliver request is tracked in-flight.
    let server_drain = shutdown.state().drain().clone();
    let server_handle = tokio::spawn(async move {
        nano::http::start_server_with_sliver_pool(
            pool_slot_clone,
            js_entrypoint,
            config,
            server_drain,
            async move {
                let _ = server_shutdown_rx.recv().await;
            },
        )
        .await
    });

    // Wait for shutdown signal
    let _ = shutdown_rx.recv().await;

    println!();
    println!("  Shutdown signal received, stopping server...");
    println!();

    tracing::info!("Shutdown signal received, initiating graceful shutdown...");

    // Signal server to stop first
    shutdown.shutdown().await;

    // shutdown() above already drained in-flight requests (await_complete, up to
    // drain_timeout). This is the post-drain join for the server task, which by now
    // has finished serving; a short bound keeps a stuck listener from hanging exit.
    let shutdown_result =
        tokio::time::timeout(std::time::Duration::from_secs(3), server_handle).await;

    match shutdown_result {
        Ok(Ok(Ok(()))) => {
            tracing::info!("Server stopped successfully");
        }
        Ok(Ok(Err(e))) => {
            tracing::error!("Server error during shutdown: {}", e);
        }
        Ok(Err(e)) => {
            tracing::error!("Server task panicked: {}", e);
        }
        Err(_) => {
            tracing::warn!("Server shutdown timed out, forcing exit");
            println!("  Warning: Shutdown timed out, forcing exit...");
        }
    }

    // Shut down the sliver pool. The server has stopped, so dropping the slot
    // drops the current pool's task senders and its workers exit after finishing
    // any in-flight work. (A SIGHUP-retired pool, if any, drains on its own task.)
    tracing::info!("Shutting down sliver pool...");
    drop(pool_slot);

    println!("  Server stopped.");
    println!();

    tracing::info!("NANO Edge Runtime (sliver mode) shutdown complete");
    Ok(())
}

/// Run server with configuration file
///
/// Loads configuration from JSON file, creates worker pools per app,
/// and starts the HTTP server with virtual host routing.
///
/// # Arguments
///
/// * `config_path` - Path to the JSON configuration file
///
/// # Config Mode Features
///
/// - Multiple apps with virtual host routing
/// - Per-app worker pools with resource limits
/// - Server port/host from config (not hardcoded)
/// - Sliver-based app support
///
/// Note: Entrypoint-only apps use basic WorkQueue dispatch.
async fn run_server_with_config(config_path: PathBuf) -> Result<()> {
    tracing::info!("Starting NANO with config: {}", config_path.display());

    // Load and validate config
    let config = nano::config::load_config(&config_path)
        .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

    tracing::info!("Loaded configuration with {} app(s)", config.apps.len());

    // Apply security settings before any worker threads are created.
    nano::runtime::fetch::set_dns_rebinding_protection(config.server.dns_rebinding_protection);
    if !config.server.dns_rebinding_protection {
        tracing::warn!(
            "DNS rebinding protection DISABLED — tenant JS can reach internal network \
             via domains that TTL-rotate to private IPs"
        );
    } else {
        tracing::info!("DNS rebinding protection: enabled");
    }

    // Apply worker queue and mtime cache settings.
    nano::worker::pool::set_queue_depth_per_worker(config.server.queue_depth_per_worker);
    nano::worker::pool::set_handler_mtime_cache_ttl_ms(config.server.handler_cache_refresh_ms);
    tracing::info!(
        "Worker queue depth: {} tasks/worker, mtime cache TTL: {}ms",
        config.server.queue_depth_per_worker,
        config.server.handler_cache_refresh_ms,
    );

    // Check for sliver-based apps in config
    let has_sliver_apps = config.apps.iter().any(|app| app.sliver.is_some());
    let has_entrypoint_apps = config
        .apps
        .iter()
        .any(|app| app.sliver.is_none() && !app.entrypoint.is_empty());

    if has_sliver_apps {
        tracing::info!("Configuration contains sliver-based apps");
    }
    if has_entrypoint_apps {
        tracing::info!("Configuration contains entrypoint-based apps");
    }

    // Initialize V8 platform (once per process)
    nano::v8::platform::initialize_platform().context("Failed to initialize V8 platform")?;
    tracing::info!("V8 platform initialized");

    // Create app registry from config
    let _registry = Arc::new(tokio::sync::RwLock::new(
        nano::app::registry::AppRegistry::from_config(config.clone()),
    ));
    tracing::info!("Created AppRegistry");

    // Set up graceful shutdown
    let drain = nano::app::drain::RequestDrain::new();
    let (shutdown, mut shutdown_rx) =
        nano::signal::setup_shutdown(nano::signal::ShutdownConfig::default(), drain);
    tracing::info!("Graceful shutdown initialized");

    // Convert server config section to ServerConfig
    let server_bind_config = nano::http::ServerConfig::from(config.server.clone());
    let addr = server_bind_config
        .socket_addr()
        .context("Failed to parse server address")?;

    tracing::info!("Starting HTTP server on {}", addr);

    // Print startup banner
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║          NANO Edge Runtime - Config Mode (Multi-App)        ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();
    println!("  Config:     {}", config_path.display());
    println!("  Address:    http://{}", addr);
    println!("  Apps:       {}", config.apps.len());
    println!();

    // Display app information
    for app in &config.apps {
        let app_type = if app.sliver.is_some() {
            "sliver"
        } else {
            "entrypoint"
        };
        println!("  - {} ({})", app.hostname, app_type);
        println!(
            "    Workers: {}, Memory: {}MB, Timeout: {}s",
            app.limits.workers, app.limits.memory_mb, app.limits.timeout_secs
        );
    }

    println!();
    println!("  Ready to accept connections...");
    println!("  Press Ctrl+C to stop");
    println!();

    // Start server with config
    let shutdown_state = shutdown.state().clone();
    let server_handle =
        tokio::spawn(
            async move { nano::http::start_server_with_config(config, shutdown_state).await },
        );

    // Wait for shutdown signal
    let _ = shutdown_rx.recv().await;

    println!();
    println!("  Shutdown signal received, stopping server...");
    println!();

    tracing::info!("Shutdown signal received, initiating graceful shutdown...");

    // Perform graceful shutdown
    shutdown.shutdown().await;

    // shutdown() above already drained in-flight requests (await_complete, up to
    // drain_timeout). This is the post-drain join for the server task, which by now
    // has finished serving; a short bound keeps a stuck listener from hanging exit.
    let shutdown_result =
        tokio::time::timeout(std::time::Duration::from_secs(3), server_handle).await;

    match shutdown_result {
        Ok(Ok(Ok(()))) => {
            tracing::info!("Server stopped successfully");
        }
        Ok(Ok(Err(e))) => {
            tracing::error!("Server error during shutdown: {}", e);
        }
        Ok(Err(e)) => {
            tracing::error!("Server task panicked: {}", e);
        }
        Err(_) => {
            tracing::warn!("Server shutdown timed out, forcing exit");
            println!("  Warning: Shutdown timed out, forcing exit...");
        }
    }

    tracing::info!("NANO config mode shutdown complete");
    Ok(())
}

/// Handle sliver management commands
async fn handle_sliver_command(cmd: cli::SliverCommand) -> Result<()> {
    use cli::validation::{validate_hostname, validate_sliver_name, validate_tag};
    use nano::sliver::packager::create_sliver_from_directory;
    use nano::sliver::{unpack_sliver, validate_sliver, SliverMetadata};

    // Initialize V8 platform if not already done (required for snapshot operations)
    if !nano::v8::platform::is_initialized() {
        nano::v8::platform::initialize_platform()
            .context("Failed to initialize V8 platform for sliver command")?;
        tracing::info!("V8 platform initialized for sliver command");
    }

    match cmd {
        cli::SliverCommand::Create(args) => {
            // Check if we're creating from a directory
            if let Some(from_dir) = args.from_dir {
                tracing::info!("Creating sliver from directory: {}", from_dir.display());

                // Get name - required for directory-based slivers
                let name = args
                    .name
                    .ok_or_else(|| anyhow::anyhow!("--name is required when using --from-dir"))?;

                // Validate name
                validate_sliver_name(&name).map_err(|e| anyhow::anyhow!("{}", e))?;

                // Validate tag if provided
                if let Some(ref tag) = args.tag {
                    validate_tag(tag).map_err(|e| anyhow::anyhow!("{}", e))?;
                }

                // Get hostname from args or use name as default
                let hostname = args.hostname.or_else(|| Some(name.clone()));

                // Create output path
                let output = args.output.map(|p| p.to_string_lossy().to_string());

                // Create the sliver from directory
                create_sliver_from_directory(
                    from_dir.to_str().unwrap_or("."),
                    &name,
                    args.tag,
                    output,
                    hostname,
                    args.source_only,
                )
                .await?;

                return Ok(());
            }

            // Original hostname-based sliver creation
            let hostname = args.hostname.ok_or_else(|| {
                anyhow::anyhow!("Either hostname or --from-dir must be specified")
            })?;

            tracing::info!("Creating sliver for hostname: {}", hostname);

            // Validate hostname
            validate_hostname(&hostname).map_err(|e| anyhow::anyhow!("{}", e))?;

            // Validate optional name
            if let Some(ref name) = args.name {
                validate_sliver_name(name).map_err(|e| anyhow::anyhow!("{}", e))?;
            }

            // Validate optional tag
            if let Some(ref tag) = args.tag {
                validate_tag(tag).map_err(|e| anyhow::anyhow!("{}", e))?;
            }

            // Determine output path
            let output = args.output.unwrap_or_else(|| {
                let name = args.name.as_ref().unwrap_or(&hostname);
                let tag = args
                    .tag
                    .as_ref()
                    .map(|t| format!("-{}", t))
                    .unwrap_or_default();
                PathBuf::from(format!("{}{}.sliver", name, tag))
            });

            // Validate output path doesn't already exist
            if output.exists() {
                anyhow::bail!(
                    "Sliver file already exists: {}. Use --output to specify a different path.",
                    output.display()
                );
            }

            let sliver_name = args.name.clone();
            let sliver_tag = args.tag.clone();

            // Create metadata
            let mut metadata = SliverMetadata::new(&hostname, env!("CARGO_PKG_VERSION"));
            metadata.name = sliver_name.clone();
            if let Some(tag) = sliver_tag {
                metadata.description = Some(format!("Tag: {}", tag));
            }

            let source_only = args.source_only;

            nano::sliver::packager::create_sliver_from_directory(
                ".",
                sliver_name.as_deref().unwrap_or(&hostname),
                args.tag.clone(),
                Some(output.to_string_lossy().to_string()),
                Some(hostname.clone()),
                source_only,
            )
            .await
            .context("Failed to create sliver")?;

            // Validate the created sliver
            let archive_data = std::fs::read(&output)
                .with_context(|| format!("Failed to read created sliver: {}", output.display()))?;
            validate_sliver(&archive_data).context("Created sliver failed validation")?;

            tracing::info!("Sliver created successfully: {}", output.display());

            Ok(())
        }
        cli::SliverCommand::List(args) => {
            tracing::info!("Listing slivers");

            // Find all .sliver files in current directory
            let mut found = false;
            let entries = std::fs::read_dir(".").context("Failed to read current directory")?;

            println!("Slivers:");
            for entry in entries {
                let entry = entry?;
                let path = entry.path();

                if path.extension().and_then(|e| e.to_str()) == Some("sliver") {
                    found = true;
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown");

                    if args.verbose {
                        // Try to read metadata from the sliver
                        match std::fs::read(&path) {
                            Ok(data) => match unpack_sliver(&data) {
                                Ok(unpacked) => {
                                    println!("  {} ({} bytes)", name, data.len());
                                    for line in unpacked.metadata.summary().lines() {
                                        println!("    {}", line);
                                    }
                                }
                                Err(e) => {
                                    println!("  {} ({} bytes) [invalid: {}]", name, data.len(), e);
                                }
                            },
                            Err(e) => {
                                println!("  {} [error reading: {}]", name, e);
                            }
                        }
                    } else {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        println!("  {} ({} bytes)", name, size);
                    }
                }
            }

            if !found {
                println!("  (none found in current directory)");
            }

            Ok(())
        }
        cli::SliverCommand::Delete(args) => {
            tracing::info!("Deleting sliver: {}", args.name);

            // Validate sliver name
            validate_sliver_name(&args.name).map_err(|e| anyhow::anyhow!("{}", e))?;

            // Look for sliver file with the given name
            let sliver_path = PathBuf::from(format!("{}.sliver", args.name));

            if !sliver_path.exists() {
                anyhow::bail!("Sliver not found: {}", args.name);
            }

            if !args.force {
                print!("Delete sliver '{}'? [y/N] ", args.name);
                use std::io::Write;
                std::io::stdout().flush()?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Deletion cancelled.");
                    return Ok(());
                }
            }

            std::fs::remove_file(&sliver_path)
                .with_context(|| format!("Failed to delete sliver: {}", args.name))?;

            println!("Deleted sliver: {}", args.name);
            tracing::info!("Sliver deleted: {}", sliver_path.display());

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_run() {
        let cli = Cli::try_parse_from(["nano-rs", "run"]);
        assert!(cli.is_ok());

        if let Ok(Cli {
            command: Some(Commands::Run { .. }),
        }) = cli
        {
            // Parsed correctly
        } else {
            panic!("Expected Run command");
        }
    }

    #[test]
    fn test_cli_parse_sliver_create() {
        let cli = Cli::try_parse_from([
            "nano-rs",
            "sliver",
            "create",
            "api.example.com",
            "--output",
            "./test.sliver",
        ]);
        assert!(cli.is_ok());

        if let Ok(Cli {
            command: Some(Commands::Sliver(cmd)),
        }) = cli
        {
            match cmd {
                cli::SliverCommand::Create(args) => {
                    assert_eq!(args.hostname, Some("api.example.com".to_string()));
                    assert_eq!(args.output, Some(PathBuf::from("./test.sliver")));
                }
                _ => panic!("Expected Create command"),
            }
        } else {
            panic!("Expected Sliver command");
        }
    }

    #[test]
    fn test_cli_parse_sliver_list() {
        let cli = Cli::try_parse_from(["nano-rs", "sliver", "list"]);
        assert!(cli.is_ok());

        if let Ok(Cli {
            command: Some(Commands::Sliver(cmd)),
        }) = cli
        {
            match cmd {
                cli::SliverCommand::List(args) => {
                    assert!(!args.verbose);
                }
                _ => panic!("Expected List command"),
            }
        } else {
            panic!("Expected Sliver command");
        }
    }

    #[test]
    fn test_cli_parse_sliver_delete() {
        let cli = Cli::try_parse_from(["nano-rs", "sliver", "delete", "test-sliver"]);
        assert!(cli.is_ok());

        if let Ok(Cli {
            command: Some(Commands::Sliver(cmd)),
        }) = cli
        {
            match cmd {
                cli::SliverCommand::Delete(args) => {
                    assert_eq!(args.name, "test-sliver");
                    assert!(!args.force);
                }
                _ => panic!("Expected Delete command"),
            }
        } else {
            panic!("Expected Sliver command");
        }
    }

    #[test]
    fn test_cli_default_to_run() {
        // Default behavior (no subcommand) should run server
        let cli = Cli::try_parse_from(["nano-rs"]);
        assert!(cli.is_ok());
        assert!(cli.unwrap().command.is_none());
    }
}
// force rebuild
