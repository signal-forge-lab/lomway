//! Backend tool errors must not be treated as transport failures.
//!
//! Before the fix, a backend's own error reply to a tool call (invalid
//! arguments, a failing tool) was counted like a broken connection. A few in
//! a row made the gateway rebuild the backend transport, and a call that
//! landed while the namespace was briefly removed failed with "Unknown tool".

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::IntoResponse,
    serve,
};
use lomway::config::load_gateway_config;
use lomway::config::migrate::to_legacy;
use lomway::config::model::MonitorConfig;
use lomway::gateway::{MonitorHandle, build_proxy_with_monitor, gateway_router_with_monitor};
use mcp_proxy::{ReconnectTracker, TransportErrorObserver, is_backend_transport_failure};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::broadcast;
use tower::{ServiceExt, util::BoxCloneService};
use tower_mcp::{
    CallToolResult, HttpTransport, McpRouter, ToolBuilder,
    client::{HttpClientConfig, HttpClientTransport, McpClient},
    error::JsonRpcError,
    protocol::{CallToolParams, McpRequest, McpResponse, RequestId},
    router::{Extensions, RouterRequest, RouterResponse},
};

#[derive(Debug, Deserialize, JsonSchema)]
struct EchoInput {
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NoInput {}

/// A backend with an argument-checked tool and failing tools. `reject`
/// always gets a JSON-RPC -32602 error response (what coding-tools-mcp sends
/// for invalid arguments). While `broken` is set every HTTP request gets a
/// 500 (a real transport failure with the port still open).
async fn spawn_backend(
    broken: Arc<AtomicBool>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let echo = ToolBuilder::new("echo")
        .description("Echo the text")
        .handler(|input: EchoInput| async move { Ok(CallToolResult::text(input.text)) })
        .build();
    let fail_result = ToolBuilder::new("fail_result")
        .description("Answer with an isError result")
        .handler(|_: NoInput| async move { Ok(CallToolResult::error("tool says no")) })
        .build();
    let fail_err = ToolBuilder::new("fail_err")
        .description("Fail with a tool error")
        .handler(|_: NoInput| async move {
            Err::<CallToolResult, _>(tower_mcp::Error::tool("tool failed"))
        })
        .build();
    let reject = ToolBuilder::new("reject")
        .description("Always rejected with invalid params")
        .handler(|_: NoInput| async move { Ok(CallToolResult::text("unreachable")) })
        .build();
    let router = McpRouter::new()
        .server_info("fixture-errors", "1.0.0")
        .tool(echo)
        .tool(fail_result)
        .tool(fail_err)
        .tool(reject);
    let app = HttpTransport::new(router)
        .into_router_at("/mcp")
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let broken = broken.clone();
            async move {
                if broken.load(Ordering::SeqCst) {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "backend down").into_response();
                }
                let (parts, body) = request.into_parts();
                let bytes = axum::body::to_bytes(body, 1 << 20)
                    .await
                    .expect("read request body");
                if let Ok(message) = serde_json::from_slice::<serde_json::Value>(&bytes)
                    && message["method"] == "tools/call"
                    && message["params"]["name"] == "reject"
                {
                    let reply = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": message["id"],
                        "error": { "code": -32602, "message": "Invalid params: cmd is required" },
                    });
                    return (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        reply.to_string(),
                    )
                        .into_response();
                }
                next.run(Request::from_parts(parts, Body::from(bytes)))
                    .await
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backend");
    let addr = listener.local_addr().expect("backend address");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve backend");
    });
    (addr, task)
}

async fn serve_gateway(
    backend: std::net::SocketAddr,
) -> (
    std::net::SocketAddr,
    MonitorHandle,
    tokio::task::JoinHandle<()>,
) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-fixtures/configs/one-backend.toml");
    let mut config = load_gateway_config(&fixture).expect("one-backend fixture loads");
    config.backends[0].url = format!("http://{backend}/mcp");
    config.backends[0].timeout_seconds = 10;
    let legacy = to_legacy(&config).expect("fixture maps to the legacy proxy");
    let settings = MonitorConfig {
        interval_seconds: 1,
        deferred_idle_minutes: 10,
    };
    let (proxy, monitor) = build_proxy_with_monitor(legacy, settings)
        .await
        .expect("build gateway");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway");
    let addr = listener.local_addr().expect("gateway address");
    let router = gateway_router_with_monitor(proxy, monitor.clone());
    let task = tokio::spawn(async move {
        serve(listener, router).await.expect("serve gateway");
    });
    (addr, monitor, task)
}

