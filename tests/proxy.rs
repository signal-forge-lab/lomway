use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    serve,
};
use lomway::config::migrate::to_legacy;
use lomway::config::model::GatewayConfig;
use lomway::gateway::Gateway;
use lomway::{build_proxy, gateway_router};
use mcp_proxy::ProxyConfig;
use schemars::JsonSchema;
use serde::Deserialize;
use tower_mcp::{
    CallToolResult, HttpTransport, McpRouter, ToolBuilder,
    client::{HttpClientTransport, McpClient},
};

#[derive(Debug, Deserialize, JsonSchema)]
struct StatusInput {}

async fn spawn_mock_backend() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool = ToolBuilder::new("status")
        .description("Return mock status")
        .handler(|_: StatusInput| async move { Ok(CallToolResult::text("ok")) })
        .build();
    let router = McpRouter::new()
        .server_info("mock-backend", "1.0.0")
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock backend");
    let addr = listener.local_addr().expect("mock backend address");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve mock backend");
    });
    (addr, task)
}

async fn spawn_status_backend(
    label: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind labeled backend");
    let addr = listener.local_addr().expect("labeled backend address");
    let task = spawn_status_backend_on(listener, label);
    (addr, task)
}

fn spawn_status_backend_on(
    listener: tokio::net::TcpListener,
    label: &'static str,
) -> tokio::task::JoinHandle<()> {
    let tool = ToolBuilder::new("status")
        .description("Return labeled mock status")
        .handler(move |_: StatusInput| async move { Ok(CallToolResult::text(label)) })
        .build();
    let router = McpRouter::new()
        .server_info(format!("mock-{label}"), "1.0.0")
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    tokio::spawn(async move {
        serve(listener, app).await.expect("serve labeled backend");
    })
}

#[derive(Clone)]
struct ExpiringSessionState {
    expire_initial_session: Arc<AtomicBool>,
    initial_session: Arc<Mutex<Option<String>>>,
}

async fn reject_expired_initial_session(
    State(state): State<ExpiringSessionState>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(session) = request
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
    {
        let mut initial = state.initial_session.lock().expect("initial session lock");
        let initial = initial.get_or_insert_with(|| session.to_owned()).clone();
        if state.expire_initial_session.load(Ordering::SeqCst) && session == initial {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    next.run(request).await
}

async fn spawn_session_expiring_backend(
    label: &'static str,
) -> (
    std::net::SocketAddr,
    Arc<AtomicBool>,
    tokio::task::JoinHandle<()>,
) {
    let tool = ToolBuilder::new("status")
        .description("Return labeled mock status")
        .handler(move |_: StatusInput| async move { Ok(CallToolResult::text(label)) })
        .build();
    let router = McpRouter::new()
        .server_info(format!("mock-{label}"), "1.0.0")
        .tool(tool);
    let state = ExpiringSessionState {
        expire_initial_session: Arc::new(AtomicBool::new(false)),
        initial_session: Arc::new(Mutex::new(None)),
    };
    let app =
        HttpTransport::new(router)
            .into_router_at("/mcp")
            .layer(middleware::from_fn_with_state(
                state.clone(),
                reject_expired_initial_session,
            ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind session-expiring backend");
    let addr = listener
        .local_addr()
        .expect("session-expiring backend address");
    let task = tokio::spawn(async move {
        serve(listener, app)
            .await
            .expect("serve session-expiring backend");
    });
    (addr, state.expire_initial_session, task)
}

async fn spawn_slow_mutation_backend(
    calls: Arc<AtomicUsize>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool = ToolBuilder::new("mutate")
        .description("Increment a counter, then deliberately respond too slowly")
        .handler(move |_: StatusInput| {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1500)).await;
                Ok(CallToolResult::text("done"))
            }
        })
        .build();
    let router = McpRouter::new()
        .server_info("slow-mutation", "1.0.0")
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind slow backend");
    let addr = listener.local_addr().expect("slow backend address");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve slow backend");
    });
    (addr, task)
}

