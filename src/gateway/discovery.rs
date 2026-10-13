//! Handle MCP 2026-07-28 stateless discovery at the gateway boundary.
//!
//! The pinned mcp-proxy backend implements tools/list but returns -32601 for
//! server/discover. The outer OAuth middleware MUST authenticate first.
//! No catalog is exposed here; individual client allowlists still constrain
//! tools/list and tools/call independently.

use axum::{
    Json,
    body::to_bytes,
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

const MODERN_VERSION: &str = "2026-07-28";

pub(super) async fn server_discover(request: Request, next: Next) -> Response {
    if request.method() != Method::POST {
        return next.run(request).await;
    }
    // For other JSON-RPC methods restore the complete original request.
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, 1_048_576).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
        return next
            .run(Request::from_parts(parts, axum::body::Body::from(bytes)))
            .await;
    };
    if payload.get("method").and_then(Value::as_str) != Some("server/discover") {
        return next
            .run(Request::from_parts(parts, axum::body::Body::from(bytes)))
            .await;
    }
    let header_method = parts
        .headers
        .get("mcp-method")
        .and_then(|v| v.to_str().ok());
    let header_version = parts
        .headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok());
    let meta_version = payload
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str);
    let valid_id = payload
        .get("id")
        .is_some_and(|id| id.is_number() || id.is_string());
    if payload.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || !valid_id
        || header_method.is_some_and(|method| method != "server/discover")
        || header_version.is_some_and(|version| version != MODERN_VERSION)
        || meta_version.is_some_and(|version| version != MODERN_VERSION)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": payload["id"],
            "result": {
                "resultType": "complete",
                "ttlMs": 0,
                "cacheScope": "private",
                "supportedVersions": [MODERN_VERSION],
                "capabilities": { "tools": {}, "resources": {} },
                "_meta": {
                    "io.modelcontextprotocol/serverInfo": {
                        "name": "lomway",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request as HttpRequest, middleware, routing::post};

    async fn app() -> Router {
        Router::new()
            .route("/", post(|| async { StatusCode::IM_A_TEAPOT }))
            .layer(middleware::from_fn(server_discover))
    }

    #[tokio::test]
    async fn returns_version_and_identity_for_modern_discovery() {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/")
            .header("content-type", "application/json")
            .header("mcp-method", "server/discover")
            .header("mcp-protocol-version", MODERN_VERSION)
            .body(Body::from(r#"{"jsonrpc":"2.0","id":7,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app().await, request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload["id"], 7);
        assert_eq!(
            payload["result"]["supportedVersions"],
            json!([MODERN_VERSION])
        );
        assert_eq!(
            payload["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "lomway"
        );
        assert_eq!(payload["result"]["resultType"], "complete");
        assert_eq!(payload["result"]["ttlMs"], 0);
        assert_eq!(payload["result"]["cacheScope"], "private");
        assert!(payload["result"].get("tools").is_none());
    }

    #[tokio::test]
    async fn does_not_intercept_other_methods() {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
            ))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app().await, request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    }

    #[tokio::test]
    async fn refuses_conflicting_protocol_version() {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/")
            .header("mcp-protocol-version", "2025-03-26")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"server/discover"}"#,
            ))
            .unwrap();
        let response = tower::ServiceExt::oneshot(app().await, request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
