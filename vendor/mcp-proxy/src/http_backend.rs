//! HTTP backend transport construction.
//!
//! Lomway patch (not upstream): upstream mcp-proxy built every HTTP backend
//! with `HttpClientTransport::new`, whose reqwest per-request timeout is
//! tower-mcp's fixed 30 s default. The configured per-backend `timeout`
//! only armed an outer tower `TimeoutLayer`, so a backend call that took
//! longer than 30 s failed with "error sending request" even when the
//! backend allowed 300 s. Because tower-mcp treats any send error as a dead
//! connection, that one slow call also failed every request queued behind it
//! and closed the backend client until the gateway rebuilt it.
//!
//! Every HTTP backend transport is built here so the request timeout follows
//! the backend's configured timeout.

use std::time::Duration;

use tower_mcp::client::{HttpClientConfig, HttpClientTransport};

/// Client settings for one HTTP backend.
///
/// `timeout_seconds` is the backend's configured request timeout. When it is
/// absent or zero the tower-mcp default is kept unchanged.
pub fn http_client_config(timeout_seconds: Option<u64>) -> HttpClientConfig {
    let mut config = HttpClientConfig::default();
    if let Some(seconds) = timeout_seconds.filter(|seconds| *seconds > 0) {
        config.request_timeout = Duration::from_secs(seconds);
    }
    config
}

/// Build the transport for one HTTP backend.
pub fn http_backend_transport(
    url: &str,
    bearer_token: Option<&str>,
    timeout_seconds: Option<u64>,
) -> HttpClientTransport {
    let mut transport = HttpClientTransport::with_config(url, http_client_config(timeout_seconds));
    if let Some(token) = bearer_token {
        transport = transport.bearer_token(token);
    }
    transport
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_timeout_follows_the_backend_timeout() {
        assert_eq!(
            http_client_config(Some(300)).request_timeout,
            Duration::from_secs(300)
        );
        assert_eq!(
            http_client_config(Some(5)).request_timeout,
            Duration::from_secs(5)
        );
    }

    #[test]
    fn missing_or_zero_timeout_keeps_the_upstream_default() {
        let default = HttpClientConfig::default().request_timeout;
        assert_eq!(http_client_config(None).request_timeout, default);
        assert_eq!(http_client_config(Some(0)).request_timeout, default);
    }

    #[test]
    fn notification_timeout_is_unchanged() {
        assert_eq!(
            http_client_config(Some(300)).notification_timeout,
            HttpClientConfig::default().notification_timeout
        );
    }
}