/// Backend exposing a single custom-named tool that always succeeds.
async fn spawn_named_tool_backend(
    tool_name: &'static str,
    description: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool = ToolBuilder::new(tool_name)
        .description(description)
        .handler(|_: StatusInput| async move { Ok(CallToolResult::text("ok")) })
        .build();
    let router = McpRouter::new()
        .server_info("named-tool", "1.0.0")
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind named-tool backend");
    let addr = listener.local_addr().expect("named-tool backend address");
    let task = tokio::spawn(async move {
        serve(listener, app)
            .await
            .expect("serve named-tool backend");
    });
    (addr, task)
}

/// Backend exposing a tool whose handler increments the counter and then
/// fails with an upstream tool error (JSON-RPC error, not a timeout).
async fn spawn_failing_mutation_backend(
    calls: Arc<AtomicUsize>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool = ToolBuilder::new("mutate")
        .description("Increment a counter, then fail without any retry")
        .handler(move |_: StatusInput| {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(tower_mcp::Error::tool(
                    "mutation refused by upstream policy",
                ))
            }
        })
        .build();
    let router = McpRouter::new()
        .server_info("failing-mutation", "1.0.0")
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failing-mutation backend");
    let addr = listener
        .local_addr()
        .expect("failing-mutation backend address");
    let task = tokio::spawn(async move {
        serve(listener, app)
            .await
            .expect("serve failing-mutation backend");
    });
    (addr, task)
}

/// Backend whose tool catalog pins a rich description and input schema so
/// the passthrough behavior can be asserted verbatim.
async fn spawn_schema_backend(
    description: &'static str,
    schema: serde_json::Value,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let shaped = ToolBuilder::new("shaped")
        .description(description)
        .input_schema(schema)
        .handler(|_: StatusInput| async move { Ok(CallToolResult::text("ok")) })
        .build();
    let plain = ToolBuilder::new("plain")
        .handler(|_: StatusInput| async move { Ok(CallToolResult::text("ok")) })
        .build();
    let router = McpRouter::new()
        .server_info("schema-backend", "1.0.0")
        .tool(shaped)
        .tool(plain);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind schema backend");
    let addr = listener.local_addr().expect("schema backend address");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve schema backend");
    });
    (addr, task)
}

fn parse_public_config(text: String) -> GatewayConfig {
    toml::from_str(&text).expect("parse public-schema test config")
}

async fn spawn_gateway_router(
    config_text: String,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("proxy.toml");
    fs::write(&path, config_text).expect("write gateway config");
    let config = ProxyConfig::load(&path).expect("load gateway config");
    let proxy = build_proxy(config).await.expect("build gateway proxy");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway listener");
    let addr = listener.local_addr().expect("gateway address");
    let task = tokio::spawn(async move {
        serve(listener, gateway_router(proxy))
            .await
            .expect("serve gateway router");
    });
    (addr, task, dir)
}

async fn connect_client(addr: std::net::SocketAddr) -> McpClient {
    let transport = HttpClientTransport::new(format!("http://{addr}/mcp"));
    let client = McpClient::connect(transport)
        .await
        .expect("connect MCP client");
    client
        .initialize("lomway-test", "1.0.0")
        .await
        .expect("initialize MCP client");
    client
}

