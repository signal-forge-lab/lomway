//! CLI argument definitions.
//!
//! Contract (binary name: `lomway`):
//!
//! ```text
//! lomway serve         --config <path>
//! lomway check         --config <path> [--probe]
//! lomway list-backends --config <path>
//! lomway migrate       --config <path> --output <path>
//! lomway version
//! ```
//!
//! Portability: the binary alone implements every core mode on every
//! platform — no PowerShell (or any shell) is required. Exit codes are
//! actionable: `0` success, `1` failure, `2` usage error (clap default).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    /// Print version and schema information.
    Version,
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
    fn migrate_requires_an_output_path() {
        let error = parse(&["migrate", "--config", "a.toml"]).expect_err("missing --output");
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }
}
