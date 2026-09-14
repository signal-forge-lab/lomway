use std::{fs, path::Path};

use lomway::validate_policy;
use mcp_proxy::ProxyConfig;
use tempfile::TempDir;

const SAFE_CONFIG: &str = r#"
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
name = "example"
transport = "http"
url = "http://127.0.0.1:19001/mcp"

[backends.timeout]
seconds = 30

[security]
max_argument_size = 1048576

[observability]
audit = true
log_level = "info"
json_logs = false

[observability.metrics]
enabled = true
"#;

fn load(text: &str) -> (TempDir, ProxyConfig) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("proxy.toml");
    fs::write(&path, text).expect("write config");
    let config = ProxyConfig::load(Path::new(&path)).expect("parse config");
    (dir, config)
}

fn replace(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "fixture must contain {from:?}");
    text.replacen(from, to, 1)
}

#[test]
fn accepts_minimal_safe_production_policy() {
    let (_dir, config) = load(SAFE_CONFIG);
    validate_policy(&config).expect("safe config should pass");
}

#[test]
fn rejects_non_loopback_listener() {
    let text = replace(SAFE_CONFIG, "host = \"127.0.0.1\"", "host = \"0.0.0.0\"");
    let (_dir, config) = load(&text);
    let error = validate_policy(&config).expect_err("non-loopback bind must fail");
    assert!(error.to_string().contains("127.0.0.1"));
}

#[test]
fn rejects_non_http_or_non_loopback_backends() {
    let stdio = SAFE_CONFIG
        .replace("transport = \"http\"", "transport = \"stdio\"")
        .replace(
            "url = \"http://127.0.0.1:19001/mcp\"\n",
            "command = \"mock.exe\"\n",
        );
    let (_dir, config) = load(&stdio);
    let error = validate_policy(&config).expect_err("gateway must not own backend processes");
    assert!(error.to_string().contains("must use HTTP"));

    let remote = replace(
        SAFE_CONFIG,
        "http://127.0.0.1:19001/mcp",
        "https://example.invalid/mcp",
    );
    let (_dir, config) = load(&remote);
    let error = validate_policy(&config).expect_err("southbound traffic must stay local");
    assert!(error.to_string().contains("loopback"));

    let nested_path = replace(
        SAFE_CONFIG,
        "http://127.0.0.1:19001/mcp",
        "http://127.0.0.1:19001/nested/mcp",
    );
    let (_dir, config) = load(&nested_path);
    let error = validate_policy(&config).expect_err("backend path must be exactly /mcp");
    assert!(error.to_string().contains("loopback"));

    let invalid_port = replace(
        SAFE_CONFIG,
        "http://127.0.0.1:19001/mcp",
        "http://127.0.0.1:not-a-port/mcp",
    );
    let (_dir, config) = load(&invalid_port);
    let error = validate_policy(&config).expect_err("backend port must be numeric");
    assert!(error.to_string().contains("loopback"));
}

#[test]
fn rejects_schema_rewriting_and_auth_forwarding() {
    let marker = "\n[backends.timeout]\n";
    for (name, line) in [
        (
            "alias",
            "aliases = [{ from = \"status\", to = \"other\" }]\n",
        ),
        ("forward_auth", "forward_auth = true\n"),
        ("hide_tools", "hide_tools = [\"status\"]\n"),
    ] {
        let text = SAFE_CONFIG.replacen(marker, &format!("\n{line}[backends.timeout]\n"), 1);
        let (_dir, config) = load(&text);
        let error = validate_policy(&config).expect_err("surface rewriting must fail");
        if name == "forward_auth" {
            assert!(
                error.to_string().contains("forward_auth"),
                "unexpected error: {error:#}"
            );
        } else {
            assert!(
                error.to_string().contains("schema/capability rewriting"),
                "unexpected error: {error:#}"
            );
        }
    }
}

#[test]
fn rejects_wrong_namespace_separator() {
    let text = replace(SAFE_CONFIG, "separator = \"_\"", "separator = \"/\"");
    let (_dir, config) = load(&text);
    let error = validate_policy(&config).expect_err("separator drift must fail");
    assert!(error.to_string().contains("separator"));
}