async fn wait_for_tool_text(client: &McpClient, name: &str, expected: &str, max_wait: Duration) {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let last_error = match client.call_tool(name, serde_json::json!({})).await {
            Ok(result) if result.all_text() == expected => return,
            Ok(result) => format!("unexpected result: {}", result.all_text()),
            Err(error) => error.to_string(),
        };
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "tool {name} did not recover to {expected:?} within {max_wait:?}; last={last_error}"
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_http_status(
    http: &reqwest::Client,
    url: &str,
    expected: reqwest::StatusCode,
    max_wait: Duration,
) {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let last = match http.get(url).send().await {
            Ok(response) if response.status() == expected => return,
            Ok(response) => response.status().to_string(),
            Err(error) => error.to_string(),
        };
        if tokio::time::Instant::now() >= deadline {
            panic!("{url} did not reach {expected} within {max_wait:?}; last={last}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn build_removes_upstream_proxy_control_plane_backend() {
    let (addr, backend_task) = spawn_mock_backend().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("proxy.toml");
    let config_text = format!(
        r#"
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
name = "mock"
transport = "http"
url = "http://{addr}/mcp"

[backends.timeout]
seconds = 5

[security]
max_argument_size = 1048576

[observability.metrics]
enabled = false
"#
    );
    fs::write(&path, config_text).expect("write config");

    let config = ProxyConfig::load(&path).expect("load config");
    let proxy = build_proxy(config).await.expect("build gateway proxy");
    let namespaces = proxy.mcp_proxy().backend_namespaces();

    assert!(
        !namespaces.iter().any(|name| name == "proxy"),
        "client-visible upstream control-plane backend must be removed: {namespaces:?}"
    );
    assert_eq!(namespaces, vec!["mock"]);
    backend_task.abort();
}

#[tokio::test]
async fn external_http_surface_exposes_mcp_and_health_but_not_admin() {
    let (backend_addr, backend_task) = spawn_mock_backend().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("proxy.toml");
    let config_text = format!(
        r#"
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
name = "mock"
transport = "http"
url = "http://{backend_addr}/mcp"

[backends.timeout]
seconds = 5

[security]
max_argument_size = 1048576

[observability.metrics]
enabled = false
"#
    );
    fs::write(&path, config_text).expect("write config");
    let config = ProxyConfig::load(&path).expect("load config");
    let proxy = build_proxy(config).await.expect("build gateway proxy");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway test listener");
    let gateway_addr = listener.local_addr().expect("gateway address");
    let gateway_task = tokio::spawn(async move {
        serve(listener, gateway_router(proxy))
            .await
            .expect("serve gateway test router");
    });

    let http = reqwest::Client::new();
    let health = http
        .get(format!("http://{gateway_addr}/healthz"))
        .send()
        .await
        .expect("health request");
    assert_eq!(health.status(), reqwest::StatusCode::OK);

    for path in ["/admin/health", "/mcp/admin/health", "/mcp/admin/backends"] {
        let response = http
            .get(format!("http://{gateway_addr}{path}"))
            .bearer_auth("test-admin-token")
            .send()
            .await
            .expect("admin isolation request");
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND, "{path}");
    }

    let transport = HttpClientTransport::new(format!("http://{gateway_addr}/mcp"));
    let client = McpClient::connect(transport)
        .await
        .expect("connect MCP client to gateway");
    client
        .initialize("lomway-test", "1.0.0")
        .await
        .expect("initialize MCP client");
    let tools = client.list_tools().await.expect("list gateway tools");
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, vec!["mock_status"]);
    assert!(!names.iter().any(|name| name.starts_with("proxy_")));

    let result = client
        .call_tool("mock_status", serde_json::json!({}))
        .await
        .expect("call namespaced mock tool");
    assert_eq!(result.all_text(), "ok");

    gateway_task.abort();
    backend_task.abort();
}

#[tokio::test]
async fn same_named_backend_tools_are_namespaced_without_collision() {
    let (a_addr, a_task) = spawn_status_backend("A").await;
    let (b_addr, b_task) = spawn_status_backend("B").await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "a"
transport = "http"
url = "http://{a_addr}/mcp"
[backends.timeout]
seconds = 5

[[backends]]
name = "b"
transport = "http"
url = "http://{b_addr}/mcp"
[backends.timeout]
seconds = 5

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;
    let tools = client.list_tools().await.expect("list namespaced tools");
    let mut names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["a_status", "b_status"]);
    assert_eq!(
        client
            .call_tool("a_status", serde_json::json!({}))
            .await
            .expect("call A")
            .all_text(),
        "A"
    );
    assert_eq!(
        client
            .call_tool("b_status", serde_json::json!({}))
            .await
            .expect("call B")
            .all_text(),
        "B"
    );

    gateway_task.abort();
    a_task.abort();
    b_task.abort();
}

