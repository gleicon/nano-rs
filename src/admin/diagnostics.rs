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
    /// When the isolate was created
    pub created_at: Instant,
    /// Number of requests processed
    pub request_count: u64,
    /// Current memory usage (if available)
    pub memory_bytes: Option<usize>,
    /// Whether the isolate is currently processing a request
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

/// Diagnostics collector for runtime state
pub struct DiagnosticsCollector {
    registry: Arc<RwLock<AppRegistry>>,
    // In real implementation, this would track worker pools
    // For now, we'll simulate with test data
}

impl DiagnosticsCollector {
    /// Create a new diagnostics collector
    pub fn new(registry: Arc<RwLock<AppRegistry>>) -> Self {
        Self { registry }
    }

    /// Collect current system diagnostics
    pub async fn collect(&self) -> SystemDiagnostics {
        let registry = self.registry.read().await;
        let mut isolates = Vec::new();
        let mut app_stats = Vec::new();

        // Collect per-app information
        for hostname in registry.all_hostnames() {
            if let Some(app_config) = registry.get(&hostname) {
                // In real implementation, query worker pools here
                // For test/demo, create simulated isolate info
                let worker_count = app_config.limits.workers;

                for worker_id in 0..worker_count {
                    isolates.push(IsolateInfo {
                        hostname: hostname.clone(),
                        worker_id,
                        created_at: Instant::now() - Duration::from_secs(60), // Simulated
                        request_count: 42 + (worker_id as u64 * 10),          // Simulated
                        memory_bytes: Some({
                            let mb = app_config.limits.memory_mb;
                            (mb as usize) * 1024 * 1024 / 4
                        }),
                        busy: worker_id % 2 == 0, // Simulated: alternating busy/idle
                        env_keys: app_config.env_vars.keys().cloned().collect(),
                    });
                }

                app_stats.push(AppStats {
                    hostname: hostname.clone(),
                    worker_count,
                    total_requests: isolates
                        .iter()
                        .filter(|i| i.hostname == hostname)
                        .map(|i| i.request_count)
                        .sum(),
                    avg_memory_mb: app_config.limits.memory_mb as f64 * 0.25,
                    uptime: format_duration(Duration::from_secs(60)),
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
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("very long string", 10), "very lo...");
    }
}
