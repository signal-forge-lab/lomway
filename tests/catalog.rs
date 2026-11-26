//! `lomway catalog` E2E against localhost mock MCP backends.
//!
//! Proves the catalog is built from one read-only probe per backend, that
//! unavailable backends (required or optional) are reported instead of
//! aborting, that snapshots carry no URLs, and that the snapshot diff
//! detects added and changed tools without misreporting an outage.

use axum::serve;
use lomway::cli::catalog::{self, BackendStatus};
use lomway::config::model::{BackendExposure, GatewayConfig};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower_mcp::{CallToolResult, HttpTransport, McpRouter, ToolBuilder};

#[derive(Debug, Deserialize, JsonSchema)]
struct EmptyInput {}

/// Serve tools named `names`, each with `description`. Every tool call is
/// counted so the test can prove the catalog never invokes a tool.
async fn spawn_backend(
    names: &[&'static str],
    description: &'static str,
    calls: Arc<AtomicUsize>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let mut router = McpRouter::new().server_info("catalog-mock", "1.0.0");
    for name in names {
        let calls = calls.clone();
        router = router.tool(
            ToolBuilder::new(*name)
                .description(description)
                .handler(move |_: EmptyInput| {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(CallToolResult::text("ok"))
                    }
                })
                .build(),
        );
    }
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

fn dead_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}

fn config(alpha: u16, beta: u16, gamma: u16) -> GatewayConfig {
    toml::from_str(&format!(
        r#"
schema_version = 1

[[backends]]
id = "alpha"
prefix = "alpha_"
url = "http://127.0.0.1:{alpha}/mcp"
timeout_seconds = 5

[[backends]]
id = "beta"
prefix = "beta_"
url = "http://127.0.0.1:{beta}/mcp"
exposure = "deferred"
required = false
timeout_seconds = 5

[[backends]]
id = "gamma"
prefix = "gamma_"
url = "http://127.0.0.1:{gamma}/mcp"
required = false
timeout_seconds = 2
"#
    ))
    .expect("parse catalog test config")
}

#[tokio::test]
async fn catalog_reports_exposure_availability_and_diffs_snapshots() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (alpha, alpha_task) =
        spawn_backend(&["status", "write"], "catalog-desc-one", calls.clone()).await;
    let (beta, beta_task) = spawn_backend(
        &["open", "close", "click"],
        "catalog-desc-one",
        calls.clone(),
    )
    .await;
    let gamma = dead_port();

    let current = catalog::collect(&config(alpha.port(), beta.port(), gamma))
        .await
        .expect("catalog with an unavailable optional backend");
    let ids: Vec<_> = current.backends.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["alpha", "beta", "gamma"], "configuration order");
    assert_eq!(current.direct_tool_count(), 2);
    assert_eq!(current.deferred_tool_count(), 3);
    assert_eq!(current.backends[1].exposure, BackendExposure::Deferred);
    assert_eq!(current.backends[2].status, BackendStatus::Unavailable);
    assert_eq!(
        catalog::render_names(&current),
        "alpha_status\nalpha_write\nbeta_click\nbeta_close\nbeta_open\n"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "catalog must never call a tool"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let baseline = dir.path().join("baseline.json");
    catalog::write_snapshot(&current, &baseline).expect("write baseline");
    let text = std::fs::read_to_string(&baseline).expect("read baseline");
    assert!(
        !text.contains("127.0.0.1"),
        "snapshots must not contain URLs: {text}"
    );
    assert!(
        !text.contains("catalog-desc-one"),
        "snapshots must not contain descriptions"
    );

    // alpha changes a description and adds a tool; beta goes down.
    alpha_task.abort();
    beta_task.abort();
    let (alpha2, alpha2_task) = spawn_backend(
        &["status", "write", "delete"],
        "catalog-desc-two",
        calls.clone(),
    )
    .await;
    let next = catalog::collect(&config(alpha2.port(), dead_port(), gamma))
        .await
        .expect("second catalog");
    let changes = catalog::diff(
        &catalog::CatalogSnapshot::load(&baseline).expect("load baseline"),
        &next,
    );
    assert_eq!(changes.tools_added, ["alpha_delete"]);
    assert_eq!(changes.tools_changed, ["alpha_status", "alpha_write"]);
    assert!(
        changes.tools_removed.is_empty(),
        "a backend outage must not be reported as removed tools: {changes:?}"
    );
    assert_eq!(changes.not_compared, ["beta", "gamma"]);
    assert!(catalog::enforce_no_change(&changes, true).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    alpha2_task.abort();
}

#[tokio::test]
async fn an_unavailable_required_backend_is_reported_not_fatal() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (beta, beta_task) = spawn_backend(&["open"], "catalog-desc-one", calls).await;
    let current = catalog::collect(&config(dead_port(), beta.port(), dead_port()))
        .await
        .expect("catalog reports even when a required backend is down");
    assert_eq!(current.backends[0].status, BackendStatus::Unavailable);
    assert!(current.backends[0].required);
    let text = catalog::render_text(&current);
    assert!(
        text.contains("alpha (required: `serve` would fail closed)"),
        "{text}"
    );
    beta_task.abort();
}