#[tokio::test]
async fn one_failed_backend_at_startup_is_skipped_when_another_is_healthy() {
    let (healthy_addr, healthy_task) = spawn_status_backend("healthy").await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "healthy"
transport = "http"
url = "http://{healthy_addr}/mcp"
[backends.timeout]
seconds = 5

[[backends]]
name = "offline"
transport = "http"
url = "http://127.0.0.1:9/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;
    let tools = client
        .list_tools()
        .await
        .expect("list tools with skipped backend");
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, vec!["healthy_status"]);
    assert_eq!(
        client
            .call_tool("healthy_status", serde_json::json!({}))
            .await
            .expect("healthy backend remains usable")
            .all_text(),
        "healthy"
    );

    gateway_task.abort();
    healthy_task.abort();
}

#[tokio::test]
async fn deferred_backend_started_after_gateway_start_becomes_searchable_without_restart() {
    let (direct_addr, direct_task) = spawn_status_backend("direct").await;
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve late deferred backend port");
    let deferred_addr = reserved
        .local_addr()
        .expect("late deferred backend address");
    drop(reserved);

    let config = parse_public_config(format!(
        r#"
schema_version = 1

[[backends]]
id = "direct"
prefix = "direct_"
url = "http://{direct_addr}/mcp"
timeout_seconds = 5

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://{deferred_addr}/mcp"
required = false
timeout_seconds = 5
exposure = "deferred"
"#
    ));

    let gateway = Gateway::build(config)
        .await
        .expect("build with offline optional deferred backend");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind late-deferred gateway listener");
    let gateway_addr = listener
        .local_addr()
        .expect("late-deferred gateway address");
    let gateway_task = tokio::spawn(async move {
        axum::serve(listener, gateway.router())
            .await
            .expect("serve late-deferred gateway router");
    });
    let client = connect_client(gateway_addr).await;

    tokio::time::sleep(Duration::from_millis(750)).await;
    let listener = tokio::net::TcpListener::bind(deferred_addr)
        .await
        .expect("bind late deferred backend");
    let deferred_task = spawn_status_backend_on(listener, "late");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let search_text = loop {
        let result = client
            .call_tool(
                "lomway_search_tools",
                serde_json::json!({
                    "query": "status",
                    "backend": "browser",
                    "limit": 5
                }),
            )
            .await
            .expect("search late deferred backend");
        let text = result.all_text();
        if text.contains("browser_status") {
            break text;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "late deferred backend never became searchable: {text}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert!(search_text.contains("browser_status"));

    let tools = client
        .list_tools()
        .await
        .expect("list tools after late deferred recovery");
    assert!(
        tools.tools.iter().all(|tool| tool.name != "browser_status"),
        "late deferred tool must remain hidden from direct tools/list"
    );

    let routed = client
        .call_tool(
            "lomway_call_tool",
            serde_json::json!({
                "name": "browser_status",
                "arguments": {}
            }),
        )
        .await
        .expect("call late deferred tool");
    assert_eq!(routed.all_text(), "late");

    gateway_task.abort();
    direct_task.abort();
    deferred_task.abort();
}

#[tokio::test]
async fn backend_skipped_at_startup_recovers_without_gateway_restart() {
    let (healthy_addr, healthy_task) = spawn_status_backend("healthy").await;
    let offline_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve offline backend address");
    let offline_addr = offline_listener
        .local_addr()
        .expect("offline backend address");
    drop(offline_listener);

    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "healthy"
transport = "http"
url = "http://{healthy_addr}/mcp"
[backends.timeout]
seconds = 5

[[backends]]
name = "offline"
transport = "http"
url = "http://{offline_addr}/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;

    let tools = client
        .list_tools()
        .await
        .expect("list tools while backend is skipped");
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, vec!["healthy_status"]);

    let listener = tokio::net::TcpListener::bind(offline_addr)
        .await
        .expect("start previously skipped backend");
    let recovered_task = spawn_status_backend_on(listener, "recovered");

    wait_for_tool_text(
        &client,
        "offline_status",
        "recovered",
        Duration::from_secs(5),
    )
    .await;

    gateway_task.abort();
    healthy_task.abort();
    recovered_task.abort();
}

#[tokio::test]
async fn timeout_does_not_retry_a_mutating_tool() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (slow_addr, slow_task) = spawn_slow_mutation_backend(Arc::clone(&calls)).await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "slow"
transport = "http"
url = "http://{slow_addr}/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;
    let error = client
        .call_tool("slow_mutate", serde_json::json!({}))
        .await
        .expect_err("timeout must surface as an MCP error");
    assert!(
        error.to_string().to_lowercase().contains("timed out"),
        "unexpected timeout error: {error}"
    );

    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "mutation must be dispatched exactly once"
    );

    gateway_task.abort();
    slow_task.abort();
}

