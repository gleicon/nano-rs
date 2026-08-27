//! Admin diagnostics for multi-app monitoring
//!
//! Provides visibility into active isolates, worker pools, and app health.
//! Similar to `ps` or `top` for NANO isolates.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::app::registry::AppRegistry;

/// Runtime information about an active isolate
#[derive(Debug, Clone)]
pub struct IsolateInfo {
    /// Hostname this isolate serves
    pub hostname: String,
    /// Worker thread ID
    pub worker_id: u32,
    /// When the isolate was created (from live worker telemetry).
    pub created_at: Instant,
    /// Requests processed by this isolate (live).
    pub request_count: u64,
    /// Last-observed V8 used-heap bytes; `None` until the first request.
    pub memory_bytes: Option<usize>,
    /// Whether the isolate is currently processing a request (live).
    pub busy: bool,
    /// App-specific environment variables (keys only, for privacy)
    pub env_keys: Vec<String>,
}

impl IsolateInfo {
    /// Get uptime as a human-readable string
    pub fn uptime(&self) -> String {
        let elapsed = self.created_at.elapsed();
        format_duration(elapsed)
    }
}

/// App-level aggregate statistics
#[derive(Debug, Clone)]
pub struct AppStats {
    /// Hostname
    pub hostname: String,
    /// Number of active workers
    pub worker_count: u32,
    /// Total requests served
    pub total_requests: u64,
    /// Average memory per isolate
    pub avg_memory_mb: f64,
    /// Uptime of the oldest isolate
    pub uptime: String,
    /// Current configuration
    pub config: AppConfigSnapshot,
}

/// Snapshot of app configuration
#[derive(Debug, Clone)]
pub struct AppConfigSnapshot {
    pub memory_limit_mb: u32,
    pub timeout_secs: u32,
    pub workers: u32,
}

/// System-wide diagnostics snapshot
#[derive(Debug, Clone)]
pub struct SystemDiagnostics {
    /// Timestamp of the snapshot
    pub timestamp: Instant,
    /// All active isolates
    pub isolates: Vec<IsolateInfo>,
    /// Per-app statistics
    pub app_stats: Vec<AppStats>,
    /// Total isolates across all apps
    pub total_isolates: usize,
    /// Total requests since startup
    pub total_requests: u64,
}

impl SystemDiagnostics {
    /// Format as human-readable text (like `ps` output)
    pub fn format_ps(&self) -> String {
        let mut output = String::new();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let datetime = format!("{} (Unix timestamp: {})", humantime(timestamp), timestamp);

        output.push_str(&format!("NANO Multi-App Runtime - {}", datetime));
        output.push('\n');
        output.push_str(&format!(
            "Total isolates: {} | Total requests: {}",
            self.total_isolates, self.total_requests
        ));
        output.push('\n');
        output.push_str(&"-".repeat(100));
        output.push('\n');

        // Header
        output.push_str(&format!(
            "{:<20} {:<8} {:<10} {:<15} {:<12} {:<10} {}\n",
            "HOSTNAME", "WORKER", "STATUS", "UPTIME", "REQUESTS", "MEMORY", "ENV_KEYS"
        ));
        output.push_str(&"-".repeat(100));
        output.push('\n');

        // Per-isolate info
        for isolate in &self.isolates {
            let status = if isolate.busy { "BUSY" } else { "IDLE" };
            let memory = isolate
                .memory_bytes
                .map(|b| format!("{:.1}MB", b as f64 / 1024.0 / 1024.0))
                .unwrap_or_else(|| "-".to_string());
            let env_summary = if isolate.env_keys.is_empty() {
                "-".to_string()
            } else {
                format!("{} vars", isolate.env_keys.len())
            };

            output.push_str(&format!(
                "{:<20} {:<8} {:<10} {:<15} {:<12} {:<10} {}\n",
                truncate(&isolate.hostname, 20),
                isolate.worker_id,
                status,
                isolate.uptime(),
                isolate.request_count,
                memory,
                env_summary
            ));
        }

        output.push('\n');

        // App-level summary
        output.push_str("App Summary:\n");
        output.push_str(&"-".repeat(80));
        output.push('\n');
        output.push_str(&format!(
            "{:<20} {:<8} {:<12} {:<15} {:<10}\n",
            "HOSTNAME", "WORKERS", "REQUESTS", "UPTIME", "LIMITS"
        ));
        output.push_str(&"-".repeat(80));
        output.push('\n');

        for app in &self.app_stats {
            let limits = format!(
                "{}MB/{}s/{}w",
                app.config.memory_limit_mb, app.config.timeout_secs, app.config.workers
            );

            output.push_str(&format!(
                "{:<20} {:<8} {:<12} {:<15} {:<10}\n",
                truncate(&app.hostname, 20),
                app.worker_count,
                app.total_requests,
                app.uptime,
                limits
            ));
        }

        output
    }

