//! CVE Scanner Tool
//!
//! Automated CVE scanning for Rust dependencies using cargo-audit.
//! Provides CI-friendly output formats and caching support.
//!
//! Usage:
//!   cargo run --bin cve-scanner -- --format json --severity critical
//!   cargo run --bin cve-scanner -- --update-db
//!   cargo run --bin cve-scanner -- --offline

use std::env;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    // Check if cargo-audit is available
    let audit_check = Command::new("cargo").args(&["audit", "--version"]).output();

    match audit_check {
        Ok(output) if output.status.success() => {
            // cargo-audit is available
        }
        _ => {
            eprintln!("Error: cargo-audit is not installed.");
            eprintln!("Install with: cargo install cargo-audit");
            return ExitCode::from(2);
        }
    }

    // Default command: run audit
    let mut cmd = Command::new("cargo");
    cmd.arg("audit");

    // Parse arguments
    let mut format = "human"; // human, json, toml
    let mut severity = None;
    let mut offline = false;
    let mut update_db = false;
    let mut deny_warnings = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if i + 1 < args.len() {
                    format = &args[i + 1];
                    i += 2;
                } else {
                    eprintln!("Error: --format requires an argument");
                    return ExitCode::from(2);
                }
            }
            "--severity" => {
                if i + 1 < args.len() {
                    severity = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --severity requires an argument");
                    return ExitCode::from(2);
                }
            }
            "--offline" => {
                offline = true;
                i += 1;
            }
            "--update-db" => {
                update_db = true;
                i += 1;
            }
            "--deny-warnings" => {
                deny_warnings = true;
                i += 1;
            }
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                print_help();
                return ExitCode::from(2);
            }
        }
    }

    // Update database if requested
    if update_db {
        println!("Updating cargo-audit database...");
        let update_result = Command::new("cargo").args(&["audit", "--update"]).status();

        match update_result {
            Ok(status) if status.success() => {
                println!("Database updated successfully.");
            }
            _ => {
                eprintln!("Warning: Failed to update database. Using cached version.");
            }
        }
    }

    // Build audit command
    if offline {
        cmd.arg("--no-update");
    }

    if deny_warnings {
        cmd.arg("--deny").arg("warnings");
    }

    // Apply format
    match format {
        "json" => cmd.arg("--json"),
        _ => &mut cmd, // human format is default
    };

    // Apply severity filter if specified
    if let Some(sev) = severity {
        cmd.arg("--severity").arg(&sev);
    }

    // Run audit
    println!("Running CVE scan...");
    match cmd.status() {
        Ok(status) => {
            if status.success() {
                println!("✅ No CVEs found.");
                ExitCode::SUCCESS
            } else {
                // CVEs found - this is informational, not an error
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("Error running cargo audit: {}", e);
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!("CVE Scanner for NANO Runtime");
    println!();
    println!("Usage: cargo run --bin cve-scanner -- [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --format <FORMAT>     Output format: human, json [default: human]");
    println!("  --severity <LEVEL>    Minimum severity: critical, high, medium, low");
    println!("  --offline             Use cached database only");
    println!("  --update-db           Update vulnerability database before scan");
    println!("  --deny-warnings       Treat warnings as failures (CI mode)");
    println!("  --help, -h            Print this help message");
    println!();
    println!("Exit codes:");
    println!("  0  - No CVEs found");
    println!("  1  - CVEs found (informational)");
    println!("  2  - Error (tool not available, etc.)");
    println!();
    println!("Examples:");
    println!("  cargo run --bin cve-scanner -- --format json");
    println!("  cargo run --bin cve-scanner -- --severity critical");
    println!("  cargo run --bin cve-scanner -- --update-db --deny-warnings");
}
