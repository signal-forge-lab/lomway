//! Portable fixture regression (LMG-G7-03 / LMG-G7-04).
//!
//! The regression suite must reproduce on a clean machine without the
//! private six-backend deployment. These tests drive redistributable
//! configuration fixtures under `test-fixtures/configs/` across the
//! zero/one/many backend populations and keep the live six-backend
//! regression gated behind `--ignored` so default CI (`cargo test --locked`)
//! never requires the private deployment.
//!
//! No retry logic exists anywhere in this suite: every backend call is
//! dispatched exactly once.

use std::{
    fs,
    path::{Path, PathBuf},
};

use axum::serve;
use lomway::config::load::{ConfigFormat, load_gateway_config_with_format};
use lomway::config::load_gateway_config;
use lomway::config::migrate::to_legacy;
use lomway::config::model::GatewayConfig;
use lomway::{build_proxy, gateway_router, validate_policy};
use schemars::JsonSchema;
use serde::Deserialize;
use tower_mcp::{
    CallToolResult, HttpTransport, McpRouter, ToolBuilder,
    client::{HttpClientTransport, McpClient},
};

const FIXTURES_DIR: &str = "test-fixtures/configs";

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES_DIR)
        .join(name)
}

#[test]
fn fixture_inventory_is_pinned_and_redistributable() {
    let mut names: Vec<String> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES_DIR))
            .expect("list fixture directory")
            .map(|entry| {
                entry
                    .expect("fixture directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".toml"))
            .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "many-backends.toml".to_string(),
            "one-backend.toml".to_string(),
            "zero-backends.toml".to_string(),
        ],
        "fixture inventory changed; at least two redistributable public fixtures must remain and the inventory must move deliberately"
    );

    let forbidden_credentials = [
        "bearer_token",
        "admin_token",
        "api_key",
        "password",
        "passphrase",
    ];
    // Distinctive identifiers of the private deployment and its optional
    // integrations. Generic words are deliberately excluded to keep this a
    // stable scan.
    let forbidden_private_identifiers =
        ["workbridge", "praxiom", "xmind", "swibo", "sops", "openai"];
    let forbidden_machine_paths = ["c:\\", "c:/", "/users/", "userprofile", "appdata", "$env:"];

    for name in &names {
        let text = fs::read_to_string(fixture_path(name)).expect("read fixture");
        assert!(
            text.contains("schema_version = 1"),
            "{name} must pin the supported public schema version"
        );
        let lowered = text.to_lowercase();
        for marker in forbidden_credentials
            .iter()
            .chain(forbidden_private_identifiers.iter())
            .chain(forbidden_machine_paths.iter())
        {
            assert!(
                !lowered.contains(*marker),
                "{name} must stay redistributable; found private marker {marker:?}"
            );
        }
        for line in text
            .lines()
            .filter(|line| line.trim_start().starts_with("url"))
        {
            let value = line.split('"').nth(1).expect("quoted URL value");
            assert!(
                value.starts_with("http://127.0.0.1:") || value.starts_with("${"),
                "{name} backend URLs must stay loopback or environment references: {value:?}"
            );
        }
    }
}

#[test]
fn fixtures_cover_zero_one_and_many_backend_populations() {
    let (zero_format, zero) = load_gateway_config_with_format(&fixture_path("zero-backends.toml"))
        .expect("zero-backend fixture loads and validates end to end");
    assert_eq!(zero_format, ConfigFormat::Public);
    assert!(zero.backends.is_empty(), "the zero fixture must stay empty");

    let (one_format, one) = load_gateway_config_with_format(&fixture_path("one-backend.toml"))
        .expect("one-backend fixture loads and validates end to end");
    assert_eq!(one_format, ConfigFormat::Public);
    assert_eq!(one.backends.len(), 1);
    assert_eq!(one.backends[0].id, "alpha");
    assert_eq!(one.backends[0].prefix, "alpha_");
    assert!(
        one.backends[0].required,
        "the single fixture backend is required"
    );

    let (many_format, many) = load_gateway_config_with_format(&fixture_path("many-backends.toml"))
        .expect("many-backend fixture loads and validates end to end");
    assert_eq!(many_format, ConfigFormat::Public);
    let ids: Vec<_> = many
        .backends
        .iter()
        .map(|backend| backend.id.as_str())
        .collect();
    assert_eq!(ids, ["alpha", "beta", "gamma"]);
    let required: Vec<_> = many
        .backends
        .iter()
        .map(|backend| backend.required)
        .collect();
    assert_eq!(
        required,
        [true, true, false],
        "gamma must keep modeling the optional degrade-on-outage population"
    );

    // The deployment gate keeps the >=1 backend rule: zero backends is valid
    // only in the public schema, never at the deployment gate.
    let zero_legacy = to_legacy(&zero).expect("zero-backend fixture maps to legacy");
    assert!(
        zero_legacy.backends.is_empty(),
        "migration never invents backends"
    );
    let error = validate_policy(&zero_legacy)
        .expect_err("the deployment gate must keep requiring at least one backend");
    assert!(
        error.to_string().contains("at least one backend"),
        "{error:#}"
    );

    let one_legacy = to_legacy(&one).expect("one-backend fixture maps to legacy");
    validate_policy(&one_legacy).expect("one-backend fixture passes the deployment gate");
    let names: Vec<_> = one_legacy
        .backends
        .iter()
        .map(|backend| backend.name.as_str())
        .collect();
    assert_eq!(names, ["alpha"]);

    let many_legacy = to_legacy(&many).expect("many-backend fixture maps to legacy");
    validate_policy(&many_legacy).expect("many-backend fixture passes the deployment gate");
    let names: Vec<_> = many_legacy
        .backends
        .iter()
        .map(|backend| backend.name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "beta", "gamma"]);
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StatusInput {}

