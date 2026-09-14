//! Configuration loading: public schema, legacy adapter detection, `${VAR}`
//! resolution, and config-path precedence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use mcp_proxy::ProxyConfig;

use crate::config::migrate;
use crate::config::model::GatewayConfig;
use crate::config::validate;
use crate::gateway::policy::validate_proxy_policy;

/// Environment variable that overrides the default config search path.
pub const CONFIG_ENV_VAR: &str = "LOMWAY_CONFIG";
/// Candidate paths checked, in order, when no explicit path is given.
pub const DEFAULT_CONFIG_CANDIDATES: [&str; 2] = ["config/proxy.local.toml", "gateway.toml"];

/// Which on-disk configuration format a file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    /// `schema_version = 1` public schema owned by this gateway.
    Public,
    /// Pre-generalization `mcp-proxy` configuration (auto-migrated).
    Legacy,
}

/// Detect the configuration format from its top-level TOML keys.
///
/// The legacy deployment schema is identified by its `[proxy]` table; every
/// other document is parsed strictly as the public schema.
pub fn detect_config_format(text: &str) -> Result<ConfigFormat> {
    let value: toml::Value = toml::from_str(text).context("configuration is not valid TOML")?;
    if value.get("proxy").is_some() {
        Ok(ConfigFormat::Legacy)
    } else {
        Ok(ConfigFormat::Public)
    }
}

/// Load and fully validate a gateway configuration from `path`.
///
/// Public-schema files are parsed strictly and accept zero, one, or many
/// backends; legacy deployment files are validated against the original
/// policy (which keeps requiring at least one backend, matching the pinned
/// upstream loader) and migrated to the public model. `${VAR}` references
/// in backend URLs are resolved for both formats.
pub fn load_gateway_config(path: &Path) -> Result<GatewayConfig> {
    load_gateway_config_with_format(path).map(|(_, config)| config)
}

/// Load a gateway configuration and report which on-disk format it used.
///
/// `check` and `list-backends` surface this so a legacy file that was
/// migrated in memory is never mistaken for a native public-schema file.
pub fn load_gateway_config_with_format(path: &Path) -> Result<(ConfigFormat, GatewayConfig)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read configuration file {}", path.display()))?;
    match detect_config_format(&text)? {
        ConfigFormat::Legacy => {
            let mut legacy = ProxyConfig::load(path).with_context(|| {
                format!("failed to load legacy configuration {}", path.display())
            })?;
            resolve_legacy_env(&mut legacy)?;
            validate_proxy_policy(&legacy)?;
            Ok((ConfigFormat::Legacy, migrate::to_public(&legacy)?))
        }
        ConfigFormat::Public => {
            let mut config: GatewayConfig = toml::from_str(&text)
                .with_context(|| format!("invalid public configuration in {}", path.display()))?;
            resolve_env_refs(&mut config)?;
            validate(&config)?;
            Ok((ConfigFormat::Public, config))
        }
    }
}

/// Migrate a legacy deployment configuration file to the public schema.
///
/// Writes `output` as a new file and never mutates `input`; an existing
/// output is refused, never overwritten (a migration is a mutation, and
/// mutations are never automatically retried or forced). `${VAR}`
/// references in backend URLs are preserved verbatim instead of baking
/// resolved machine values into the written file; the fully resolved form
/// is validated end to end before anything is written. Returns the number
/// of migrated backends.
pub fn migrate_legacy_to_public_file(input: &Path, output: &Path) -> Result<usize> {
    migrate_legacy_to_public_file_with(input, output, |name| std::env::var(name).ok())
}