async fn connect_client(addr: std::net::SocketAddr) -> McpClient {
    let config = HttpClientConfig {
        request_timeout: Duration::from_secs(30),
        ..Default::default()
    };
    let transport = HttpClientTransport::with_config(format!("http://{addr}/mcp"), config);
    let client = McpClient::connect(transport)
        .await
        .expect("connect MCP client");
    client
        .initialize("lomway-tool-error-test", "1.0.0")
        .await
        .expect("initialize MCP client");
    client
}

/// The call failed one way or another (JSON-RPC error or isError result).
async fn call_fails(client: &McpClient, name: &str, args: serde_json::Value) -> String {
    match client.call_tool(name, args).await {
        Err(error) => error.to_string(),
        Ok(result) => {
            assert!(result.is_error, "{name} unexpectedly succeeded");
            result.all_text()
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_backend_tool_errors_do_not_rebuild_the_transport() {
    let broken = Arc::new(AtomicBool::new(false));
    let (backend, backend_task) = spawn_backend(broken).await;
    let (gateway, monitor, gateway_task) = serve_gateway(backend).await;
    let client = connect_client(gateway).await;

    for _ in 0..4 {
        let error = call_fails(&client, "alpha_reject", serde_json::json!({})).await;
        assert!(error.contains("-32602"), "{error}");
        let error = call_fails(&client, "alpha_echo", serde_json::json!({ "wrong": 1 })).await;
        assert!(!error.contains("Unknown tool"), "{error}");
    }
    for _ in 0..3 {
        call_fails(&client, "alpha_fail_result", serde_json::json!({})).await;
        call_fails(&client, "alpha_fail_err", serde_json::json!({})).await;
    }

    // Give the 1 s monitor several ticks to (wrongly) act on them.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        monitor.reconnect_count(),
        0,
        "backend error replies must not rebuild the transport"
    );
    assert!(monitor.ready());

    let ok = client
        .call_tool("alpha_echo", serde_json::json!({ "text": "still here" }))
        .await
        .expect("normal call after tool errors");
    assert_eq!(ok.all_text(), "still here");

    gateway_task.abort();
    backend_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failures_still_rebuild_the_transport() {
    let broken = Arc::new(AtomicBool::new(false));
    let (backend, backend_task) = spawn_backend(broken.clone()).await;
    let (gateway, monitor, gateway_task) = serve_gateway(backend).await;
    let client = connect_client(gateway).await;

    broken.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        call_fails(&client, "alpha_echo", serde_json::json!({ "text": "x" })).await;
    }
    broken.store(false, Ordering::SeqCst);

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if monitor.reconnect_count() >= 1
            && let Ok(result) = client
                .call_tool("alpha_echo", serde_json::json!({ "text": "back" }))
                .await
            && !result.is_error
        {
            assert_eq!(result.all_text(), "back");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "HTTP 500s must still rebuild the transport (reconnects = {})",
            monitor.reconnect_count()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    gateway_task.abort();
    backend_task.abort();
}

#[test]
fn only_transport_errors_are_classified_as_transport_failures() {
    for message in [
        "Backend error: Transport error: HTTP request failed: error sending request",
        "Backend error: Transport error: Connection closed",
        "Backend error: Session expired",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32000, message: \"server returned HTTP 500\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32005, message: \"session not found\", data: None }",
        // Python MCP SDK after a restart: HTTP 404 + -32600 "Session not found".
        "Backend error: JSON-RPC error: JsonRpcError { code: -32600, message: \"Session not found\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32600, message: \"SESSION NOT FOUND\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32603, message: \"session not found\", data: Some(Object {}) }",
        "request timed out",
    ] {
        assert!(is_backend_transport_failure(message), "{message}");
    }
    for message in [
        "Backend error: JSON-RPC error: JsonRpcError { code: -32602, message: \"invalid params\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32601, message: \"no such method\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32603, message: \"tool failed: unknown session\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32603, message: \"tool failed: session not found\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32600, message: \"Invalid request\", data: None }",
        "Backend error: JSON-RPC error: JsonRpcError { code: -32602, message: \"Session not found in workspace\", data: None }",
        "Session not found",
        "Unknown tool: alpha_echo",
        "Backend 'alpha' is reconnecting; the tool call was not sent to it. Retry shortly.",
    ] {
        assert!(!is_backend_transport_failure(message), "{message}");
    }
}

/// A Python MCP SDK streamable HTTP server (e.g. Praxiom) that restarted:
/// while `lost` is set, every request that carries a session id gets HTTP 404
/// with `-32600 "Session not found"` (the SDK's exact reply). The first
/// request without a session id (a fresh `initialize`) clears it.
async fn spawn_restarting_backend(
    lost: Arc<AtomicBool>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let echo = ToolBuilder::new("echo")
        .description("Echo the text")
        .handler(|input: EchoInput| async move { Ok(CallToolResult::text(input.text)) })
        .build();
    let router = McpRouter::new()
        .server_info("fixture-restart", "1.0.0")
        .tool(echo);
    let app = HttpTransport::new(router)
        .into_router_at("/mcp")
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let lost = lost.clone();
            async move {
                if lost.load(Ordering::SeqCst) {
                    if request.headers().contains_key("mcp-session-id") {
                        return (
                            StatusCode::NOT_FOUND,
                            [(header::CONTENT_TYPE, "application/json")],
                            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Session not found"}}"#,
                        )
                            .into_response();
                    }
                    lost.store(false, Ordering::SeqCst);
                }
                next.run(request).await
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backend");
    let addr = listener.local_addr().expect("backend address");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve backend");
    });
    (addr, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_not_found_after_backend_restart_rebuilds_the_transport() {
    let lost = Arc::new(AtomicBool::new(false));
    let (backend, backend_task) = spawn_restarting_backend(lost.clone()).await;
    let (gateway, monitor, gateway_task) = serve_gateway(backend).await;
    let client = connect_client(gateway).await;

    let ok = client
        .call_tool("alpha_echo", serde_json::json!({ "text": "before" }))
        .await
        .expect("call before the restart");
    assert_eq!(ok.all_text(), "before");

    // The backend restarts and forgets the gateway's session.
    lost.store(true, Ordering::SeqCst);
    let error = call_fails(&client, "alpha_echo", serde_json::json!({ "text": "x" })).await;
    assert!(error.contains("Session not found"), "{error}");

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(result) = client
            .call_tool("alpha_echo", serde_json::json!({ "text": "back" }))
            .await
            && !result.is_error
        {
            assert_eq!(result.all_text(), "back");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "-32600 Session not found must rebuild the transport (reconnects = {})",
            monitor.reconnect_count()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(monitor.reconnect_count() >= 1);
    assert!(!lost.load(Ordering::SeqCst), "the gateway re-initialized");

    gateway_task.abort();
    backend_task.abort();
}

// --- "Unknown tool" while a transport is being rebuilt ---------------------

fn call_request(name: &str) -> RouterRequest {
    RouterRequest {
        id: RequestId::Number(1),
        inner: McpRequest::CallTool(CallToolParams {
            name: name.to_string(),
            arguments: serde_json::json!({}),
            input_responses: None,
            request_state: None,
            meta: None,
            task: None,
        }),
        extensions: Extensions::new(),
    }
}

/// Inner service answering with `replies` in order (the last one repeats).
fn scripted(
    replies: Vec<Result<&'static str, String>>,
) -> (
    BoxCloneService<RouterRequest, RouterResponse, Infallible>,
    Arc<AtomicUsize>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let replies = Arc::new(replies);
    let service = tower::service_fn(move |request: RouterRequest| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let reply = replies[n.min(replies.len() - 1)].clone();
        async move {
            Ok::<_, Infallible>(RouterResponse {
                id: request.id,
                inner: match reply {
                    Ok(text) => Ok(McpResponse::CallTool(CallToolResult::text(text))),
                    Err(message) => Err(JsonRpcError::invalid_params(message)),
                },
            })
        }
    });
    (BoxCloneService::new(service), calls)
}

fn observer(
    service: BoxCloneService<RouterRequest, RouterResponse, Infallible>,
    tracker: &ReconnectTracker,
    wait: Duration,
) -> (TransportErrorObserver, broadcast::Receiver<String>) {
    let (tx, rx) = broadcast::channel(8);
    let observer = TransportErrorObserver::new(
        service,
        tx,
        vec![("alpha_".to_string(), "alpha".to_string())],
        tracker.clone(),
    )
    .with_reconnect_wait(wait);
    (observer, rx)
}

fn text_of(response: RouterResponse) -> Result<String, String> {
    match response.inner {
        Ok(McpResponse::CallTool(result)) => Ok(result.all_text()),
        Ok(_) => Err("unexpected response".to_string()),
        Err(error) => Err(error.message),
    }
}

#[tokio::test]
async fn a_call_during_a_rebuild_is_sent_after_the_rebuild() {
    let tracker = ReconnectTracker::default();
    let (service, calls) = scripted(vec![Err("Unknown tool: alpha_echo".into()), Ok("done")]);
    let (observer, mut rx) = observer(service, &tracker, Duration::from_secs(5));

    let rebuilding = tracker.begin("alpha");
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(rebuilding);
    });
    let response = observer
        .oneshot(call_request("alpha_echo"))
        .await
        .expect("infallible");
    release.await.expect("release task");

    assert_eq!(text_of(response), Ok("done".to_string()));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(rx.try_recv().is_err(), "no transport failure is reported");
}

#[tokio::test]
async fn a_rebuild_that_does_not_settle_gives_a_clear_transient_error() {
    let tracker = ReconnectTracker::default();
    let (service, calls) = scripted(vec![Err("Unknown tool: alpha_echo".into())]);
    let (observer, _rx) = observer(service, &tracker, Duration::from_millis(100));

    let _rebuilding = tracker.begin("alpha");
    let response = observer
        .oneshot(call_request("alpha_echo"))
        .await
        .expect("infallible");

    let error = text_of(response).expect_err("still rebuilding");
    assert!(error.contains("reconnecting"), "{error}");
    assert!(!error.contains("Unknown tool"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "never sent while absent");
}

#[tokio::test]
async fn unknown_tool_without_a_rebuild_is_unchanged_and_not_retried() {
    let tracker = ReconnectTracker::default();
    let (service, calls) = scripted(vec![Err("Unknown tool: alpha_nope".into()), Ok("x")]);
    let (observer, _rx) = observer(service, &tracker, Duration::from_secs(5));

    let response = observer
        .oneshot(call_request("alpha_nope"))
        .await
        .expect("infallible");

    assert_eq!(
        text_of(response),
        Err("Unknown tool: alpha_nope".to_string())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn backend_error_replies_are_not_reported_but_transport_errors_are() {
    let tracker = ReconnectTracker::default();
    let app_error = "Backend error: JSON-RPC error: JsonRpcError { code: -32602, message: \"bad\", data: None }";
    let (service, _) = scripted(vec![Err(app_error.into())]);
    let (observer_app, mut rx_app) = observer(service, &tracker, Duration::from_secs(1));
    let response = observer_app
        .oneshot(call_request("alpha_echo"))
        .await
        .expect("infallible");
    assert_eq!(text_of(response), Err(app_error.to_string()));
    assert!(rx_app.try_recv().is_err());

    let (service, _) = scripted(vec![Err(
        "Backend error: Transport error: Connection closed".into(),
    )]);
    let (observer_tx, mut rx_tx) = observer(service, &tracker, Duration::from_secs(1));
    observer_tx
        .oneshot(call_request("alpha_echo"))
        .await
        .expect("infallible");
    assert_eq!(rx_tx.try_recv().ok().as_deref(), Some("alpha"));
}
