//! Capability discovery and safe invocation of low-frequency backend tools.
//!
//! Deferred tools are hidden by the outer mcp-proxy capability filter. This
//! module publishes exactly three Lomway-owned meta-tools:
//!
//! * `lomway_search_tools` searches direct tools, deferred tools (from the
//!   cached manifest), and declared internal capabilities. It never starts,
//!   reconnects, or activates a backend.
//! * `lomway_describe_tool` describes one direct, deferred, or internal
//!   capability. Describing a deferred tool (or a `<prefix>*` placeholder)
//!   is an explicit use and may activate that backend.
//! * `lomway_call_tool` invokes deferred tools only.
//!
//! Results are filtered by the caller's PAT/OAuth allowlist, which the
//! northbound authorization layer injects as a hidden argument.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;
use tower::Service;
use tower_mcp::{
    CallToolResult, McpRouter, ToolBuilder, ToolDefinition,
    client::ChannelTransport,
    protocol::{CallToolParams, McpRequest, McpResponse, RequestId},
    proxy::McpProxy,
    router::{Extensions, RouterRequest},
    schemars::JsonSchema,
};

use crate::config::model::{BackendExposure, GatewayConfig};
use crate::gateway::capability::{
    CallerAccess, CapabilityCatalog, IndexEntry, Invocation, Status, ToolSource, build_index,
    invocation_guidance, search,
};
use crate::gateway::manifest::{ManifestStore, manifest_path};
use crate::gateway::reconnect::MonitorHandle;

const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 20;
const META_TOOL_PREFIX: &str = "lomway_";
/// Hidden argument carrying the caller's tool allowlist. The northbound
/// authorization layer overwrites it for restricted callers and strips it
/// for unrestricted ones; it is not part of any published input schema.
pub(super) const CALLER_ALLOWLIST_ARGUMENT: &str = "_lomway_allowed_tools";
/// Meta-tools whose results depend on the caller's allowlist.
pub(super) const ACCESS_SCOPED_META_TOOLS: [&str; 2] =
    ["lomway_search_tools", "lomway_describe_tool"];

#[derive(Debug, Clone)]
struct ConfiguredBackend {
    id: String,
    prefix: String,
    deferred: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DeferredCatalog {
    backends: Arc<Vec<ConfiguredBackend>>,
    routing_guidance: Arc<Option<String>>,
    manifests: ManifestStore,
}

impl DeferredCatalog {
    pub(super) fn from_config(config: &GatewayConfig) -> Self {
        let backends = config
            .backends
            .iter()
            .map(|backend| ConfiguredBackend {
                id: backend.id.clone(),
                prefix: backend.prefix.clone(),
                deferred: backend.exposure == BackendExposure::Deferred,
            })
            .collect();
        Self {
            backends: Arc::new(backends),
            routing_guidance: Arc::new(config.server.instructions.clone()),
            manifests: ManifestStore::in_memory(),
        }
    }

    /// Persist deferred manifests under `state_dir` (per-user state, never
    /// the repository or deployment runtime directory).
    pub(super) fn with_state_dir(mut self, state_dir: Option<PathBuf>, port: u16) -> Self {
        if let Some(state_dir) = state_dir {
            let deferred: Vec<(String, String)> = self
                .deferred_backends()
                .map(|backend| (backend.id.clone(), backend.prefix.clone()))
                .collect();
            self.manifests = ManifestStore::open(manifest_path(&state_dir, port, &deferred));
        }
        self
    }

    fn deferred_backends(&self) -> impl Iterator<Item = &ConfiguredBackend> {
        self.backends.iter().filter(|backend| backend.deferred)
    }

    /// Longest configured prefix wins so nested prefixes stay unambiguous.
    fn backend_for_name(&self, name: &str) -> Option<&ConfiguredBackend> {
        self.backends
            .iter()
            .filter(|backend| name.starts_with(&backend.prefix))
            .max_by_key(|backend| backend.prefix.len())
    }

    fn deferred_backend_for_name(&self, name: &str) -> Option<&ConfiguredBackend> {
        self.backend_for_name(name)
            .filter(|backend| backend.deferred)
    }