/// Injectable core of [`migrate_legacy_to_public_file`], unit-testable
/// without touching the process environment.
fn migrate_legacy_to_public_file_with(
    input: &Path,
    output: &Path,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<usize> {
    ensure!(
        input != output,
        "refusing to overwrite the input configuration in place: {}",
        output.display()
    );
    ensure!(
        !output.exists(),
        "refusing to overwrite an existing file: {} (delete it explicitly to re-run the migration)",
        output.display()
    );
    let text = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read configuration file {}", input.display()))?;
    ensure!(
        detect_config_format(&text)? == ConfigFormat::Legacy,
        "{} is already in the public schema; migration converts a legacy [proxy] configuration",
        input.display()
    );
    let mut legacy = ProxyConfig::load(input)
        .with_context(|| format!("failed to load legacy configuration {}", input.display()))?;
    // Remember which backend URLs carried `${VAR}` references so the written
    // public file keeps the references instead of resolved machine values.
    let templates: Vec<String> = legacy
        .backends
        .iter()
        .map(|backend| backend.url.clone().unwrap_or_default())
        .collect();
    // Resolve backend URLs with the injectable lookup so validation sees
    // concrete loopback endpoints. Only URLs matter here: every other field
    // that can hold an environment reference is dropped by the migration.
    for backend in &mut legacy.backends {
        if let Some(url) = backend.url.as_ref().filter(|url| url.contains("${")) {
            let resolved = expand_env_vars_with(url, &lookup)?;
            backend.url = Some(resolved);
        }
    }
    validate_proxy_policy(&legacy)?;
    let mut public = migrate::to_public(&legacy)?;
    validate(&public).context("migrated public configuration fails policy validation")?;
    for (entry, template) in public.backends.iter_mut().zip(templates) {
        if template.contains("${") {
            let resolved = expand_env_vars_with(&template, &lookup)?;
            ensure!(
                resolved == entry.url,
                "backend '{}': resolving ${{VAR}} references produced a different URL across passes; refusing to write a migrated file that would not reload identically",
                entry.id
            );
            entry.url = template;
        }
    }
    let body = toml::to_string_pretty(&public)
        .context("failed to serialize the migrated public configuration")?;
    std::fs::write(output, body)
        .with_context(|| format!("failed to write {}", output.display()))?;
    Ok(public.backends.len())
}

/// Compatibility loader used by the legacy deployment tooling: loads a
/// legacy `mcp-proxy` configuration, resolves environment references, and
/// enforces the gateway policy without migrating it.
pub fn load_config(path: &Path) -> Result<ProxyConfig> {
    let mut config = ProxyConfig::load(path)?;
    resolve_legacy_env(&mut config)?;
    validate_proxy_policy(&config)?;
    Ok(config)
}

fn resolve_legacy_env(config: &mut ProxyConfig) -> Result<()> {
    resolve_legacy_env_with(config, |name| std::env::var(name).ok())
}

/// Injectable core of [`resolve_legacy_env`], unit-testable without touching
/// the process environment.
fn resolve_legacy_env_with(
    config: &mut ProxyConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<()> {
    let missing = config.check_env_vars();
    ensure!(
        missing.is_empty(),
        "configuration contains unresolved environment references: {}",
        missing.join(", ")
    );
    config.resolve_env_vars();
    // Upstream resolves environment references in credentials and env maps
    // only. Backend URLs are this gateway's policy surface, so `${VAR}`
    // references there are expanded here; unresolved variables fail the load.
    for backend in &mut config.backends {
        if let Some(url) = backend.url.as_ref().filter(|url| url.contains("${")) {
            backend.url = Some(expand_env_vars_with(url, &lookup)?);
        }
    }
    Ok(())
}

/// Resolve `${VAR}` references in backend URLs against the process
/// environment. Unresolved variables fail the load.
fn resolve_env_refs(config: &mut GatewayConfig) -> Result<()> {
    for backend in &mut config.backends {
        if backend.url.contains("${") {
            backend.url = expand_env_vars(&backend.url)?;
        }
    }
    Ok(())
}

/// Expand every `${NAME}` occurrence using the current environment.
fn expand_env_vars(input: &str) -> Result<String> {
    expand_env_vars_with(input, |name| std::env::var(name).ok())
}

/// Pure core of [`expand_env_vars`] with an injectable variable lookup.
fn expand_env_vars_with(input: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            bail!("unterminated environment variable reference in {input:?}");
        };
        let name = &after[..end];
        ensure!(
            !name.is_empty(),
            "empty environment variable reference in {input:?}"
        );
        match lookup(name) {
            Some(value) => output.push_str(&value),
            None => bail!("configuration contains unresolved environment reference ${{{name}}}"),
        }
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

/// Resolve the configuration path.
///
/// Precedence, highest first:
///
/// 1. explicit `--config <path>`;
/// 2. the `LOMWAY_CONFIG` environment variable;
/// 3. the first existing default candidate, relative to the working
///    directory: `config/proxy.local.toml`, then `gateway.toml`.
pub fn resolve_config_path(explicit: Option<&Path>) -> Result<PathBuf> {
    let env = std::env::var(CONFIG_ENV_VAR).ok();
    resolve_config_path_with(explicit, env.as_deref(), Path::new("."))
}

/// Pure core of [`resolve_config_path`], unit-testable without touching the
/// process environment or working directory.
fn resolve_config_path_with(
    explicit: Option<&Path>,
    env_value: Option<&str>,
    base: &Path,
) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(value) = env_value {
        let path = PathBuf::from(value);
        ensure!(
            path.exists(),
            "{CONFIG_ENV_VAR} points to a missing configuration file: {}",
            path.display()
        );
        return Ok(path);
    }
    for candidate in DEFAULT_CONFIG_CANDIDATES {
        let path = base.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    bail!(
        "no configuration file found; pass --config <path>, set {CONFIG_ENV_VAR}, or create one of {}",
        DEFAULT_CONFIG_CANDIDATES.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_public_and_legacy_formats() {
        assert_eq!(
            detect_config_format("schema_version = 1\n").expect("public detect"),
            ConfigFormat::Public
        );
        assert_eq!(
            detect_config_format("[proxy]\nname = \"x\"\n").expect("legacy detect"),
            ConfigFormat::Legacy
        );
        detect_config_format("not toml =").expect_err("invalid toml");
    }

    #[test]
    fn config_path_precedence_is_documented_and_tested() {
        let base = std::env::temp_dir().join(format!("lomway-precedence-{}", std::process::id()));
        std::fs::create_dir_all(base.join("config")).expect("create base");
        let explicit = base.join("explicit.toml");
        std::fs::write(&explicit, "schema_version = 1").expect("write explicit");

        // 1. explicit wins over everything.
        assert_eq!(
            resolve_config_path_with(Some(&explicit), Some("elsewhere.toml"), &base)
                .expect("explicit"),
            explicit
        );

        // 2. env var wins over default candidates.
        let env_path = base.join("from-env.toml");
        std::fs::write(&env_path, "schema_version = 1").expect("write env config");
        assert_eq!(
            resolve_config_path_with(None, Some(env_path.to_str().expect("utf8")), &base)
                .expect("env"),
            env_path
        );
        let missing = resolve_config_path_with(
            None,
            Some(base.join("nope.toml").to_str().expect("utf8")),
            &base,
        );
        assert!(missing.is_err(), "missing LOMWAY_CONFIG target must fail");

        // 3. default candidates in documented order.
        let local = base.join("config/proxy.local.toml");
        std::fs::write(&local, "[proxy]\nname = \"x\"\n").expect("write local");
        assert_eq!(
            resolve_config_path_with(None, None, &base).expect("default"),
            local
        );
        std::fs::remove_file(&local).expect("cleanup local");
        let gateway = base.join("gateway.toml");
        std::fs::write(&gateway, "schema_version = 1").expect("write gateway");
        assert_eq!(
            resolve_config_path_with(None, None, &base).expect("fallback"),
            gateway
        );

        // 4. no candidates at all is a clear error.
        std::fs::remove_file(&gateway).expect("cleanup gateway");
        let error = resolve_config_path_with(None, None, &base).expect_err("no config");
        assert!(error.to_string().contains("--config"));
        std::fs::remove_dir_all(&base).expect("cleanup base");
    }

    #[test]
    fn empty_single_and_many_public_configs_load_cleanly() {
        let base = std::env::temp_dir().join(format!("lomway-population-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("create base");

        // Zero backends: a valid public configuration.
        let empty = base.join("empty.toml");
        std::fs::write(&empty, "schema_version = 1").expect("write empty config");
        let config = load_gateway_config(&empty).expect("zero backends load cleanly");
        assert!(config.backends.is_empty());

        // One backend.
        let one = base.join("one.toml");
        std::fs::write(
            &one,
            "schema_version = 1\n\n[[backends]]\nid = \"fs\"\nprefix = \"fs_\"\nurl = \"http://127.0.0.1:18701/mcp\"\n",
        )
        .expect("write one-backend config");
        let config = load_gateway_config(&one).expect("one backend loads cleanly");
        assert_eq!(config.backends.len(), 1);

        // Many backends, mixed required/optional.
        let many = base.join("many.toml");
        std::fs::write(
            &many,
            "schema_version = 1\n\n[[backends]]\nid = \"fs\"\nprefix = \"fs_\"\nurl = \"http://127.0.0.1:18701/mcp\"\n\n[[backends]]\nid = \"calc\"\nprefix = \"calc_\"\nurl = \"http://127.0.0.1:18702/mcp\"\nrequired = false\n",
        )
        .expect("write many-backend config");
        let config = load_gateway_config(&many).expect("many backends load cleanly");
        assert_eq!(config.backends.len(), 2);
        assert!(!config.backends[1].required);

        std::fs::remove_dir_all(&base).expect("cleanup base");
    }

    #[test]
    fn env_refs_resolve_or_fail_loudly() {
        let lookup = |name: &str| (name == "LOMWAY_TEST_PORT").then(|| "18999".to_string());
        let resolved = expand_env_vars_with("http://127.0.0.1:${LOMWAY_TEST_PORT}/mcp", lookup)
            .expect("resolve");
        assert_eq!(resolved, "http://127.0.0.1:18999/mcp");

        let missing = expand_env_vars_with("http://127.0.0.1:${LOMWAY_TEST_PORT}/mcp", |_| None)
            .expect_err("missing var");
        assert!(missing.to_string().contains("LOMWAY_TEST_PORT"));

        let unterminated = expand_env_vars_with("http://127.0.0.1:${LOMWAY_TEST_PORT", |_| None)
            .expect_err("unterminated");
        assert!(unterminated.to_string().contains("unterminated"));
    }

    const LEGACY_MIGRATE_FIXTURE: &str = r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
hot_reload = false
tool_exposure = "direct"
tool_discovery = false

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "alpha"
transport = "http"
url = "http://127.0.0.1:${LOMWAY_TEST_PORT}/mcp"

[security]
max_argument_size = 1048576

[observability]
audit = true
log_level = "info"
json_logs = false
"#;

    #[test]
    fn legacy_backend_urls_resolve_env_references_before_policy_validation() {
        // Upstream resolves env references in credentials and env maps only;
        // backend URLs are expanded here so a legacy file with `${VAR}` URL
        // references validates exactly like a literal-URL file.
        let lookup = |name: &str| (name == "LOMWAY_TEST_PORT").then(|| "18779".to_string());
        let mut resolved: ProxyConfig =
            toml::from_str(LEGACY_MIGRATE_FIXTURE).expect("parse legacy (resolved pass)");
        resolve_legacy_env_with(&mut resolved, lookup).expect("resolve legacy env");
        assert_eq!(
            resolved.backends[0].url.as_deref(),
            Some("http://127.0.0.1:18779/mcp")
        );
        crate::gateway::policy::validate_proxy_policy(&resolved)
            .expect("resolved legacy config passes policy");

        // A missing variable fails the load loudly, never silently.
        let mut missing: ProxyConfig =
            toml::from_str(LEGACY_MIGRATE_FIXTURE).expect("parse legacy (missing pass)");
        let error = resolve_legacy_env_with(&mut missing, |_| None).expect_err("missing var");
        assert!(error.to_string().contains("LOMWAY_TEST_PORT"), "{error:#}");
    }

    #[test]
    fn migration_writes_public_schema_and_preserves_env_references() {
        let base = std::env::temp_dir().join(format!("lomway-migrate-ok-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("create base");
        let input = base.join("legacy.toml");
        let output = base.join("public.toml");
        std::fs::write(&input, LEGACY_MIGRATE_FIXTURE).expect("write legacy fixture");

        let lookup = |name: &str| (name == "LOMWAY_TEST_PORT").then(|| "18777".to_string());
        let count = migrate_legacy_to_public_file_with(&input, &output, lookup).expect("migrate");
        assert_eq!(count, 1);

        // The written file keeps the reference instead of the resolved port.
        let written = std::fs::read_to_string(&output).expect("read migrated file");
        assert!(written.contains("schema_version = 1"), "{written}");
        assert!(
            written.contains("${LOMWAY_TEST_PORT}"),
            "env references must be preserved, not baked in: {written}"
        );
        assert!(
            !written.contains("18777"),
            "resolved values must never be written: {written}"
        );
        // The written file loads cleanly as a public configuration once the
        // variable resolves: parse, expand, and validate it end to end.
        let mut reloaded: GatewayConfig = toml::from_str(&written).expect("parse migrated file");
        for backend in &mut reloaded.backends {
            backend.url = expand_env_vars_with(&backend.url, lookup).expect("re-expand");
        }
        crate::config::validate::validate(&reloaded).expect("migrated file validates");
        assert_eq!(reloaded.backends[0].id, "alpha");
        assert!(
            !reloaded.backends[0].required,
            "migrated backends stay optional"
        );

        std::fs::remove_dir_all(&base).expect("cleanup base");
    }

    #[test]
    fn migration_never_overwrites_and_refuses_public_input() {
        let base =
            std::env::temp_dir().join(format!("lomway-migrate-refuse-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("create base");
        let input = base.join("legacy.toml");
        std::fs::write(&input, LEGACY_MIGRATE_FIXTURE).expect("write legacy fixture");
        let lookup = |name: &str| (name == "LOMWAY_TEST_PORT").then(|| "18777".to_string());

        // A migration is a mutation: an existing output is refused, never
        // overwritten, and never automatically retried.
        let output = base.join("public.toml");
        migrate_legacy_to_public_file_with(&input, &output, lookup).expect("first migration");
        let error = migrate_legacy_to_public_file_with(&input, &output, lookup)
            .expect_err("second migration must refuse");
        assert!(
            error.to_string().contains("refusing to overwrite"),
            "{error:#}"
        );

        // In-place migration is refused.
        let error = migrate_legacy_to_public_file_with(&input, &input, lookup)
            .expect_err("in-place migration must refuse");
        assert!(error.to_string().contains("in place"), "{error:#}");

        // Public-schema input is already migrated; there is nothing to do.
        let public_input = base.join("public-input.toml");
        std::fs::write(&public_input, "schema_version = 1").expect("write public fixture");
        let error =
            migrate_legacy_to_public_file_with(&public_input, &base.join("out.toml"), lookup)
                .expect_err("public input must refuse");
        assert!(
            error.to_string().contains("already in the public schema"),
            "{error:#}"
        );

        std::fs::remove_dir_all(&base).expect("cleanup base");
    }

    #[test]
    fn load_reports_the_on_disk_format() {
        let base = std::env::temp_dir().join(format!("lomway-format-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("create base");

        let public = base.join("public.toml");
        std::fs::write(&public, "schema_version = 1").expect("write public");
        let (format, config) = load_gateway_config_with_format(&public).expect("public load");
        assert_eq!(format, ConfigFormat::Public);
        assert!(config.backends.is_empty());

        let legacy = base.join("legacy.toml");
        std::fs::write(
            &legacy,
            "[proxy]\nname = \"lomway\"\nversion = \"0.1.0\"\nseparator = \"_\"\n\n[proxy.listen]\nhost = \"127.0.0.1\"\nport = 17777\n\n[[backends]]\nname = \"alpha\"\ntransport = \"http\"\nurl = \"http://127.0.0.1:18778/mcp\"\n\n[security]\nmax_argument_size = 1048576\n\n[observability]\naudit = true\nlog_level = \"info\"\njson_logs = false\n",
        )
        .expect("write legacy");
        let (format, config) = load_gateway_config_with_format(&legacy).expect("legacy load");
        assert_eq!(format, ConfigFormat::Legacy);
        assert_eq!(config.backends.len(), 1);

        std::fs::remove_dir_all(&base).expect("cleanup base");
    }
}
