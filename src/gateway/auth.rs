use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};

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
    aud: Option<Value>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    exp: Option<u64>,
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
    next.run(request).await
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
    if !token.scope.as_deref().is_some_and(|scopes| {
        scopes
            .split_ascii_whitespace()
            .any(|scope| scope == state.config.required_scope)
    }) {
        return false;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |duration| duration.as_secs());
    token.exp.is_some_and(|exp| exp > now)
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
                            "exp": now + 3600
                        }),
                        Some("wrong-aud") => json!({
                            "active": true,
                            "aud": "https://other.example/mcp",
                            "scope": "devspace",
                            "exp": now + 3600
                        }),
                        Some("wrong-scope") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "other",
                            "exp": now + 3600
                        }),
                        Some("expired") => json!({
                            "active": true,
                            "aud": resource,
                            "scope": "devspace",
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
            ("wrong-scope", reqwest::StatusCode::UNAUTHORIZED),
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
    }
}
