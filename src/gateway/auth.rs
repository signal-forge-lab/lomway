use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::model::NorthboundOAuthConfig;

#[derive(Clone)]
struct OAuthState {
    config: NorthboundOAuthConfig,
    client: reqwest::Client,
    metadata_url: String,
    authorization_server: String,
}

#[derive(Debug, Deserialize)]
struct IntrospectionResponse {
    active: bool,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    aud: Option<Value>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    pat: bool,
    #[serde(default)]
    oauth_unrestricted_legacy: bool,
    #[serde(default)]
    allowed_tools: Vec<String>,
}

pub(super) fn protect_mcp_router(
    router: Router,
    config: NorthboundOAuthConfig,
) -> (Router, Router) {
    let resource =
        reqwest::Url::parse(&config.resource_url).expect("validated OAuth resource URL must parse");
    let authorization_server = format!("{}/", resource.origin().ascii_serialization());
    let metadata_url = format!(
        "{authorization_server}.well-known/oauth-protected-resource{}",
        resource.path()
    );
    let metadata_path = format!("/.well-known/oauth-protected-resource{}", resource.path());
    let state = OAuthState {
        config,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("building local introspection client"),
        metadata_url,
        authorization_server,
    };

    let protected = router.layer(middleware::from_fn_with_state(state.clone(), require_oauth));
    let metadata = Router::new()
        .route(&metadata_path, get(protected_resource_metadata))
        .with_state(state);
    (protected, metadata)
}

async fn protected_resource_metadata(State(state): State<OAuthState>) -> Json<Value> {
    Json(json!({
        "resource": state.config.resource_url,
        "authorization_servers": [state.authorization_server],
        "scopes_supported": [state.config.required_scope],
        "bearer_methods_supported": ["header"]
    }))
}

async fn require_oauth(State(state): State<OAuthState>, request: Request, next: Next) -> Response {
    let Some(token) = bearer_token(&request) else {
        return unauthorized(&state);
    };

    let introspection = match state
        .client
        .post(&state.config.introspection_url)
        .form(&[("token", token)])
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            match response.json::<IntrospectionResponse>().await {
                Ok(response) => response,
                Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            }
        }
        _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };

    if !token_is_authorized(&state, &introspection) {
        return unauthorized(&state);
    }
    if !token_has_required_scope(&state, &introspection) {
        return insufficient_scope(&state);
    }
    let Some(client_id) = introspection.client_id.as_deref() else {
        return unauthorized(&state);
    };
    let actor = format!("{:x}", Sha256::digest(client_id.as_bytes()));
    let actor = &actor[..16];
    if introspection.pat || !introspection.oauth_unrestricted_legacy {
        return restricted_pat_request(request, next, &introspection.allowed_tools, actor).await;
    }
    unrestricted_audited_request(request, next, actor).await
}

