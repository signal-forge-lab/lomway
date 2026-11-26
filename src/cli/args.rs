//! CLI argument definitions.
//!
//! Contract (binary name: `lomway`):
//!
//! ```text
//! lomway serve         --config <path>
//! lomway check         --config <path> [--probe]
//! lomway list-backends --config <path>
//! lomway migrate       --config <path> --output <path>
//! lomway catalog       --config <path> [--format text|json|names]
//!                      [--backend <id>]... [--output <path>]
//!                      [--diff <snapshot>] [--fail-on-change]
//! lomway version
//! ```
//!
//! Portability: the binary alone implements every core mode on every
//! platform — no PowerShell (or any shell) is required. Exit codes are
//! actionable: `0` success, `1` failure, `2` usage error (clap default).

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// `lomway` command line.
#[derive(Debug, Parser)]
#[command(
    name = "lomway",
    version,
    about = "Lomway local MCP aggregation gateway"
)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate startup semantics and serve MCP on the loopback listener.
    Serve {
        /// Configuration file (see `check` for resolution precedence).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Validate parse, policy, and collision preflight without serving.
    ///
    /// Non-blocking: no backend is contacted unless `--probe` requests the
    /// doctor-style diagnostic, which probes each backend exactly once
    /// (probes never retry) and reports degraded backends.
    Check {
        /// Configuration file.
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Also contact backends and verify final tool names (doctor mode).
        #[arg(long)]
        probe: bool,
    },
    /// Print the configured backends without contacting them.
    ListBackends {
        /// Configuration file.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Convert a legacy deployment configuration to the public schema.
    ///
    /// Writes `--output` as a new file; existing files (including the
    /// input) are never overwritten. `${VAR}` references are preserved
    /// verbatim, and migrated backends stay optional so the original
    /// degrade-on-outage behavior is kept.
    Migrate {
        /// Legacy configuration file (legacy `[proxy]` schema).
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Destination path for the public-schema TOML; must not exist.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Print the final tool catalog and optionally diff it against a
    /// saved snapshot.
    ///
    /// Read-only: each backend is probed exactly once (`initialize` +
    /// `tools/list`, never retried, never `tools/call`). Unavailable
    /// backends are reported instead of aborting. Snapshots contain no
    /// URLs, credentials, descriptions, or schemas.
    Catalog {
        /// Configuration file.
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output format on stdout.
        #[arg(long, value_enum, default_value_t = CatalogFormat::Text)]
        format: CatalogFormat,
        /// Restrict to these backend ids (repeatable).
        #[arg(long = "backend", value_name = "ID")]
        backends: Vec<String>,
        /// Also save the catalog as a JSON snapshot; must not exist.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Compare the current catalog against this saved snapshot.
        #[arg(long, value_name = "SNAPSHOT")]
        diff: Option<PathBuf>,
        /// Exit with status 1 when `--diff` finds any change.
        #[arg(long, requires = "diff")]
        fail_on_change: bool,
    },
    /// Print version and schema information.
    Version,
}

/// `catalog` stdout formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CatalogFormat {
    /// Per-backend summary table.
    Text,
    /// The snapshot JSON (same content as `--output`).
    Json,
    /// One final tool name per line (for tool allowlists).
    Names,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("lomway").chain(args.iter().copied()))
    }

    #[test]
    fn parses_every_documented_command() {
        let cli = parse(&["serve", "--config", "a.toml"]).expect("serve");
        let Command::Serve { config } = cli.command else {
            panic!("expected Serve, got {:?}", cli.command);
        };
        assert_eq!(config, Some(PathBuf::from("a.toml")));

        let cli = parse(&["check", "--config", "a.toml", "--probe"]).expect("check");
        let Command::Check { config, probe } = cli.command else {
            panic!("expected Check, got {:?}", cli.command);
        };
        assert_eq!(config, Some(PathBuf::from("a.toml")));
        assert!(probe);

        let cli =
            parse(&["migrate", "--config", "legacy.toml", "-o", "public.toml"]).expect("migrate");
        let Command::Migrate { config, output } = cli.command else {
            panic!("expected Migrate, got {:?}", cli.command);
        };
        assert_eq!(config, Some(PathBuf::from("legacy.toml")));
        assert_eq!(output, PathBuf::from("public.toml"));

        let cli = parse(&["version"]).expect("version");
        assert!(matches!(cli.command, Command::Version));
    }

    #[test]
    fn config_is_optional_and_probe_defaults_to_false() {
        let cli = parse(&["list-backends"]).expect("list-backends");
        let Command::ListBackends { config } = cli.command else {
            panic!("expected ListBackends, got {:?}", cli.command);
        };
        assert_eq!(config, None, "config resolution precedence supplies it");

        let cli = parse(&["check"]).expect("check without flags");
        let Command::Check { config, probe } = cli.command else {
            panic!("expected Check, got {:?}", cli.command);
        };
        assert_eq!(config, None);
        assert!(!probe, "--probe is the opt-in doctor diagnostic");
    }

    #[test]
    fn parses_catalog_options() {
        let cli = parse(&[
            "catalog",
            "--format",
            "names",
            "--backend",
            "a",
            "--backend",
            "b",
            "--diff",
            "old.json",
            "--fail-on-change",
        ])
        .expect("catalog");
        let Command::Catalog {
            format,
            backends,
            diff,
            fail_on_change,
            output,
            ..
        } = cli.command
        else {
            panic!("expected Catalog, got {:?}", cli.command);
        };
        assert_eq!(format, CatalogFormat::Names);
        assert_eq!(backends, ["a", "b"]);
        assert_eq!(diff, Some(PathBuf::from("old.json")));
        assert!(fail_on_change);
        assert_eq!(output, None);

        let cli = parse(&["catalog"]).expect("catalog defaults");
        let Command::Catalog { format, .. } = cli.command else {
            panic!("expected Catalog");
        };
        assert_eq!(format, CatalogFormat::Text);
    }

    #[test]
    fn fail_on_change_requires_diff() {
        let error = parse(&["catalog", "--fail-on-change"]).expect_err("needs --diff");
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn migrate_requires_an_output_path() {
        let error = parse(&["migrate", "--config", "a.toml"]).expect_err("missing --output");
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }
}