#[tokio::test]
async fn brief_backend_restart_recovers_without_gateway_restart() {
    let (backend_addr, backend_task) = spawn_status_backend("before").await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "restartable"
transport = "http"
url = "http://{backend_addr}/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config.clone()).await;
    let client = connect_client(gateway_addr).await;

    assert_eq!(
        client
            .call_tool("restartable_status", serde_json::json!({}))
            .await
            .expect("initial call succeeds")
            .all_text(),
        "before"
    );

    backend_task.abort();
    let _ = backend_task.await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let first_error = client
        .call_tool("restartable_status", serde_json::json!({}))
        .await
        .expect_err("call while backend is down must fail");
    eprintln!("brief outage error: {first_error}");
    assert!(
        first_error.to_string().to_lowercase().contains("transport")
            || first_error.to_string().to_lowercase().contains("connect")
            || first_error.to_string().to_lowercase().contains("request"),
        "unexpected backend-down error: {first_error}"
    );

    let listener = tokio::net::TcpListener::bind(backend_addr)
        .await
        .expect("rebind backend on same address");
    let restarted_backend_task = spawn_status_backend_on(listener, "after");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let direct_transport = HttpClientTransport::new(format!("http://{backend_addr}/mcp"));
    let direct = McpClient::connect(direct_transport)
        .await
        .expect("connect directly to restarted backend");
    direct
        .initialize("direct-restart-check", "1.0.0")
        .await
        .expect("initialize direct restarted backend client");
    assert_eq!(
        direct
            .call_tool("status", serde_json::json!({}))
            .await
            .expect("direct call after restart succeeds")
            .all_text(),
        "after"
    );

    assert_eq!(
        client
            .call_tool("restartable_status", serde_json::json!({}))
            .await
            .expect("gateway should recover after a brief backend restart")
            .all_text(),
        "after"
    );

    gateway_task.abort();
    let (replacement_gateway_addr, replacement_gateway_task, _replacement_dir) =
        spawn_gateway_router(config).await;
    let replacement_client = connect_client(replacement_gateway_addr).await;
    assert_eq!(
        replacement_client
            .call_tool("restartable_status", serde_json::json!({}))
            .await
            .expect("fresh gateway transport recovers")
            .all_text(),
        "after"
    );

    replacement_gateway_task.abort();
    restarted_backend_task.abort();
}