async fn unrestricted_audited_request(request: Request, next: Next, actor: &str) -> Response {
    // Audit tool names, never token values or request arguments.
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, 1_048_576).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let message = serde_json::from_slice::<Value>(&bytes).ok();
    let method = message
        .as_ref()
        .and_then(|v| v.get("method"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let method = audit_field(method);
    let tool = message
        .as_ref()
        .and_then(|v| v.pointer("/params/name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool = audit_field(tool);
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    tracing::info!(actor, method, tool, decision="legacy_allow",
        http_status=%response.status(), "mcp authorization audit");
    response
}

/// PATs do not inherit the OAuth owner's unrestricted capability catalog.
/// The MCP request and tools/list response are both enforced, including SSE.
async fn restricted_pat_request(
    request: Request,
    next: Next,
    allowed: &[String],
    actor: &str,
) -> Response {
    if request.method() != Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, 1_048_576).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // Batch JSON-RPC and opaque calls are deliberately denied.
        return StatusCode::FORBIDDEN.into_response();
    };
    let tool = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool = audit_field(tool);
    let deferred_target = audit_field(
        message
            .pointer("/params/arguments/name")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    let should_filter = match method {
        "initialize" | "ping" | "notifications/initialized" | "notifications/cancelled" => false,
        "tools/list" => true,
        "tools/call" => {
            let name = message.pointer("/params/name").and_then(Value::as_str);
            if !name.is_some_and(|name| permitted_pat_call(name, &message, allowed)) {
                tracing::info!(
                    actor,
                    method = audit_field(method),
                    tool,
                    deferred_target,
                    decision = "deny",
                    "mcp authorization audit"
                );
                return StatusCode::FORBIDDEN.into_response();
            }
            false
        }
        _ => {
            tracing::info!(
                actor,
                method = audit_field(method),
                tool,
                deferred_target,
                decision = "deny",
                "mcp authorization audit"
            );
            return StatusCode::FORBIDDEN.into_response();
        }
    };
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let response = if should_filter {
        filter_pat_tools_list(response, allowed).await
    } else {
        response
    };
    tracing::info!(actor, method = audit_field(method), tool, deferred_target, decision="restricted",
        http_status=%response.status(), "mcp authorization audit");
    response
}

fn permitted_pat_call(name: &str, message: &Value, allowed: &[String]) -> bool {
    if !allowed.iter().any(|tool| tool == name) {
        return false;
    }
    if name == "lomway_call_tool" {
        // No unrestricted deferred dispatch. The target itself must also be
        // explicitly whitelisted, even though it is hidden from tools/list.
        return message
            .pointer("/params/arguments/name")
            .and_then(Value::as_str)
            .is_some_and(|target| {
                target != "lomway_call_tool" && allowed.iter().any(|tool| tool == target)
            });
    }
    true
}

fn audit_field(value: &str) -> &str {
    if value.len() <= 128
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-' | b'.' | b':' | b'/'))
    {
        value
    } else {
        "[redacted]"
    }
}

fn restrict_tool_catalog(message: &mut Value, allowed: &[String]) -> bool {
    let Some(tools) = message
        .pointer_mut("/result/tools")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    tools.retain(|tool| {
        tool.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| allowed.iter().any(|item| item == name))
    });
    true
}

async fn filter_pat_tools_list(response: Response, allowed: &[String]) -> Response {
    if !response.status().is_success() {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, 4_194_304)).await
    {
        Ok(Ok(bytes)) => bytes,
        _ => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let encoded = if content_type.starts_with("application/json") {
        let Ok(mut json) = serde_json::from_slice::<Value>(&bytes) else {
            return StatusCode::BAD_GATEWAY.into_response();
        };
        if !restrict_tool_catalog(&mut json, allowed) {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        serde_json::to_vec(&json).ok()
    } else if content_type.starts_with("text/event-stream") {
        let Ok(sse) = std::str::from_utf8(&bytes) else {
            return StatusCode::BAD_GATEWAY.into_response();
        };
        let mut found = false;
        let mut output = String::new();
        for line in sse.split_inclusive('\n') {
            if let Some(data) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:") {
                let Ok(mut message) = serde_json::from_str::<Value>(data.trim_start()) else {
                    return StatusCode::BAD_GATEWAY.into_response();
                };
                if restrict_tool_catalog(&mut message, allowed) {
                    found = true;
                }
                output.push_str("data: ");
                match serde_json::to_string(&message) {
                    Ok(text) => output.push_str(&text),
                    Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
                }
                output.push('\n');
            } else {
                output.push_str(line);
            }
        }
        if !found {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Some(output.into_bytes())
    } else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let Some(encoded) = encoded else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(encoded))
}

fn bearer_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

fn token_is_authorized(state: &OAuthState, token: &IntrospectionResponse) -> bool {
    if !token.active || !audience_matches(token.aud.as_ref(), &state.config.resource_url) {
        return false;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |duration| duration.as_secs());
    token.exp.is_some_and(|exp| exp > now)
}

fn token_has_required_scope(state: &OAuthState, token: &IntrospectionResponse) -> bool {
    token.scope.as_deref().is_some_and(|scopes| {
        scopes
            .split_ascii_whitespace()
            .any(|scope| scope == state.config.required_scope)
    })
}

fn audience_matches(audience: Option<&Value>, expected: &str) -> bool {
    match audience {
        Some(Value::String(value)) => value == expected,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(expected)),
        _ => false,
    }
}

fn unauthorized(state: &OAuthState) -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    let challenge = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\"",
        state.metadata_url, state.config.required_scope
    );
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