async fn spawn_fixture_backend(
    label: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool = ToolBuilder::new("status")
        .description("Return fixture mock status")
        .handler(move |_: StatusInput| async move { Ok(CallToolResult::text(label)) })
        .build();
    let router = McpRouter::new()
        .server_info(format!("fixture-{label}"), "1.0.0")
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture mock backend");
    let addr = listener.local_addr().expect("fixture mock backend address");
    let task = tokio::spawn(async move {
        serve(listener, app)
            .await
            .expect("serve fixture mock backend");
    });
    (addr, task)
}

async fn serve_fixture_gateway(
    config: &GatewayConfig,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let legacy = to_legacy(config).expect("fixture maps to the legacy proxy");
    validate_policy(&legacy).expect("fixture passes the deployment gate");
    let proxy = build_proxy(legacy).await.expect("build fixture gateway");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture gateway listener");
    let addr = listener.local_addr().expect("fixture gateway address");
    let task = tokio::spawn(async move {
        serve(listener, gateway_router(proxy))
            .await
            .expect("serve fixture gateway");
    });
    (addr, task)
}

async fn connect_client(addr: std::net::SocketAddr) -> McpClient {
    let transport = HttpClientTransport::new(format!("http://{addr}/mcp"));
    let client = McpClient::connect(transport)
        .await
        .expect("connect MCP client");
    client
        .initialize("lomway-fixture-test", "1.0.0")
        .await
        .expect("initialize MCP client");
    client
}

#[tokio::test]
async fn one_backend_fixture_drives_a_live_namespaced_gateway() {
    let (backend_addr, backend_task) = spawn_fixture_backend("alpha-live").await;
    let mut config =
        load_gateway_config(&fixture_path("one-backend.toml")).expect("one-backend fixture loads");
    config.backends[0].url = format!("http://{backend_addr}/mcp");

    let (gateway_addr, gateway_task) = serve_fixture_gateway(&config).await;
    let client = connect_client(gateway_addr).await;

    let tools = client
        .list_tools()
        .await
        .expect("list fixture gateway tools");
    let mut names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha_status"]);
    assert!(
        !names.iter().any(|name| name.starts_with("proxy_")),
        "upstream control-plane tools must stay removed on the fixture path"
    );

    let result = client
        .call_tool("alpha_status", serde_json::json!({}))
        .await
        .expect("call the namespaced fixture tool");
    assert_eq!(result.all_text(), "alpha-live");

    gateway_task.abort();
    backend_task.abort();
}

#[tokio::test]
async fn many_backend_fixture_serves_namespaces_and_degrades_the_optional_backend() {
    let (alpha_addr, alpha_task) = spawn_fixture_backend("alpha-live").await;
    let (beta_addr, beta_task) = spawn_fixture_backend("beta-live").await;
    let mut config = load_gateway_config(&fixture_path("many-backends.toml"))
        .expect("many-backend fixture loads");
    // Live substitutes replace the required sample endpoints; gamma keeps its
    // unreachable sample URL so the degrade-on-outage path stays exercised.
    config.backends[0].url = format!("http://{alpha_addr}/mcp");
    config.backends[1].url = format!("http://{beta_addr}/mcp");

    let (gateway_addr, gateway_task) = serve_fixture_gateway(&config).await;
    let client = connect_client(gateway_addr).await;

    let tools = client
        .list_tools()
        .await
        .expect("list fixture gateway tools");
    let mut names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["alpha_status", "beta_status"],
        "the unreachable optional backend must degrade, never fail startup"
    );

    for (tool, expected) in [("alpha_status", "alpha-live"), ("beta_status", "beta-live")] {
        let result = client
            .call_tool(tool, serde_json::json!({}))
            .await
            .expect("call the namespaced fixture tool");
        assert_eq!(result.all_text(), expected);
    }

    gateway_task.abort();
    alpha_task.abort();
    beta_task.abort();
}

#[test]
fn live_six_backend_regression_stays_ignored_and_machine_local() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/real_backends.rs"))
            .expect("read tests/real_backends.rs");
    assert!(
        source.contains("#[ignore"),
        "tests/real_backends.rs must stay #[ignore]-gated: the live six-backend \
         regression requires the private deployment and must never run in the \
         default `cargo test --locked` CI suite"
    );
    assert!(
        source.contains("config/proxy.local.toml"),
        "the live regression must keep loading only the machine-local, \
         gitignored configuration"
    );

    let gitignore = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".gitignore"))
        .expect("read .gitignore");
    assert!(
        gitignore
            .lines()
            .any(|line| line.trim() == "config/proxy.local.toml"),
        ".gitignore must keep excluding the private deployment configuration"
    );
}
