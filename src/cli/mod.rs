//! Public command-line interface.
//!
//! Modes (all core modes run from the binary alone; no shell or PowerShell
//! is required on any platform):
//!
//! * `serve` — run the gateway on the loopback listener;
//! * `check` — non-blocking configuration validation (no network I/O);
//!   `check --probe` adds the doctor-style backend health diagnostic
//!   (one probe per backend, never retried);
//! * `list-backends` — print the configured backends without contacting
//!   them;
//! * `migrate` — convert a legacy deployment configuration to the public
//!   schema as a new file (never overwrites, never retries);
//! * `version` — print version and schema information.
//!
//! Exit codes are actionable: `0` success, `1` failure, `2` usage error.

pub mod args;

use std::path::Path;

use anyhow::{Context, Result};

pub use args::{Cli, Command};

use crate::backend::registry::BackendRegistry;
use crate::config::load::{
    ConfigFormat, load_gateway_config, load_gateway_config_with_format,
    migrate_legacy_to_public_file, resolve_config_path,
};
use crate::config::model::GatewayConfig;
use crate::gateway::Gateway;
use crate::namespace::collision::plan_tools;

/// Dispatch a parsed command.
pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Version => {
            print_version();
            Ok(())
        }
        Command::Serve { config } => {
            let path = resolve_config_path(config.as_deref())?;
            let config = load_gateway_config(&path)?;
            init_logging(&config);
            let gateway = Gateway::build(config)
                .await
                .with_context(|| format!("failed to start gateway with {}", path.display()))?;
            print_startup(&gateway);
            gateway.serve().await
        }
        Command::Check { config, probe } => {
            let path = resolve_config_path(config.as_deref())?;
            let (format, config) = load_gateway_config_with_format(&path)?;
            let report = check_report(&path, format, &config, probe).await?;
            println!("{report}");
            Ok(())
        }
        Command::ListBackends { config } => {
            let path = resolve_config_path(config.as_deref())?;
            let (format, config) = load_gateway_config_with_format(&path)?;
            println!("Backends for {} ({})", path.display(), format_note(format));
            print_backends(&config);
            Ok(())
        }
        Command::Migrate { config, output } => {
            let path = resolve_config_path(config.as_deref())?;
            let count = migrate_legacy_to_public_file(&path, &output)?;
            println!(
                "Migrated {count} backend(s) to the public schema: {}",
                output.display()
            );
            println!("${{VAR}} references were preserved, not resolved into machine values.");
            println!(
                "Migrated backends stay optional (required = false) to preserve degrade-on-outage behavior."
            );
            Ok(())
        }
    }
}

fn print_version() {
    println!(
        "{} {} (mcp protocol 2026-07-28, config schema_version {})",
        crate::DEFAULT_SERVER_NAME,
        env!("CARGO_PKG_VERSION"),
        crate::config::validate::SCHEMA_VERSION
    );
}

fn print_startup(gateway: &Gateway) {
    let (host, port) = gateway.listen_addr();
    println!("Serving MCP on http://{host}:{port}/mcp (healthz /healthz, readyz /readyz)");
}

fn format_note(format: ConfigFormat) -> &'static str {
    match format {
        ConfigFormat::Public => "public schema",
        ConfigFormat::Legacy => "legacy schema, migrated in memory",
    }
}

/// Build the full `check` report.
///
/// Without `probe` this is a purely local, non-blocking validation: parse,
/// policy, and config-level collision preflight only, with zero network
/// I/O. With `probe` the doctor-style diagnostic contacts every backend
/// exactly once (probes never retry) and verifies final tool names.
async fn check_report(
    path: &Path,
    format: ConfigFormat,
    config: &GatewayConfig,
    probe: bool,
) -> Result<String> {
    let mut report = format!("Config OK: {} ({})\n", path.display(), format_note(format));
    report.push_str(&format!("schema_version: {}\n", config.schema_version));
    report.push_str(&format!(
        "Listen: {}:{}\n",
        config.server.host, config.server.port
    ));
    report.push_str(&format!("Backends: {}\n", config.backends.len()));
    for backend in &config.backends {
        report.push_str(&format!(
            "- {} (prefix {}, {}, {})\n",
            backend.id,
            backend.prefix,
            if backend.required {
                "required"
            } else {
                "optional"
            },
            backend.url
        ));
    }
    if probe {
        let registry = BackendRegistry::from_config(config)?;
        let probed = crate::backend::probe::probe_registry(&registry).await?;
        let plan = plan_tools(&probed)?;
        report.push_str(&format!(
            "Doctor probe: {} healthy, {} unavailable; {} final tool names planned, no collisions\n",
            probed.healthy.len(),
            probed.unavailable.len(),
            plan.total_tools()
        ));
        if !probed.unavailable.is_empty() {
            report.push_str(&format!(
                "Degraded (optional): {}\n",
                probed.unavailable_ids().join(", ")
            ));
        }
    } else {
        report.push_str("Collision preflight (config level): OK (unique ids and prefixes)\n");
        report.push_str(
            "Backends not contacted (non-blocking validation); use --probe for the doctor-style health check\n",
        );
    }
    Ok(report)
}

fn print_backends(config: &GatewayConfig) {
    println!("{:<20} {:<20} {:<10} URL", "ID", "PREFIX", "REQUIRED");
    for backend in &config.backends {
        println!(
            "{:<20} {:<20} {:<10} {}",
            backend.id, backend.prefix, backend.required, backend.url
        );
    }
}

/// Initialize `tracing` from the configuration (env overrides win). Tool
/// arguments and secrets are never part of the log surface.
fn init_logging(config: &GatewayConfig) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.observability.log_level));
    if config.observability.json_logs {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLIC_CONFIG: &str = r#"
schema_version = 1

[[backends]]
id = "fs"
prefix = "fs_"
url = "http://127.0.0.1:18701/mcp"

[[backends]]
id = "calc"
prefix = "calc_"
url = "http://127.0.0.1:18702/mcp"
required = false
"#;

    #[tokio::test]
    async fn check_without_probe_is_nonblocking_and_reports_format() {
        let config: GatewayConfig = toml::from_str(PUBLIC_CONFIG).expect("parse public fixture");
        let report = check_report(
            Path::new("public.toml"),
            ConfigFormat::Public,
            &config,
            false,
        )
        .await
        .expect("check report");
        assert!(report.contains("public schema"), "{report}");
        assert!(report.contains("Backends: 2"), "{report}");
        assert!(report.contains("not contacted"), "{report}");
        assert!(
            !report.contains("Doctor probe"),
            "the doctor diagnostic must stay behind --probe: {report}"
        );
    }

    #[tokio::test]
    async fn check_reports_in_memory_legacy_migration() {
        let config: GatewayConfig = toml::from_str(PUBLIC_CONFIG).expect("parse fixture");
        let report = check_report(
            Path::new("legacy.toml"),
            ConfigFormat::Legacy,
            &config,
            false,
        )
        .await
        .expect("check report");
        assert!(
            report.contains("legacy schema, migrated in memory"),
            "{report}"
        );
    }

    #[test]
    fn list_backends_reports_the_on_disk_format() {
        // The format note is the only piece of list-backends behavior that
        // changed; pin both renderings.
        assert_eq!(format_note(ConfigFormat::Public), "public schema");
        assert_eq!(
            format_note(ConfigFormat::Legacy),
            "legacy schema, migrated in memory"
        );
    }
}
