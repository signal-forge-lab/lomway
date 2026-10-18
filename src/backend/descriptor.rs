//! Validated backend descriptor.
//!
//! A descriptor can only be created from a configuration entry that passes
//! the public policy validators, so no unvalidated URL, prefix, or id can
//! ever reach the gateway runtime.

use std::time::Duration;

use anyhow::Result;

use crate::config::model::{BackendEntry, BackendToken};
use crate::config::validate;

/// A fully validated backend description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendDescriptor {
    id: String,
    prefix: String,
    url: String,
    required: bool,
    timeout: Duration,
    /// Redacted in `Debug`; only the transport reads the value.
    bearer_token: Option<BackendToken>,
}

impl BackendDescriptor {
    /// Construct a descriptor from a configuration entry, re-running every
    /// public validator. Unvalidated entries are rejected here.
    pub fn try_from_entry(entry: &BackendEntry) -> Result<Self> {
        validate::validate_backend_id(&entry.id)?;
        validate::validate_backend_prefix(&entry.prefix)?;
        validate::validate_backend_url(&entry.url)?;
        anyhow::ensure!(
            (1..=validate::MAX_BACKEND_TIMEOUT_SECONDS).contains(&entry.timeout_seconds),
            "backend '{}' timeout_seconds must be between 1 and {}",
            entry.id,
            validate::MAX_BACKEND_TIMEOUT_SECONDS
        );
        crate::namespace::mcp_name_from_prefix(&entry.prefix).ok_or_else(|| {
            anyhow::anyhow!(
                "backend '{}' prefix {:?} is not a valid namespace prefix: it must end with the '_' separator and contain a non-empty stem",
                entry.id,
                entry.prefix
            )
        })?;
        match (&entry.bearer_token_env, &entry.bearer_token) {
            (Some(name), Some(_)) => validate::validate_bearer_token_env_name(&entry.id, name)?,
            (None, None) => {}
            _ => anyhow::bail!(
                "backend '{}' bearer_token_env must be set and resolved together; the gateway fails closed instead of connecting without credentials",
                entry.id
            ),
        }
        Ok(Self {
            id: entry.id.clone(),
            prefix: entry.prefix.clone(),
            url: entry.url.clone(),
            required: entry.required,
            timeout: Duration::from_secs(entry.timeout_seconds.max(1)),
            bearer_token: entry.bearer_token.clone(),
        })
    }

    /// Stable backend identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Namespace prefix applied to this backend's tools.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Loopback Streamable HTTP MCP endpoint.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether an outage at startup must fail the gateway.
    pub fn required(&self) -> bool {
        self.required
    }

    /// Request and probe timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Bearer token sent to this backend, if one is configured.
    pub(crate) fn bearer_token(&self) -> Option<&BackendToken> {
        self.bearer_token.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, prefix: &str, url: &str) -> BackendEntry {
        BackendEntry {
            id: id.to_string(),
            prefix: prefix.to_string(),
            url: url.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn descriptors_are_constructed_only_from_validated_config() {
        let good =
            BackendDescriptor::try_from_entry(&entry("fs", "fs_", "http://127.0.0.1:18701/mcp"))
                .expect("valid entry");
        assert_eq!(good.prefix(), "fs_");
        assert_eq!(good.url(), "http://127.0.0.1:18701/mcp");
        assert!(good.required());
        assert_eq!(good.timeout(), Duration::from_secs(30));

        let bad_url = BackendDescriptor::try_from_entry(&entry(
            "remote",
            "remote_",
            "https://example.invalid/mcp",
        ))
        .expect_err("non-loopback URL must be rejected");
        assert!(bad_url.to_string().contains("loopback"));

        let bad_prefix =
            BackendDescriptor::try_from_entry(&entry("fs", "fs", "http://127.0.0.1:18701/mcp"))
                .expect_err("prefix without separator must be rejected");
        assert!(bad_prefix.to_string().contains("separator"));

        let bad_id =
            BackendDescriptor::try_from_entry(&entry("Fs", "fs_", "http://127.0.0.1:18701/mcp"))
                .expect_err("ambiguous id must be rejected");
        assert!(bad_id.to_string().contains("not normalized"));

        let bad_timeout = BackendDescriptor::try_from_entry(&BackendEntry {
            timeout_seconds: 0,
            ..entry("fs", "fs_", "http://127.0.0.1:18701/mcp")
        })
        .expect_err("zero timeout must be rejected");
        assert!(bad_timeout.to_string().contains("timeout_seconds"));

        let secret = "descriptor-secret-0123456789abcdefghij";
        let with_token = BackendDescriptor::try_from_entry(&BackendEntry {
            bearer_token_env: Some("LOMWAY_BACKEND_FS_TOKEN".to_string()),
            bearer_token: Some(BackendToken::new(secret).expect("token")),
            ..entry("fs", "fs_", "http://127.0.0.1:18701/mcp")
        })
        .expect("resolved credential");
        assert!(with_token.bearer_token().is_some());
        assert!(!format!("{with_token:?}").contains(secret));

        let unresolved = BackendDescriptor::try_from_entry(&BackendEntry {
            bearer_token_env: Some("LOMWAY_BACKEND_FS_TOKEN".to_string()),
            ..entry("fs", "fs_", "http://127.0.0.1:18701/mcp")
        })
        .expect_err("unresolved credential must not build a descriptor");
        assert!(unresolved.to_string().contains("fails closed"));
    }
}