    /// `<prefix>*` names an unverified deferred backend as a whole.
    fn placeholder_backend(&self, name: &str) -> Option<&ConfiguredBackend> {
        let prefix = name.strip_suffix('*')?;
        self.deferred_backends()
            .find(|backend| backend.prefix == prefix)
    }

    fn backend_is_configured(&self, id: &str) -> bool {
        id == "internal" || self.backends.iter().any(|backend| backend.id == id)
    }

    fn deferred_ids(&self) -> Vec<String> {
        self.deferred_backends()
            .map(|backend| backend.id.clone())
            .collect()
    }

    fn routing_guidance(&self) -> Option<&str> {
        self.routing_guidance.as_deref()
    }

    fn group_deferred(&self, tools: &[ToolDefinition]) -> BTreeMap<String, Vec<ToolDefinition>> {
        let mut grouped: BTreeMap<String, Vec<ToolDefinition>> = BTreeMap::new();
        for tool in tools {
            if let Some(backend) = self.deferred_backend_for_name(&tool.name) {
                grouped
                    .entry(backend.id.clone())
                    .or_default()
                    .push(tool.clone());
            }
        }
        grouped
    }

    /// Record a fresh observation of deferred backends (startup or after an
    /// explicit activation).
    fn observe_deferred(&self, tools: &[ToolDefinition]) {
        for (backend, definitions) in self.group_deferred(tools) {
            self.manifests.observe(&backend, definitions);
        }
    }

    /// Fill in manifests that were never recorded from the proxy's already
    /// cached catalog. Reading that cache never contacts a backend.
    fn observe_missing(&self, tools: &[ToolDefinition]) {
        for (backend, definitions) in self.group_deferred(tools) {
            if self.manifests.get(&backend).is_none() {
                self.manifests.observe(&backend, definitions);
            }
        }
    }

    /// Searchable sources: live direct tools, cached deferred manifests, and
    /// `unverified` placeholders for deferred backends never observed.
    fn sources(&self, tools: &[ToolDefinition]) -> Vec<ToolSource> {
        let mut sources = Vec::new();
        for tool in tools {
            if tool.name.starts_with(META_TOOL_PREFIX) {
                continue;
            }
            let backend = self.backend_for_name(&tool.name);
            if backend.is_some_and(|backend| backend.deferred) {
                continue;
            }
            sources.push(ToolSource {
                name: tool.name.clone(),
                backend: backend.map_or_else(|| "direct".to_string(), |b| b.id.clone()),
                invocation: Invocation::Direct,
                status: Status::Available,
                manifest_seen_at: None,
                description: tool.description.clone(),
                placeholder_prefix: None,
                via: None,
            });
        }
        for backend in self.deferred_backends() {
            match self.manifests.get(&backend.id) {
                Some(manifest) => {
                    for tool in manifest.tools {
                        sources.push(ToolSource {
                            name: tool.name,
                            backend: backend.id.clone(),
                            invocation: Invocation::Deferred,
                            status: Status::Cached,
                            manifest_seen_at: Some(manifest.seen_at.clone()),
                            description: tool.description,
                            placeholder_prefix: None,
                            via: None,
                        });
                    }
                }
                None => sources.push(ToolSource {
                    name: format!("{}*", backend.prefix),
                    backend: backend.id.clone(),
                    invocation: Invocation::Deferred,
                    status: Status::Unverified,
                    manifest_seen_at: None,
                    description: None,
                    placeholder_prefix: Some(backend.prefix.clone()),
                    via: None,
                }),
            }
        }
        sources
    }