#[tokio::test]
async fn sustained_backend_outage_recovers_without_gateway_restart() {
    let (backend_addr, backend_task) = spawn_status_backend("before").await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "restartable"
transport = "http"
url = "http://{backend_addr}/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;

    assert_eq!(
        client
            .call_tool("restartable_status", serde_json::json!({}))
            .await
            .expect("initial call succeeds")
            .all_text(),
        "before"
    );

    backend_task.abort();
    let _ = backend_task.await;

    for attempt in 1..=3 {
        let error = client
            .call_tool("restartable_status", serde_json::json!({}))
            .await
            .expect_err("call while backend is down must fail");
        eprintln!("sustained outage attempt {attempt}: {error}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let listener = tokio::net::TcpListener::bind(backend_addr)
        .await
        .expect("rebind backend on same address");
    let restarted_backend_task = spawn_status_backend_on(listener, "after");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let direct_transport = HttpClientTransport::new(format!("http://{backend_addr}/mcp"));
    let direct = McpClient::connect(direct_transport)
        .await
        .expect("connect directly to restarted backend");
    direct
        .initialize("direct-sustained-restart-check", "1.0.0")
        .await
        .expect("initialize direct restarted backend client");
    assert_eq!(
        direct
            .call_tool("status", serde_json::json!({}))
            .await
            .expect("direct call after sustained restart succeeds")
            .all_text(),
        "after"
    );

    wait_for_tool_text(
        &client,
        "restartable_status",
        "after",
        Duration::from_secs(5),
    )
    .await;

    gateway_task.abort();
    restarted_backend_task.abort();
}

#[tokio::test]
async fn session_only_backend_failure_recovers_without_port_outage() {
    let (backend_addr, expire_session, backend_task) =
        spawn_session_expiring_backend("stable").await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "restartable"
transport = "http"
url = "http://{backend_addr}/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;

    assert_eq!(
        client
            .call_tool("restartable_status", serde_json::json!({}))
            .await
            .expect("initial call succeeds")
            .all_text(),
        "stable"
    );

    expire_session.store(true, Ordering::SeqCst);

    wait_for_tool_text(
        &client,
        "restartable_status",
        "stable",
        Duration::from_secs(5),
    )
    .await;

    gateway_task.abort();
    backend_task.abort();
}

#[tokio::test]
async fn readiness_tracks_backend_outage_and_recovery() {
    let (backend_addr, backend_task) = spawn_status_backend("ready").await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "restartable"
transport = "http"
url = "http://{backend_addr}/mcp"
[backends.timeout]
seconds = 1

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let http = reqwest::Client::new();
    let health_url = format!("http://{gateway_addr}/healthz");
    let ready_url = format!("http://{gateway_addr}/readyz");

    wait_for_http_status(
        &http,
        &ready_url,
        reqwest::StatusCode::OK,
        Duration::from_secs(3),
    )
    .await;

    backend_task.abort();
    let _ = backend_task.await;
    wait_for_http_status(
        &http,
        &ready_url,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        Duration::from_secs(4),
    )
    .await;
    assert_eq!(
        http.get(&health_url)
            .send()
            .await
            .expect("liveness request")
            .status(),
        reqwest::StatusCode::OK,
        "liveness must remain healthy while an upstream backend is down"
    );

    let listener = tokio::net::TcpListener::bind(backend_addr)
        .await
        .expect("rebind backend on same address");
    let restarted_backend_task = spawn_status_backend_on(listener, "recovered");

    wait_for_http_status(
        &http,
        &ready_url,
        reqwest::StatusCode::OK,
        Duration::from_secs(5),
    )
    .await;

    gateway_task.abort();
    restarted_backend_task.abort();
}

// ---------------------------------------------------------------------------
// LMG-G3-03: schema preservation regression.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_list_preserves_upstream_descriptions_and_input_schemas() {
    const DESCRIPTION: &str = "Preserve upstream wording verbatim ✓ 日本語テスト";
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Target path"},
            "mode": {
                "type": "string",
                "enum": ["read", "write"],
                "default": "read"
            }
        },
        "required": ["path"],
        "additionalProperties": false
    });
    let (backend_addr, backend_task) = spawn_schema_backend(DESCRIPTION, schema.clone()).await;
    let config = parse_public_config(format!(
        r#"
schema_version = 1

[[backends]]
id = "shape"
prefix = "shape_"
url = "http://{backend_addr}/mcp"
timeout_seconds = 5
"#
    ));

    let gateway = Gateway::build(config)
        .await
        .expect("build gateway for the schema passthrough check");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway listener");
    let gateway_addr = listener.local_addr().expect("gateway address");
    let gateway_task = tokio::spawn(async move {
        axum::serve(listener, gateway.router())
            .await
            .expect("serve gateway router");
    });

    let client = connect_client(gateway_addr).await;
    let tools = client.list_tools().await.expect("list gateway tools");

    let shaped = tools
        .tools
        .iter()
        .find(|tool| tool.name == "shape_shaped")
        .expect("namespaced shaped tool is exposed");
    assert_eq!(
        shaped.description.as_deref(),
        Some(DESCRIPTION),
        "upstream tool descriptions must pass through verbatim"
    );
    assert_eq!(
        shaped.input_schema, schema,
        "upstream input schemas must pass through verbatim"
    );

    let plain = tools
        .tools
        .iter()
        .find(|tool| tool.name == "shape_plain")
        .expect("namespaced plain tool is exposed");
    assert_eq!(
        plain.description, None,
        "an absent upstream description must stay absent, never invented"
    );

    gateway_task.abort();
    backend_task.abort();
}