    /// Format as JSON for API consumption
    pub fn format_json(&self) -> String {
        use serde_json::json;

        let apps: Vec<_> = self
            .app_stats
            .iter()
            .map(|app| {
                json!({
                    "hostname": app.hostname,
                    "workers": app.worker_count,
                    "total_requests": app.total_requests,
                    "memory_limit_mb": app.config.memory_limit_mb,
                    "timeout_secs": app.config.timeout_secs,
                    "uptime": app.uptime,
                })
            })
            .collect();

        json!({
            "total_isolates": self.total_isolates,
            "total_requests": self.total_requests,
            "app_count": self.app_stats.len(),
            "apps": apps,
        })
        .to_string()
    }
}

/// Diagnostics collector for runtime state.
///
/// Reads app configuration from the registry and live per-isolate stats from
/// [`crate::worker::telemetry`], which the worker threads publish.
pub struct DiagnosticsCollector {
    registry: Arc<RwLock<AppRegistry>>,
}

impl DiagnosticsCollector {
    /// Create a new diagnostics collector
    pub fn new(registry: Arc<RwLock<AppRegistry>>) -> Self {
        Self { registry }
    }

    /// Collect current system diagnostics.
    ///
    /// Per-isolate runtime stats (request count, busy, used heap, creation time)
    /// come from live worker telemetry ([`crate::worker::telemetry`]); app-level
    /// configuration (limits) comes from the registry. Isolates are created
    /// lazily on first request, so an app with no traffic reports zero isolates.
    pub async fn collect(&self) -> SystemDiagnostics {
        let registry = self.registry.read().await;

        // Real, live per-isolate stats published by the worker threads.
        let mut isolates: Vec<IsolateInfo> = crate::worker::telemetry::snapshot()
            .into_iter()
            .map(|s| IsolateInfo {
                hostname: s.hostname,
                worker_id: s.worker_id,
                created_at: s.created_at,
                request_count: s.request_count,
                memory_bytes: s.memory_bytes,
                busy: s.busy,
                env_keys: s.env_keys,
            })
            .collect();
        // Stable ordering for a predictable ps-style listing.
        isolates.sort_by(|a, b| {
            a.hostname
                .cmp(&b.hostname)
                .then(a.worker_id.cmp(&b.worker_id))
        });

        // Per-app stats: aggregate the app's live isolates, plus configured limits.
        let mut app_stats = Vec::new();
        for hostname in registry.all_hostnames() {
            if let Some(app_config) = registry.get(&hostname) {
                let app_isolates: Vec<&IsolateInfo> =
                    isolates.iter().filter(|i| i.hostname == hostname).collect();

                let total_requests: u64 = app_isolates.iter().map(|i| i.request_count).sum();

                let mem_samples: Vec<usize> =
                    app_isolates.iter().filter_map(|i| i.memory_bytes).collect();
                let avg_memory_mb = if mem_samples.is_empty() {
                    0.0
                } else {
                    (mem_samples.iter().sum::<usize>() as f64 / mem_samples.len() as f64)
                        / (1024.0 * 1024.0)
                };

                // App uptime ≈ how long its oldest live isolate has existed.
                let uptime = app_isolates
                    .iter()
                    .map(|i| i.created_at)
                    .min()
                    .map(|created| format_duration(created.elapsed()))
                    .unwrap_or_else(|| "0s".to_string());

                app_stats.push(AppStats {
                    hostname: hostname.clone(),
                    worker_count: app_config.limits.workers,
                    total_requests,
                    avg_memory_mb,
                    uptime,
                    config: AppConfigSnapshot {
                        memory_limit_mb: app_config.limits.memory_mb,
                        timeout_secs: app_config.limits.timeout_secs,
                        workers: app_config.limits.workers,
                    },
                });
            }
        }

        let total_requests = app_stats.iter().map(|a| a.total_requests).sum();
        let total_isolates = isolates.len();

        SystemDiagnostics {
            timestamp: Instant::now(),
            isolates,
            app_stats,
            total_isolates,
            total_requests,
        }
    }
}

/// Format duration as human-readable string
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Truncate string to max length with ellipsis
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len - 3])
    } else {
        s.to_string()
    }
}

