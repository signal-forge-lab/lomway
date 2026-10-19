//! Backend request timeout regression.
//!
//! A backend call that runs longer than tower-mcp's 30 s HTTP default must
//! succeed when the backend's configured timeout allows it, and it must not
//! close the backend client for the calls that follow or run beside it.
//! Before the fix every such call failed at 30 s with "error sending
//! request", and the next call failed with "Connection closed".

use std::time::{Duration, Instant};

use axum::serve;
use lomway::config::load_gateway_config;
use lomway::config::migrate::to_legacy;
use lomway::{build_proxy, gateway_router, validate_policy};
use schemars::JsonSchema;
use serde::Deserialize;
use tower_mcp::{
    CallToolResult, HttpTransport, McpRouter, ToolBuilder,
    client::{HttpClientConfig, HttpClientTransport, McpClient},
};

#[derive(Debug, Deserialize, JsonSchema)]
struct SleepInput {
    ms: u64,
}

async fn spawn_slow_backend() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool = ToolBuilder::new("sleep")
        .description("Sleep for the given milliseconds, then answer")
        .handler(|input: SleepInput| async move {
            tokio::time::sleep(Duration::from_millis(input.ms)).await;
            Ok(CallToolResult::text(format!("slept {}", input.ms)))
        })
        .build();
    let router = McpRouter::new()
        .server_info("fixture-slow", "1.0.0")
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

async fn serve_gateway(
    backend: std::net::SocketAddr,
    timeout_seconds: u64,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-fixtures/configs/one-backend.toml");
    let mut config = load_gateway_config(&fixture).expect("one-backend fixture loads");
    config.backends[0].url = format!("http://{backend}/mcp");
    config.backends[0].timeout_seconds = timeout_seconds;
    let legacy = to_legacy(&config).expect("fixture maps to the legacy proxy");
    validate_policy(&legacy).expect("fixture passes the deployment gate");
    let proxy = build_proxy(legacy).await.expect("build gateway");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway");
    let addr = listener.local_addr().expect("gateway address");
    let task = tokio::spawn(async move {
        serve(listener, gateway_router(proxy))
            .await
            .expect("serve gateway");
    });
    (addr, task)
}

async fn connect_client(addr: std::net::SocketAddr) -> McpClient {
    // The test client itself must outlast the slow call.
    let config = HttpClientConfig {
        request_timeout: Duration::from_secs(120),
        ..Default::default()
    };
    let transport = HttpClientTransport::with_config(format!("http://{addr}/mcp"), config);
    let client = McpClient::connect(transport)
        .await
        .expect("connect MCP client");
    client
        .initialize("lomway-timeout-test", "1.0.0")
        .await
        .expect("initialize MCP client");
    client
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backend_call_longer_than_30s_succeeds_and_keeps_the_backend_open() {
    let (backend, backend_task) = spawn_slow_backend().await;
    let (gateway, gateway_task) = serve_gateway(backend, 90).await;
    let slow_client = connect_client(gateway).await;
    let side_client = connect_client(gateway).await;

    let started = Instant::now();
    let slow = tokio::spawn(async move {
        slow_client
            .call_tool("alpha_sleep", serde_json::json!({ "ms": 32_000 }))
            .await
            .map(|result| result.all_text())
            .map_err(|error| error.to_string())
    });

    // A short call made while the slow one is running.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let side = side_client
        .call_tool("alpha_sleep", serde_json::json!({ "ms": 10 }))
        .await
        .expect("short call beside a slow call");
    assert_eq!(side.all_text(), "slept 10");

    let slow = slow.await.expect("slow call task");
    assert_eq!(
        slow.as_deref(),
        Ok("slept 32000"),
        "a call within the backend timeout must not fail at the 30 s HTTP default"
    );
    assert!(started.elapsed() >= Duration::from_secs(32));

    // The backend client must still be open after the slow call.
    let after = side_client
        .call_tool("alpha_sleep", serde_json::json!({ "ms": 10 }))
        .await
        .expect("call after a slow call must not see a closed connection");
    assert_eq!(after.all_text(), "slept 10");

    gateway_task.abort();
    backend_task.abort();
}

#[tokio::test]
async fn backend_timeout_still_bounds_a_slow_call() {
    let (backend, backend_task) = spawn_slow_backend().await;
    let (gateway, gateway_task) = serve_gateway(backend, 1).await;
    let client = connect_client(gateway).await;

    let started = Instant::now();
    let result = client
        .call_tool("alpha_sleep", serde_json::json!({ "ms": 5_000 }))
        .await;
    let failed = match result {
        Err(_) => true,
        Ok(result) => result.is_error,
    };
    assert!(failed, "a call beyond the backend timeout must fail");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the configured 1 s timeout must bound the call, took {:?}",
        started.elapsed()
    );

    gateway_task.abort();
    backend_task.abort();
}