#[tokio::test]
async fn deferred_backend_is_hidden_but_searchable_describable_and_callable() {
    let (direct_addr, direct_task) = spawn_status_backend("direct").await;
    let (deferred_addr, deferred_task) =
        spawn_named_tool_backend("navigate", "Navigate a browser page").await;
    let config = parse_public_config(format!(
        r#"
schema_version = 1

[server]
instructions = "Use the browser deferred backend for web-page automation."

[[backends]]
id = "direct"
prefix = "direct_"
url = "http://{direct_addr}/mcp"
timeout_seconds = 5

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://{deferred_addr}/mcp"
timeout_seconds = 5
exposure = "deferred"
"#
    ));

    let gateway = Gateway::build(config)
        .await
        .expect("build mixed direct/deferred gateway");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mixed gateway listener");
    let gateway_addr = listener.local_addr().expect("mixed gateway address");
    let gateway_task = tokio::spawn(async move {
        axum::serve(listener, gateway.router())
            .await
            .expect("serve mixed gateway router");
    });

    let client = connect_client(gateway_addr).await;
    let tools = client.list_tools().await.expect("list mixed gateway tools");
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"direct_status"));
    assert!(!names.contains(&"browser_navigate"));
    assert!(names.contains(&"lomway_search_tools"));
    assert!(names.contains(&"lomway_describe_tool"));
    assert!(names.contains(&"lomway_call_tool"));
    let search_tool = tools
        .tools
        .iter()
        .find(|tool| tool.name == "lomway_search_tools")
        .expect("search meta-tool is exposed");
    let search_description = search_tool
        .description
        .as_deref()
        .expect("search meta-tool has routing guidance");
    assert!(
        search_description.contains("browser"),
        "configured deferred backend ids must be visible to the agent: {search_description}"
    );
    assert!(
        search_description.contains("Omit backend when unsure"),
        "agent should be told it can search all deferred backends: {search_description}"
    );
    assert!(
        search_description.contains("lomway_describe_tool")
            && search_description.contains("lomway_call_tool"),
        "agent should be told the search -> describe -> call flow: {search_description}"
    );
    assert!(
        search_description.contains("Use the browser deferred backend for web-page automation."),
        "deployment routing guidance must be visible in the search tool description: {search_description}"
    );
    assert!(
        names.iter().all(|name| !name.starts_with("proxy_")),
        "upstream proxy control-plane tools must stay absent: {names:?}"
    );

    let direct = client
        .call_tool("direct_status", serde_json::json!({}))
        .await
        .expect("direct tool remains normally callable");
    assert_eq!(direct.all_text(), "direct");

    let direct_deferred_error = client
        .call_tool("browser_navigate", serde_json::json!({}))
        .await
        .expect_err("deferred tool must reject a direct call");
    assert!(
        direct_deferred_error
            .to_string()
            .contains("browser_navigate"),
        "{direct_deferred_error}"
    );

    let search = client
        .call_tool(
            "lomway_search_tools",
            serde_json::json!({
                "query": "navigate browser",
                "backend": "browser",
                "limit": 5
            }),
        )
        .await
        .expect("search deferred tools");
    assert!(search.all_text().contains("browser_navigate"));

    let described = client
        .call_tool(
            "lomway_describe_tool",
            serde_json::json!({"name": "browser_navigate"}),
        )
        .await
        .expect("describe deferred tool");
    let description = described.all_text();
    assert!(description.contains("browser_navigate"), "{description}");
    assert!(
        description.contains("Navigate a browser page"),
        "{description}"
    );
    assert!(description.contains("inputSchema"), "{description}");

    let routed = client
        .call_tool(
            "lomway_call_tool",
            serde_json::json!({
                "name": "browser_navigate",
                "arguments": {}
            }),
        )
        .await
        .expect("invoke deferred tool through safe meta-tool");
    assert_eq!(routed.all_text(), "ok");

    let direct_target_error = client
        .call_tool(
            "lomway_call_tool",
            serde_json::json!({
                "name": "direct_status",
                "arguments": {}
            }),
        )
        .await
        .expect("meta-tool rejection is returned as an MCP tool error result");
    assert!(
        direct_target_error.is_error,
        "meta-tool must reject direct targets"
    );
    assert!(
        direct_target_error.all_text().contains("deferred"),
        "{}",
        direct_target_error.all_text()
    );

    gateway_task.abort();
    direct_task.abort();
    deferred_task.abort();
}

