//! CLI Sliver Commands
//!
//! Provides subcommands for managing slivers (isolate snapshots):
//! - `create`: Create a new sliver from a running app or directory
//! - `list`: List all slivers in the store
//! - `delete`: Remove a sliver by name

use clap::{Args, Subcommand};
use std::path::PathBuf;

/// Sliver management commands
#[derive(Debug, Subcommand)]
pub enum SliverCommand {
    /// Create a new sliver from a running app or directory
    Create(SliverCreateArgs),

    /// List all slivers
    List(SliverListArgs),

    /// Delete a sliver by name
    Delete(SliverDeleteArgs),
}

/// Arguments for the `sliver create` command
#[derive(Debug, Args)]
pub struct SliverCreateArgs {
    /// Hostname of the app to snapshot (optional when using --from-dir)
    pub hostname: Option<String>,

    /// Output file path for the sliver
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Name for the sliver (defaults to hostname if not specified)
    #[arg(short, long)]
    pub name: Option<String>,

    /// Tag/version for the sliver
    #[arg(short, long)]
    pub tag: Option<String>,

    /// Create sliver from directory (no running app required)
    #[arg(long, value_name = "DIR")]
    pub from_dir: Option<PathBuf>,

    /// Skip bytecode compilation; pack JS source only (portable across V8 versions)
    #[arg(long, default_value = "false")]
    pub source_only: bool,
}

/// Arguments for the `sliver list` command
#[derive(Debug, Args)]
pub struct SliverListArgs {
    /// Show detailed information
    #[arg(short, long)]
    pub verbose: bool,
}

/// Arguments for the `sliver delete` command
#[derive(Debug, Args)]
pub struct SliverDeleteArgs {
    /// Name of the sliver to delete
    pub name: String,

    /// Force deletion without confirmation
    #[arg(short, long)]
    pub force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Command, Parser};

    fn parse_sliver_command(args: &[&str]) -> Result<SliverCommand, clap::Error> {
        // Create a test command that includes just the sliver subcommand
        // Note: args should NOT include "sliver" since we're testing SliverCommand directly
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(subcommand)]
            command: SliverCommand,
        }

        TestCli::try_parse_from(std::iter::once("nano-rs").chain(args.iter().copied()))
            .map(|cli| cli.command)
    }

    #[test]
    fn test_parse_create_with_hostname() {
        // Test without "sliver" prefix since we're testing SliverCommand directly
        let result = parse_sliver_command(&["create", "api.example.com"]);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        if let Ok(SliverCommand::Create(args)) = result {
            assert_eq!(args.hostname, Some("api.example.com".to_string()));
            assert!(args.output.is_none());
            assert!(args.name.is_none());
            assert!(args.tag.is_none());
            assert!(args.from_dir.is_none());
        }
    }

    #[test]
    fn test_parse_create_with_output() {
        let result =
            parse_sliver_command(&["create", "api.example.com", "--output", "./api-v1.sliver"]);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        if let Ok(SliverCommand::Create(args)) = result {
            assert_eq!(args.hostname, Some("api.example.com".to_string()));
            assert_eq!(args.output, Some(PathBuf::from("./api-v1.sliver")));
        }
    }

    #[test]
    fn test_parse_create_with_name_and_tag() {
        let result = parse_sliver_command(&[
            "create",
            "api.example.com",
            "--name",
            "api-prod",
            "--tag",
            "v1.0",
        ]);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        if let Ok(SliverCommand::Create(args)) = result {
            assert_eq!(args.hostname, Some("api.example.com".to_string()));
            assert_eq!(args.name, Some("api-prod".to_string()));
            assert_eq!(args.tag, Some("v1.0".to_string()));
        }
    }

    #[test]
    fn test_parse_create_from_dir() {
        let result = parse_sliver_command(&[
            "create",
            "--from-dir",
            "./dist",
            "--name",
            "myapp",
            "--tag",
            "v1.0",
        ]);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        if let Ok(SliverCommand::Create(args)) = result {
            assert_eq!(args.hostname, None);
            assert_eq!(args.from_dir, Some(PathBuf::from("./dist")));
            assert_eq!(args.name, Some("myapp".to_string()));
            assert_eq!(args.tag, Some("v1.0".to_string()));
        }
    }

    #[test]
    fn test_parse_create_from_dir_with_hostname() {
        // Test that --from-dir can be combined with hostname
        let result = parse_sliver_command(&[
            "create",
            "app.example.com",
            "--from-dir",
            "./dist",
            "--name",
            "myapp",
        ]);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        if let Ok(SliverCommand::Create(args)) = result {
            assert_eq!(args.hostname, Some("app.example.com".to_string()));
            assert_eq!(args.from_dir, Some(PathBuf::from("./dist")));
        }
    }
}
