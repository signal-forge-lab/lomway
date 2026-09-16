//! The arbitrary backend registry.
//!
//! Holds 0..N validated descriptors in deterministic (configuration) order
//! and knows nothing about any specific deployment, integration, or vendor.

use anyhow::Result;

use crate::backend::descriptor::BackendDescriptor;
use crate::config::model::GatewayConfig;

/// Ordered registry of validated backend descriptors.
#[derive(Debug, Clone, Default)]
pub struct BackendRegistry {
    backends: Vec<BackendDescriptor>,
}

impl BackendRegistry {
    /// Build the registry from a configuration, validating every entry.
    /// Order is exactly the configuration order, so registry iteration is
    /// deterministic.
    pub fn from_config(config: &GatewayConfig) -> Result<Self> {
        crate::config::validate::validate(config)?;
        let backends = config
            .backends
            .iter()
            .map(|entry| {
                BackendDescriptor::try_from_entry_with_policy(
                    entry,
                    config.policy.allow_non_loopback_backends,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { backends })
    }

    /// Construct a registry from already-validated descriptors.
    #[cfg(test)]
    pub fn from_descriptors(backends: Vec<BackendDescriptor>) -> Self {
        Self { backends }
    }

    /// Look up a backend by id.
    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<&BackendDescriptor> {
        self.backends.iter().find(|backend| backend.id() == id)
    }

    /// Deterministic iteration in configuration order.
    pub fn iter(&self) -> impl Iterator<Item = &BackendDescriptor> {
        self.backends.iter()
    }

    /// Number of registered backends.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::BackendEntry;

    fn entry(id: &str, prefix: &str, url: &str) -> BackendEntry {
        BackendEntry {
            id: id.to_string(),
            prefix: prefix.to_string(),
            url: url.to_string(),
            ..Default::default()
        }
    }

    fn config(backends: Vec<BackendEntry>) -> GatewayConfig {
        GatewayConfig {
            schema_version: crate::config::validate::SCHEMA_VERSION,
            server: Default::default(),
            observability: Default::default(),
            policy: Default::default(),
            backends,
        }
    }

    #[test]
    fn supports_zero_one_and_many_backends() {
        assert!(
            BackendRegistry::from_config(&config(vec![]))
                .expect("zero backends are a valid registry")
                .is_empty()
        );

        let one = BackendRegistry::from_config(&config(vec![entry(
            "a",
            "a_",
            "http://127.0.0.1:19001/mcp",
        )]))
        .expect("one backend");
        assert_eq!(one.len(), 1);

        let many = BackendRegistry::from_config(&config(vec![
            entry("a", "a_", "http://127.0.0.1:19001/mcp"),
            entry("b", "b_", "http://127.0.0.1:19002/mcp"),
            entry("c", "c_", "http://127.0.0.1:19003/mcp"),
        ]))
        .expect("many backends");
        let ids: Vec<_> = many.iter().map(|backend| backend.id()).collect();
        assert_eq!(
            ids,
            vec!["a", "b", "c"],
            "order matches configuration order"
        );
    }

    #[test]
    fn lookup_by_id_is_exact() {
        let registry = BackendRegistry::from_config(&config(vec![
            entry("workbridge", "workbridge_", "http://127.0.0.1:19001/mcp"),
            entry("memory", "memory_", "http://127.0.0.1:19002/mcp"),
        ]))
        .expect("registry");
        assert_eq!(registry.get("memory").expect("found").prefix(), "memory_");
        assert!(registry.get("mem").is_none(), "lookup is exact, not fuzzy");
        assert!(registry.get("Memory").is_none(), "ids are never normalized");
    }

    #[test]
    fn explicit_policy_allows_tailnet_https_backend() {
        let mut public = config(vec![entry(
            "workbridge_mac",
            "workbridge_mac_",
            "https://workbridge-mac.example-tailnet.ts.net/mcp",
        )]);
        public.policy.allow_non_loopback_backends = true;
        let registry = BackendRegistry::from_config(&public).expect("tailnet backend registry");
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.get("workbridge_mac").expect("mac backend").url(),
            "https://workbridge-mac.example-tailnet.ts.net/mcp"
        );
    }

    #[test]
    fn contains_no_hard_coded_private_backend_names() {
        // The registry is built purely from configuration; nothing about any
        // specific deployment is baked into the type. This is a compile-time
        // property demonstrated by constructing arbitrary names.
        let registry = BackendRegistry::from_descriptors(vec![
            BackendDescriptor::try_from_entry(&entry(
                "anything",
                "anything_",
                "http://127.0.0.1:19001/mcp",
            ))
            .expect("valid"),
        ]);
        assert_eq!(registry.get("anything").expect("found").id(), "anything");
    }
}