// ---------------------------------------------------------------------------
// LMG-G7-01: public mock fixture E2E — required-failure and collision paths.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_required_backends_down_fails_gateway_startup() {
    let config = parse_public_config(
        r#"
schema_version = 1

[[backends]]
id = "dead"
prefix = "dead_"
url = "http://127.0.0.1:9/mcp"
required = true
timeout_seconds = 1
"#
        .to_string(),
    );

    let public_error = Gateway::build(config.clone())
        .await
        .expect_err("all-required-down must fail startup on the public path");
    let message = public_error.to_string();
    assert!(
        message.contains("required backend(s) unavailable"),
        "unexpected public-path startup error: {message}"
    );
    assert!(message.contains("dead"), "{message}");
    assert!(message.contains("fails closed"), "{message}");

    // The legacy entry point keeps the deployment's historical
    // degrade-on-outage semantics (migration marks backends optional), so the
    // same all-down configuration must still fail closed there, via the
    // all-unavailable branch.
    let legacy_error = Gateway::build_from_legacy(
        to_legacy(&config).expect("public config maps to the legacy schema"),
    )
    .await
    .expect_err("all-down must still fail closed on the legacy path");
    let legacy_message = legacy_error.to_string();
    assert!(
        legacy_message.contains("all backends are unavailable"),
        "unexpected legacy-path startup error: {legacy_message}"
    );
    assert!(legacy_message.contains("fails closed"), "{legacy_message}");
}

#[tokio::test]
async fn overlapping_prefixes_fail_before_cross_prefix_tool_ownership_can_be_ambiguous() {
    let config = parse_public_config(
        r#"
schema_version = 1

[[backends]]
id = "a"
prefix = "a_"
url = "http://127.0.0.1:19001/mcp"
timeout_seconds = 5

[[backends]]
id = "a_b"
prefix = "a_b_"
url = "http://127.0.0.1:19002/mcp"
timeout_seconds = 5
"#
        .to_string(),
    );

    let error = Gateway::build(config)
        .await
        .expect_err("overlapping prefixes must fail before backend probing");
    let message = error.to_string();
    assert!(
        message.contains("overlapping backend prefixes"),
        "{message}"
    );
    assert!(message.contains("a_"), "{message}");
    assert!(message.contains("a_b_"), "{message}");
}

// ---------------------------------------------------------------------------
// LMG-G7-02: exactly-once mutation semantics on the error path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_error_path_does_not_retry_a_mutating_tool() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (err_addr, err_task) = spawn_failing_mutation_backend(Arc::clone(&calls)).await;
    let config = format!(
        r#"
[proxy]
name = "lomway"
version = "0.1.0"
separator = "_"
tool_exposure = "direct"

[proxy.listen]
host = "127.0.0.1"
port = 17777

[[backends]]
name = "err"
transport = "http"
url = "http://{err_addr}/mcp"
[backends.timeout]
seconds = 5

[security]
max_argument_size = 1048576
"#
    );
    let (gateway_addr, gateway_task, _dir) = spawn_gateway_router(config).await;
    let client = connect_client(gateway_addr).await;

    let failed = client
        .call_tool("err_mutate", serde_json::json!({}))
        .await
        .expect("the failure must surface to the client instead of being masked");
    assert!(
        failed.is_error,
        "an upstream tool error must surface as an error result: {failed:?}"
    );

    // Give any hypothetical retry ample time to land before asserting the
    // backend saw the mutation exactly once.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "an errored mutation must be dispatched exactly once; no automatic retry"
    );

    gateway_task.abort();
    err_task.abort();
}