fn insufficient_scope(state: &OAuthState) -> Response {
    let mut response = StatusCode::FORBIDDEN.into_response();
    let challenge = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\", error=\"insufficient_scope\"",
        state.metadata_url, state.config.required_scope
    );
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::{Form, Router, http::StatusCode, routing::post};
    use serde_json::json;

    use super::*;

    async fn spawn_introspection(resource: String) -> std::net::SocketAddr {
        let app = Router::new().route(
            "/oauth/introspect",
            post(move |Form(form): Form<HashMap<String, String>>| {
                let resource = resource.clone();
                async move {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("clock")
                        .as_secs();
                    let response = match form.get("token").map(String::as_str) {
                        Some("valid") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "devspace",
                            "exp": now + 3600,
                            "client_id": "legacy-client",
                            "oauth_unrestricted_legacy": true
                        }),
                        Some("oauth-limited") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "devspace",
                            "exp": now + 3600,
                            "client_id": "new-client",
                            "allowed_tools": ["safe_status"]
                        }),
                        Some("pat") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "devspace",
                            "exp": now + 3600,
                            "pat": true,
                            "client_id": "pat:synthetic",
                            "allowed_tools": ["safe_status", "lomway_call_tool"]
                        }),
                        Some("wrong-aud") => json!({
                            "active": true,
                            "aud": "https://other.example/mcp",
                            "scope": "devspace",
                            "client_id": "wrong-client",
                            "exp": now + 3600
                        }),
                        Some("wrong-scope") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "other",
                            "client_id": "wrong-client",
                            "exp": now + 3600
                        }),
                        Some("expired") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "devspace",
                            "client_id": "expired-client",
                            "exp": now.saturating_sub(1)
                        }),
                        _ => json!({ "active": false }),
                    };
                    Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind introspection");
        let addr = listener.local_addr().expect("introspection addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve introspection");
        });
        addr
    }

    #[tokio::test]
    async fn oauth_protects_mcp_and_serves_public_resource_metadata() {
        let resource = "https://lomway.example/mcp".to_string();
        let introspection = spawn_introspection(resource.clone()).await;
        let config = NorthboundOAuthConfig {
            resource_url: resource.clone(),
            introspection_url: format!("http://{introspection}/oauth/introspect"),
            required_scope: "devspace".to_string(),
        };
        let protected = Router::new().fallback(|| async { StatusCode::OK });
        let (protected, metadata) = protect_mcp_router(protected, config);
        let app = Router::new().merge(metadata).nest("/mcp", protected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind protected app");
        let addr = listener.local_addr().expect("protected addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve protected app");
        });

        let client = reqwest::Client::new();
        let metadata = client
            .get(format!(
                "http://{addr}/.well-known/oauth-protected-resource/mcp"
            ))
            .send()
            .await
            .expect("metadata request");
        assert_eq!(metadata.status(), reqwest::StatusCode::OK);
        let metadata: Value = metadata.json().await.expect("metadata json");
        assert_eq!(metadata["resource"], resource);
        assert_eq!(
            metadata["authorization_servers"][0],
            "https://lomway.example/"
        );
        assert_eq!(metadata["scopes_supported"][0], "devspace");

        let unauthenticated = client
            .post(format!("http://{addr}/mcp"))
            .send()
            .await
            .expect("unauthenticated request");
        assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
        let challenge = unauthenticated
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .expect("challenge")
            .to_str()
            .expect("challenge text");
        assert!(challenge.contains(
            "resource_metadata=\"https://lomway.example/.well-known/oauth-protected-resource/mcp\""
        ));

        for (token, expected) in [
            ("valid", reqwest::StatusCode::OK),
            ("invalid", reqwest::StatusCode::UNAUTHORIZED),
            ("wrong-aud", reqwest::StatusCode::UNAUTHORIZED),
            ("wrong-scope", reqwest::StatusCode::FORBIDDEN),
            ("expired", reqwest::StatusCode::UNAUTHORIZED),
        ] {
            let response = client
                .post(format!("http://{addr}/mcp"))
                .bearer_auth(token)
                .send()
                .await
                .expect("protected request");
            assert_eq!(response.status(), expected, "token {token}");
        }

        let response = client
            .post(format!("http://{addr}/mcp"))
            .bearer_auth("wrong-scope")
            .send()
            .await
            .expect("insufficient-scope request");
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(
            response
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .expect("insufficient-scope challenge")
                .to_str()
                .expect("challenge text"),
            "Bearer resource_metadata=\"https://lomway.example/.well-known/oauth-protected-resource/mcp\", scope=\"devspace\", error=\"insufficient_scope\""
        );
    }

    #[tokio::test]
    async fn pat_allows_only_approved_calls_and_hides_unapproved_tools() {
        let resource = "https://lomway.example/mcp".to_string();
        let introspection = spawn_introspection(resource.clone()).await;
        let config = NorthboundOAuthConfig {
            resource_url: resource,
            introspection_url: format!("http://{introspection}/oauth/introspect"),
            required_scope: "devspace".to_string(),
        };
        let all_tools = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [
                { "name": "safe_status" },
                { "name": "dangerous_mutation" },
                { "name": "lomway_call_tool" }
            ]}
        });
        let upstream = Router::new().fallback(move || {
            let tools = all_tools.clone();
            async move { Json(tools) }
        });
        let (upstream, _) = protect_mcp_router(upstream, config);
        let app = Router::new().nest("/mcp", upstream);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let client = reqwest::Client::new();
        let base = format!("http://{addr}/mcp");
        for method in [
            "initialize",
            "ping",
            "notifications/initialized",
            "tools/call",
        ] {
            let response = client.post(&base).bearer_auth("pat")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":{"name":"safe_status"}}))
                .send().await.expect("allowed method");
            assert_eq!(response.status(), reqwest::StatusCode::OK, "{method}");
        }
        let deferred_safe = client
            .post(&base)
            .bearer_auth("pat")
            .json(
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
                    "name":"lomway_call_tool","arguments":{"name":"safe_status","arguments":{}}
                }}),
            )
            .send()
            .await
            .expect("approved deferred invocation");
        assert_eq!(deferred_safe.status(), reqwest::StatusCode::OK);
        let deferred_forbidden = client.post(&base).bearer_auth("pat")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
                "name":"lomway_call_tool","arguments":{"name":"dangerous_mutation","arguments":{}}
            }}))
            .send().await.expect("unapproved deferred invocation");
        assert_eq!(deferred_forbidden.status(), reqwest::StatusCode::FORBIDDEN);
        for (method, name) in [
            ("tools/call", "dangerous_mutation"),
            ("tools/call", "lomway_call_tool"),
            ("resources/list", ""),
        ] {
            let response = client
                .post(&base)
                .bearer_auth("pat")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":{"name":name}}))
                .send()
                .await
                .expect("restricted method");
            assert_eq!(
                response.status(),
                reqwest::StatusCode::FORBIDDEN,
                "{method} {name}"
            );
        }
        let response = client.post(&base).bearer_auth("pat")
            .json(&json!([{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"safe_status"}}]))
            .send().await.expect("batch request");
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        let tools: Value = client
            .post(&base)
            .bearer_auth("pat")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .expect("filtered list")
            .json()
            .await
            .expect("json");
        let names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(names, vec!["safe_status", "lomway_call_tool"]);

        // OAuth is not automatically unrestricted: a new DCR client sees
        // only the tools authorized by its explicit client policy.
        let oauth_tools: Value = client
            .post(&base)
            .bearer_auth("oauth-limited")
            .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .send()
            .await
            .expect("limited OAuth list")
            .json()
            .await
            .expect("json");
        let oauth_names: Vec<&str> = oauth_tools["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(oauth_names, vec!["safe_status"]);
        let forbidden = client
            .post(&base)
            .bearer_auth("oauth-limited")
            .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"dangerous_mutation"}}))
            .send()
            .await
            .expect("limited OAuth request");
        assert_eq!(forbidden.status(), reqwest::StatusCode::FORBIDDEN);
        let allowed = client
            .post(&base)
            .bearer_auth("oauth-limited")
            .json(&json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
                "params":{"name":"safe_status"}}))
            .send()
            .await
            .expect("limited OAuth request");
        assert_eq!(allowed.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn pat_filters_sse_tool_lists_fail_closed() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[{\"name\":\"safe_status\"},{\"name\":\"dangerous_mutation\"}]}}\n\n";
        let response = Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from(sse))
            .expect("response");
        let filtered = filter_pat_tools_list(response, &["safe_status".into()]).await;
        assert_eq!(filtered.status(), StatusCode::OK);
        let bytes = to_bytes(filtered.into_body(), 4096).await.expect("body");
        let text = String::from_utf8(bytes.to_vec()).expect("UTF-8");
        assert!(text.contains("safe_status"));
        assert!(!text.contains("dangerous_mutation"));
        let unknown = Response::builder()
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("unfiltered catalog"))
            .expect("response");
        assert_eq!(
            filter_pat_tools_list(unknown, &[]).await.status(),
            StatusCode::BAD_GATEWAY
        );
    }
}