    fn index(&self, tools: &[ToolDefinition]) -> Vec<IndexEntry> {
        build_index(CapabilityCatalog::builtin(), self.sources(tools))
    }
}

/// Read the gateway's own aggregated catalog. The upstream proxy answers
/// this from its per-backend capability cache without contacting backends.
async fn current_tools(proxy: &McpProxy) -> Result<Vec<ToolDefinition>, tower_mcp::Error> {
    let mut proxy = proxy.clone();
    let request = RouterRequest {
        id: RequestId::Number(0),
        inner: McpRequest::ListTools(Default::default()),
        extensions: Extensions::new(),
    };
    let response = proxy
        .call(request)
        .await
        .map_err(|_| tower_mcp::Error::tool("internal tool discovery failed"))?;
    match response.inner {
        Ok(McpResponse::ListTools(result)) => Ok(result.tools),
        Ok(_) => Err(tower_mcp::Error::tool(
            "tool discovery returned an unexpected response type",
        )),
        Err(error) => Err(tower_mcp::Error::tool(error.message)),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchInput {
    /// Natural-language description of the capability or action needed, in
    /// English or Japanese. Upstream OSS names (for example Serena) work.
    query: String,
    /// Optional exact backend id (direct or deferred) or `internal`. Omit
    /// when unsure to search everything.
    backend: Option<String>,
    /// Maximum results, capped at 20.
    limit: Option<usize>,
    #[serde(default, rename = "_lomway_allowed_tools")]
    #[schemars(skip)]
    caller_allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DescribeInput {
    /// Exact name returned by lomway_search_tools: a direct or deferred tool,
    /// an internal capability, or a deferred `<prefix>*` placeholder.
    name: String,
    #[serde(default, rename = "_lomway_allowed_tools")]
    #[schemars(skip)]
    caller_allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CallInput {
    /// Exact fully-qualified deferred tool name returned by search.
    name: String,
    /// Arguments forwarded to the selected backend tool.
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
}

fn not_registered(name: &str) -> tower_mcp::Error {
    // Identical for unknown and unauthorized names so a restricted caller
    // cannot learn whether a hidden tool exists.
    tower_mcp::Error::tool(format!(
        "tool '{name}' is not a currently registered Lomway tool"
    ))
}

fn entry_for<'a>(entries: &'a [IndexEntry], name: &str) -> Option<&'a IndexEntry> {
    entries.iter().find(|entry| entry.source.name == name)
}

fn capability_fields(entry: Option<&IndexEntry>) -> serde_json::Value {
    let Some(entry) = entry else {
        return serde_json::json!({});
    };
    serde_json::json!({
        "provider_oss": entry.metadata.provider_oss,
        "domain": entry.metadata.domain,
        "aliases": entry.metadata.aliases,
        "actions": entry.metadata.actions,
    })
}

fn merge(mut body: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    body
}

async fn describe(
    catalog: &DeferredCatalog,
    proxy: &McpProxy,
    monitor: &MonitorHandle,
    input: DescribeInput,
) -> Result<CallToolResult, tower_mcp::Error> {
    let access = CallerAccess::new(input.caller_allowed_tools);
    let name = input.name;

    if let Some(internal) = CapabilityCatalog::builtin().internal_named(&name) {
        if !access.tool(&internal.via) {
            return Err(not_registered(&name));
        }
        let body = serde_json::json!({
            "name": internal.name,
            "backend": "internal",
            "invocation": Invocation::Internal,
            "status": Status::Declared,
            "via": internal.via,
            "provider_oss": internal.provider_oss,
            "domain": internal.domain,
            "aliases": internal.aliases,
            "description": internal.summary,
            "how_to_invoke": format!(
                "Internal capability without its own MCP tool: reach it through `{}`.",
                internal.via
            ),
        });
        return Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?));
    }

    if let Some(backend) = catalog.placeholder_backend(&name) {
        if !access.any_with_prefix(&backend.prefix) {
            return Err(not_registered(&name));
        }
        monitor
            .activate(&backend.id)
            .await
            .map_err(|e| tower_mcp::Error::tool(e.to_string()))?;
        let tools = current_tools(proxy).await?;
        catalog.observe_deferred(&tools);
        let manifest = catalog.manifests.get(&backend.id);
        let listed: Vec<serde_json::Value> = manifest
            .as_ref()
            .map(|manifest| {
                manifest
                    .tools
                    .iter()
                    .filter(|tool| access.tool(&tool.name))
                    .map(|tool| serde_json::json!({"name": tool.name, "description": tool.description}))
                    .collect()
            })
            .unwrap_or_default();
        let body = serde_json::json!({
            "name": name,
            "backend": backend.id,
            "invocation": Invocation::Deferred,
            "status": if manifest.is_some() { Status::Cached } else { Status::Unverified },
            "manifest_seen_at": manifest.as_ref().map(|manifest| manifest.seen_at.clone()),
            "tools": listed,
            "how_to_invoke": "Describe one listed tool with lomway_describe_tool for its exact schema, then invoke it with lomway_call_tool.",
        });
        return Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?));
    }

    if !access.tool(&name) {
        return Err(not_registered(&name));
    }

    if let Some(backend) = catalog.deferred_backend_for_name(&name) {
        monitor
            .activate(&backend.id)
            .await
            .map_err(|e| tower_mcp::Error::tool(e.to_string()))?;
        let tools = current_tools(proxy).await?;
        catalog.observe_deferred(&tools);
        let Some(tool) = tools.iter().find(|tool| tool.name == name) else {
            return Err(not_registered(&name));
        };
        let entries = catalog.index(&tools);
        let entry = entry_for(&entries, &name);
        let body = serde_json::json!({
            "name": tool.name,
            "backend": backend.id,
            "invocation": Invocation::Deferred,
            "status": Status::Available,
            "manifest_seen_at": catalog.manifests.get(&backend.id).map(|m| m.seen_at),
            "description": tool.description,
            "inputSchema": tool.input_schema,
            "outputSchema": tool.output_schema,
            "annotations": tool.annotations,
            "how_to_invoke": format!("Deferred tool: invoke it with lomway_call_tool using name \"{}\".", tool.name),
        });
        let body = merge(body, capability_fields(entry));
        return Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?));
    }

    // Direct tools are described from the gateway's cached catalog without
    // activating anything.
    let tools = current_tools(proxy).await?;
    let Some(tool) = tools.iter().find(|tool| tool.name == name) else {
        return Err(not_registered(&name));
    };
    let entries = catalog.index(&tools);
    let entry = entry_for(&entries, &name);
    let guidance = entry.map_or_else(
        || {
            format!(
                "Direct tool: it is listed in tools/list, so call `{name}` directly. Do not use lomway_call_tool."
            )
        },
        |entry| invocation_guidance(entry, None),
    );
    let body = serde_json::json!({
        "name": tool.name,
        "backend": entry.map_or_else(|| "lomway".to_string(), |entry| entry.source.backend.clone()),
        "invocation": Invocation::Direct,
        "status": Status::Available,
        "manifest_seen_at": serde_json::Value::Null,
        "description": tool.description,
        "inputSchema": tool.input_schema,
        "outputSchema": tool.output_schema,
        "annotations": tool.annotations,
        "how_to_invoke": guidance,
    });
    let body = merge(body, capability_fields(entry));
    Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?))
}