/// Simple human-readable timestamp (shows seconds as-is, for brevity)
fn humantime(unix_secs: u64) -> String {
    // For simplicity in tests, just return the raw timestamp
    // In production, you'd use chrono or time crate
    format!("{}s since epoch", unix_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        // Boundaries pin the `<` comparisons (59/60 and 3599/3600).
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
        assert_eq!(format_duration(Duration::from_secs(60)), "1m 0s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59m 59s");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h 0m");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        // Exactly max_len must NOT truncate — pins `>` vs `>=`.
        assert_eq!(truncate("exactly10!", 10), "exactly10!");
        assert_eq!(truncate("very long string", 10), "very lo...");
    }

    #[test]
    fn test_humantime_includes_timestamp() {
        assert_eq!(humantime(1700000000), "1700000000s since epoch");
    }

    #[test]
    fn test_isolate_uptime_formats_elapsed() {
        let info = IsolateInfo {
            hostname: "h".to_string(),
            worker_id: 0,
            created_at: Instant::now() - Duration::from_secs(65),
            request_count: 0,
            memory_bytes: None,
            busy: false,
            env_keys: vec![],
        };
        // ~65s elapsed → "1m Xs"; pins uptime() delegating to format_duration.
        assert!(
            info.uptime().starts_with("1m"),
            "uptime was {}",
            info.uptime()
        );
    }

    fn sample_diagnostics() -> SystemDiagnostics {
        SystemDiagnostics {
            timestamp: Instant::now(),
            isolates: vec![IsolateInfo {
                hostname: "api.example.com".to_string(),
                worker_id: 3,
                created_at: Instant::now(),
                request_count: 128,
                memory_bytes: Some(45 * 1024 * 1024),
                busy: true,
                env_keys: vec!["API_KEY".to_string()],
            }],
            app_stats: vec![AppStats {
                hostname: "api.example.com".to_string(),
                worker_count: 2,
                total_requests: 128,
                avg_memory_mb: 45.0,
                uptime: "5m 0s".to_string(),
                config: AppConfigSnapshot {
                    memory_limit_mb: 256,
                    timeout_secs: 30,
                    workers: 2,
                },
            }],
            total_isolates: 1,
            total_requests: 128,
        }
    }

    #[test]
    fn format_ps_contains_real_fields() {
        let out = sample_diagnostics().format_ps();
        // The ps-style output must carry the real values, not empties.
        assert!(out.contains("api.example.com"), "hostname: {out}");
        assert!(out.contains("128"), "request count present");
        assert!(out.contains("BUSY"), "busy status rendered");
        assert!(out.contains("Total isolates: 1"), "summary line");
        assert!(out.contains("45.0MB"), "real memory rendered");
        assert!(out.contains("1 vars"), "env-var count rendered");
    }

    #[test]
    fn format_json_has_real_values() {
        let out = sample_diagnostics().format_json();
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v["total_isolates"], 1);
        assert_eq!(v["total_requests"], 128);
        assert_eq!(v["app_count"], 1);
        assert_eq!(v["apps"][0]["hostname"], "api.example.com");
        assert_eq!(v["apps"][0]["total_requests"], 128);
        assert_eq!(v["apps"][0]["memory_limit_mb"], 256);
    }

    /// The collector must surface real live telemetry (not fabricated data) and
    /// aggregate it per app.
    #[tokio::test]
    async fn collect_reflects_live_telemetry() {
        use crate::app::registry::AppRegistry;
        use crate::config::{AppConfig, AppLimits};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        // Unique hostname so this test is isolated from any other live entries.
        let hostname = format!("diag-test-{}.local", std::process::id());

        let mut apps = HashMap::new();
        apps.insert(
            hostname.clone(),
            AppConfig {
                hostname: hostname.clone(),
                entrypoint: "./app.js".to_string(),
                sliver: None,
                compat: Default::default(),
                env_vars: HashMap::new(),
                limits: AppLimits::default(),
                vfs_backend: Default::default(),
                vfs_disk: None,
                vfs_s3: None,
            },
        );
        let registry = Arc::new(RwLock::new(AppRegistry::new(apps)));
        let collector = DiagnosticsCollector::new(registry);

        // No live isolates yet → app present but zero isolates.
        let before = collector.collect().await;
        assert_eq!(
            before
                .isolates
                .iter()
                .filter(|i| i.hostname == hostname)
                .count(),
            0,
            "no isolates before any worker registers"
        );

        // Register two live isolates and record traffic.
        let id0 = format!("iso_{}_0", std::process::id());
        let id1 = format!("iso_{}_1", std::process::id());
        let _g0 =
            crate::worker::telemetry::register_isolate(id0.clone(), hostname.clone(), 0, vec![]);
        let _g1 =
            crate::worker::telemetry::register_isolate(id1.clone(), hostname.clone(), 1, vec![]);
        crate::worker::telemetry::record_request(&id0, 4 * 1024 * 1024);
        crate::worker::telemetry::record_request(&id0, 4 * 1024 * 1024);
        crate::worker::telemetry::record_request(&id1, 8 * 1024 * 1024);
        crate::worker::telemetry::mark_busy(&id1, true);

        let after = collector.collect().await;
        let mine: Vec<_> = after
            .isolates
            .iter()
            .filter(|i| i.hostname == hostname)
            .collect();
        assert_eq!(mine.len(), 2, "two live isolates surfaced");
        assert_eq!(
            mine.iter().map(|i| i.request_count).sum::<u64>(),
            3,
            "real aggregated request count"
        );
        assert!(mine.iter().any(|i| i.busy), "busy flag reflected");
        assert!(
            mine.iter().all(|i| i.memory_bytes.is_some()),
            "real memory recorded"
        );

        let app = after
            .app_stats
            .iter()
            .find(|a| a.hostname == hostname)
            .expect("app stats present");
        assert_eq!(app.total_requests, 3);
        // id0=4 MiB, id1=8 MiB → mean 6.0 MiB. Exact value pins the averaging
        // arithmetic (sum/len then /1MiB), not just "> 0".
        assert_eq!(
            app.avg_memory_mb, 6.0,
            "avg memory = mean of live samples in MiB"
        );
    }
}