#[test]
fn rejects_hot_reload_until_gateway_policy_is_revalidated_on_reload() {
    let text = replace(SAFE_CONFIG, "hot_reload = false", "hot_reload = true");
    let (_dir, config) = load(&text);
    let error = validate_policy(&config).expect_err("hot reload must fail closed in v1");
    assert!(error.to_string().contains("hot_reload"));
}

#[test]
fn rejects_admin_token_when_admin_plane_is_not_served() {
    let text = SAFE_CONFIG.replace(
        "[security]\n",
        "[security]\nadmin_token = \"unnecessary-secret\"\n",
    );
    let (_dir, config) = load(&text);
    let error = validate_policy(&config).expect_err("v1 gateway has no admin plane");
    assert!(error.to_string().contains("admin_token"));
}

#[test]
fn rejects_search_or_discovery_exposure() {
    let text = replace(
        SAFE_CONFIG,
        "tool_discovery = false",
        "tool_discovery = true",
    );
    let (_dir, config) = load(&text);
    let error = validate_policy(&config).expect_err("discovery tools expand control surface");
    assert!(error.to_string().contains("tool_discovery"));
}

#[test]
fn rejects_retry_hedging_and_cache_per_backend() {
    for (name, block) in [
        (
            "retry",
            "\n[backends.retry]\nmax_retries = 1\ninitial_backoff_ms = 10\nmax_backoff_ms = 10\n",
        ),
        (
            "hedging",
            "\n[backends.hedging]\ndelay_ms = 10\nmax_hedges = 1\n",
        ),
        ("cache", "\n[backends.cache]\nttl_seconds = 1\n"),
    ] {
        let marker = "\n[security]\n";
        let text = SAFE_CONFIG.replacen(marker, &format!("{block}{marker}"), 1);
        let (_dir, config) = load(&text);
        let error = validate_policy(&config).expect_err("unsafe middleware must fail");
        assert!(
            error.to_string().contains(name),
            "unexpected error: {error:#}"
        );
    }
}

#[test]
fn rejects_fanout_failover_and_request_coalescing() {
    let cases = [
        ("mirror", "mirror_of = \"example\"\n"),
        ("failover", "failover_for = \"example\"\n"),
        ("canary", "canary_of = \"example\"\n"),
    ];

    for (name, line) in cases {
        let marker = "\n[security]\n";
        let extra_backend = format!(
            "\n[[backends]]\nname = \"other\"\ntransport = \"http\"\nurl = \"http://127.0.0.1:19002/mcp\"\n{line}\n[security]\n"
        );
        let text = SAFE_CONFIG.replacen(marker, &extra_backend, 1);
        let (_dir, config) = load(&text);
        let error = validate_policy(&config).expect_err("fanout/failover must fail");
        assert!(
            error.to_string().contains(name),
            "unexpected error: {error:#}"
        );
    }

    let marker = "\n[security]\n";
    let text = SAFE_CONFIG.replacen(
        marker,
        "\n[performance]\ncoalesce_requests = true\n\n[security]\n",
        1,
    );
    let (_dir, config) = load(&text);
    let error = validate_policy(&config).expect_err("coalescing is not part of v1 policy");
    assert!(error.to_string().contains("coalesce"));
}

#[test]
fn public_release_hygiene_excludes_private_tunnel_paths_and_local_build_artifacts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tunnel = fs::read_to_string(root.join("integrations/openai-secure-tunnel/tunnel.ps1"))
        .expect("read tunnel.ps1");
    let configure =
        fs::read_to_string(root.join("integrations/openai-secure-tunnel/configure-tunnel.ps1"))
            .expect("read configure-tunnel.ps1");
    let gitignore = fs::read_to_string(root.join(".gitignore")).expect("read .gitignore");

    for (name, script) in [("tunnel.ps1", tunnel), ("configure-tunnel.ps1", configure)] {
        assert!(
            !script.contains("$client = Join-Path $env:USERPROFILE 'Documents\\"),
            "{name} must not resolve the tunnel client from a user-profile Documents path"
        );
        assert!(
            script.contains("LOCAL_MCP_TUNNEL_CLIENT"),
            "{name} must use the portable tunnel-client override"
        );
    }

    for ignored in ["/target-deploy/", "/.tmp_vendor/", "/.tmp_tower_mcp/"] {
        assert!(
            gitignore.lines().any(|line| line.trim() == ignored),
            ".gitignore must exclude local release artifact {ignored}"
        );
    }
}