/// Register the safe discovery backend under the reserved lomway namespace.
/// Upstream proxy/admin tools remain removed.
pub(super) async fn register(
    proxy: &McpProxy,
    catalog: DeferredCatalog,
    monitor: MonitorHandle,
) -> Result<()> {
    // Startup observation: the proxy has just connected every reachable
    // backend, so its cached deferred catalog is a fresh manifest.
    if let Ok(tools) = current_tools(proxy).await {
        catalog.observe_deferred(&tools);
    }

    let search_catalog = catalog.clone();
    let search_proxy = proxy.clone();
    let deferred_ids = catalog.deferred_ids().join(", ");
    let routing_guidance = catalog
        .routing_guidance()
        .map(|guidance| format!(" Deployment routing guidance: {guidance}"))
        .unwrap_or_default();
    let search = ToolBuilder::new("search_tools")
        .description(format!(
            "Use this first whenever you are unsure whether a capability exists, before concluding it is missing or proposing to install anything. Searches every capability this caller may use: direct tools already in tools/list, deferred tools hidden behind Lomway deferred exposure (from the last cached manifest; searching never starts a backend), and internal capabilities reached through another tool. Matches upstream OSS names (for example Serena, Graft, Hindsight), English and Japanese aliases, action names, and descriptions. Each result gives invocation (direct|deferred|internal), action, provider_oss, status, manifest_seen_at, and how_to_invoke: call direct results directly by name and never through lomway_call_tool; for deferred results use lomway_describe_tool for the exact schema, then lomway_call_tool. Omit backend when unsure and search everything. Configured deferred backend ids: {deferred_ids}.{routing_guidance}"
        ))
        .handler(move |input: SearchInput| {
            let catalog = search_catalog.clone();
            let proxy = search_proxy.clone();
            async move {
                if input.query.trim().is_empty() {
                    return Err(tower_mcp::Error::tool("search query must not be empty"));
                }
                if let Some(backend) = input.backend.as_deref()
                    && !catalog.backend_is_configured(backend)
                {
                    return Err(tower_mcp::Error::tool(format!(
                        "backend '{backend}' is not a configured Lomway backend"
                    )));
                }
                // No backend activation here: direct tools come from the
                // proxy's cached catalog and deferred tools from manifests.
                let tools = current_tools(&proxy).await?;
                catalog.observe_missing(&tools);
                let entries = catalog.index(&tools);
                let access = CallerAccess::new(input.caller_allowed_tools);
                let results = search(
                    &entries,
                    &input.query,
                    input.backend.as_deref(),
                    &access,
                    input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
                );
                Ok(CallToolResult::text(serde_json::to_string_pretty(&results)?))
            }
        })
        .build();

    let describe_catalog = catalog.clone();
    let describe_proxy = proxy.clone();
    let describe_monitor = monitor.clone();
    let describe_tool = ToolBuilder::new("describe_tool")
        .description(
            "Use after lomway_search_tools, or whenever arguments are uncertain. Return the exact current MCP description and input/output schema for one Lomway tool (direct or deferred), an internal capability, or a deferred `<prefix>*` placeholder. Direct tools are called directly; only deferred tools use lomway_call_tool.",
        )
        .handler(move |input: DescribeInput| {
            let catalog = describe_catalog.clone();
            let proxy = describe_proxy.clone();
            let monitor = describe_monitor.clone();
            async move { describe(&catalog, &proxy, &monitor, input).await }
        })
        .build();

    let call_catalog = catalog;
    let call_proxy = proxy.clone();
    let call_monitor = monitor;
    let call = ToolBuilder::new("call_tool")
        .description(
            "Invoke one exact currently available deferred Lomway tool using arguments that match the schema returned by lomway_describe_tool. Normally use after lomway_search_tools and lomway_describe_tool. Direct/control-plane tools are rejected.",
        )
        .handler(move |input: CallInput| {
            let catalog = call_catalog.clone();
            let mut proxy = call_proxy.clone();
            let monitor = call_monitor.clone();
            async move {
                let backend_id = catalog.deferred_backend_for_name(&input.name).map(|b| b.id.clone());
                if let Some(backend) = backend_id.as_deref() {
                    monitor.activate(backend).await.map_err(|e| tower_mcp::Error::tool(e.to_string()))?;
                }
                let tools = current_tools(&proxy).await?;
                if backend_id.is_some() {
                    catalog.observe_deferred(&tools);
                }
                if backend_id.is_none() || !tools.iter().any(|tool| tool.name == input.name) {
                    return Err(tower_mcp::Error::tool(format!(
                        "tool '{}' is not a currently registered deferred Lomway tool",
                        input.name
                    )));
                }
                let request = RouterRequest {
                    id: RequestId::Number(0),
                    inner: McpRequest::CallTool(CallToolParams {
                        name: input.name,
                        arguments: input.arguments.unwrap_or_default().into(),
                        input_responses: None,
                        request_state: None,
                        meta: None,
                        task: None,
                    }),
                    extensions: Extensions::new(),
                };
                match proxy.call(request).await {
                    Ok(response) => match response.inner {
                        Ok(McpResponse::CallTool(result)) => Ok(result),
                        Ok(_) => Err(tower_mcp::Error::tool(
                            "deferred tool returned an unexpected response type",
                        )),
                        Err(error) => {
                            if (error.message.to_ascii_lowercase().contains("transport")
                                || error.message.to_ascii_lowercase().contains("session")
                                || error.message.to_ascii_lowercase().contains("connection"))
                                && let Some(backend) = backend_id.as_deref()
                            {
                                monitor.transport_failure(backend);
                            }
                            Err(tower_mcp::Error::tool(error.message))
                        }
                    },
                    Err(_) => {
                        if let Some(backend) = backend_id.as_deref() {
                            monitor.transport_failure(backend);
                        }
                        Err(tower_mcp::Error::tool("internal deferred tool dispatch failed"))
                    },
                }
            }
        })
        .build();

    let router = McpRouter::new()
        .server_info("lomway-deferred-tools", env!("CARGO_PKG_VERSION"))
        .tool(search)
        .tool(describe_tool)
        .tool(call);
    proxy
        .add_backend("lomway", ChannelTransport::new(router))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[derive(Debug, serde::Deserialize, JsonSchema)]
    struct Empty {}

    fn backend_router(name: &'static str, tools: &[(&'static str, &'static str)]) -> McpRouter {
        let mut router = McpRouter::new().server_info(name, "1.0.0");
        for (tool, description) in tools {
            router = router.tool(
                ToolBuilder::new(*tool)
                    .description(*description)
                    .handler(|_: Empty| async move { Ok(CallToolResult::text("ok")) })
                    .build(),
            );
        }
        router
    }

    fn config(with_offline_deferred: bool) -> GatewayConfig {
        let mut text = String::from(
            r#"
schema_version = 1

[[backends]]
id = "workbridge"
prefix = "workbridge_"
url = "http://127.0.0.1:18001/mcp"

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://127.0.0.1:18002/mcp"
exposure = "deferred"
"#,
        );
        if with_offline_deferred {
            text.push_str(
                r#"
[[backends]]
id = "chrome"
prefix = "chrome_"
url = "http://127.0.0.1:18003/mcp"
exposure = "deferred"
"#,
            );
        }
        toml::from_str(&text).expect("test config")
    }

    async fn gateway(with_offline_deferred: bool) -> (McpProxy, MonitorHandle) {
        let proxy = McpProxy::builder("lomway-test", "1.0.0")
            .separator("_")
            .backend(
                "workbridge",
                ChannelTransport::new(backend_router(
                    "workbridge",
                    &[
                        (
                            "run_semantic_action",
                            "Run one read-only Serena semantic query against the exact root of an open workspace.",
                        ),
                        ("read", "Read a file in a workspace."),
                    ],
                )),
            )
            .await
            .backend(
                "browser",
                ChannelTransport::new(backend_router(
                    "browser",
                    &[("navigate", "Navigate a browser page")],
                )),
            )
            .await
            .build_strict()
            .await
            .expect("test proxy");
        let monitor = MonitorHandle::snapshot_for_proxy(proxy.clone());
        let catalog = DeferredCatalog::from_config(&config(with_offline_deferred));
        register(&proxy, catalog, monitor.clone())
            .await
            .expect("register meta-tools");
        (proxy, monitor)
    }

    async fn call(proxy: &McpProxy, tool: &str, arguments: Value) -> CallToolResult {
        let mut proxy = proxy.clone();
        let response = proxy
            .call(RouterRequest {
                id: RequestId::Number(1),
                inner: McpRequest::CallTool(CallToolParams {
                    name: tool.to_string(),
                    arguments,
                    input_responses: None,
                    request_state: None,
                    meta: None,
                    task: None,
                }),
                extensions: Extensions::new(),
            })
            .await
            .expect("infallible");
        match response.inner {
            Ok(McpResponse::CallTool(result)) => result,
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn results(result: &CallToolResult) -> Vec<Value> {
        assert!(!result.is_error, "{}", result.all_text());
        serde_json::from_str::<Value>(&result.all_text())
            .expect("json")
            .as_array()
            .cloned()
            .expect("array")
    }

    #[tokio::test]
    async fn search_covers_direct_tools_without_activating_deferred_backends() {
        let (proxy, monitor) = gateway(true).await;
        let found = results(&call(&proxy, "lomway_search_tools", json!({"query": "Serena"})).await);
        assert_eq!(found[0]["name"], "workbridge_run_semantic_action");
        assert_eq!(found[0]["invocation"], "direct");
        assert_eq!(found[0]["provider_oss"], "Serena");
        assert!(
            found[0]["how_to_invoke"]
                .as_str()
                .unwrap()
                .contains("Do not use lomway_call_tool")
        );
        let refs = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "関数の参照元を探す"}),
            )
            .await,
        );
        assert_eq!(refs[0]["action"], "find_referencing_symbols");

        // Deferred: the connected backend is served from the startup
        // manifest; the never-observed one is an unverified placeholder.
        let browser = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "navigate browser"}),
            )
            .await,
        );
        let navigate = browser
            .iter()
            .find(|result| result["name"] == "browser_navigate")
            .expect("cached deferred tool");
        assert_eq!(navigate["invocation"], "deferred");
        assert_eq!(navigate["status"], "cached");
        assert!(navigate["manifest_seen_at"].as_str().is_some());
        let chrome = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "chrome devtools"}),
            )
            .await,
        );
        assert_eq!(chrome[0]["name"], "chrome_*");
        assert_eq!(chrome[0]["status"], "unverified");
        assert!(chrome[0]["manifest_seen_at"].is_null());

        assert!(
            results(
                &call(
                    &proxy,
                    "lomway_search_tools",
                    json!({"query": "量子暗号鍵配送"})
                )
                .await
            )
            .is_empty()
        );
        assert_eq!(
            monitor.activation_count(),
            0,
            "search must never activate a backend"
        );

        // Describing a deferred tool is an explicit use and does activate.
        let _ = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "browser_navigate"}),
        )
        .await;
        assert_eq!(monitor.activation_count(), 1);
    }

    #[tokio::test]
    async fn restricted_caller_cannot_see_or_describe_unapproved_tools() {
        let (proxy, monitor) = gateway(false).await;
        let restricted = json!([
            "lomway_search_tools",
            "lomway_describe_tool",
            "browser_navigate"
        ]);
        let hidden = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "Serena", "_lomway_allowed_tools": restricted}),
            )
            .await,
        );
        assert!(hidden.is_empty(), "{hidden:?}");
        let unknown = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "workbridge_does_not_exist", "_lomway_allowed_tools": restricted}),
        )
        .await;
        let unauthorized = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "workbridge_run_semantic_action", "_lomway_allowed_tools": restricted}),
        )
        .await;
        assert!(unknown.is_error && unauthorized.is_error);
        assert_eq!(
            unknown
                .all_text()
                .replace("workbridge_does_not_exist", "<name>"),
            unauthorized
                .all_text()
                .replace("workbridge_run_semantic_action", "<name>"),
            "unauthorized and nonexistent tools must be indistinguishable"
        );
        let allowed = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "navigate", "_lomway_allowed_tools": restricted}),
            )
            .await,
        );
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0]["name"], "browser_navigate");
        assert_eq!(monitor.activation_count(), 0);
    }

    #[tokio::test]
    async fn describe_explains_direct_tools_without_activation() {
        let (proxy, monitor) = gateway(false).await;
        let described = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "workbridge_run_semantic_action"}),
        )
        .await;
        assert!(!described.is_error, "{}", described.all_text());
        let body: Value = serde_json::from_str(&described.all_text()).expect("json");
        assert_eq!(body["invocation"], "direct");
        assert_eq!(body["backend"], "workbridge");
        assert_eq!(body["provider_oss"], "Serena");
        assert!(body["inputSchema"].is_object());
        assert!(
            body["how_to_invoke"]
                .as_str()
                .unwrap()
                .contains("Do not use lomway_call_tool")
        );
        let internal = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "browserskill"}),
        )
        .await;
        let internal: Value = serde_json::from_str(&internal.all_text()).expect("json");
        assert_eq!(internal["invocation"], "internal");
        assert_eq!(internal["via"], "workbridge_exec_command");
        assert_eq!(monitor.activation_count(), 0);
    }

    #[tokio::test]
    async fn hidden_allowlist_argument_is_not_published_in_schemas() {
        let (proxy, _) = gateway(false).await;
        let tools = current_tools(&proxy).await.expect("tools");
        for name in ACCESS_SCOPED_META_TOOLS {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("meta tool");
            assert!(
                !tool
                    .input_schema
                    .to_string()
                    .contains(CALLER_ALLOWLIST_ARGUMENT),
                "{name} schema leaks the hidden argument"
            );
        }
    }
}
