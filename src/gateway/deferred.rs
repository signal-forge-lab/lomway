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
use crate::gateway::manifest::{ManifestStore, endpoint_fingerprint, manifest_path};
use crate::gateway::reconnect::MonitorHandle;
use crate::gateway::schema_compat::portable_input_schema;
use crate::gateway::workbench::Workbench;

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
    /// Name under which `mcp-proxy` and the reconnect monitor (supervisor)
    /// register this backend: the prefix stem, which differs from `id` for
    /// backends such as `cua-windows` (`cua_windows`) or `jev-ultrafast`
    /// (`jev`). Every monitor call (activation, transport failure, health)
    /// must use this name; `id` stays the public, client-visible identifier
    /// and the manifest-cache key.
    supervisor_name: String,
    deferred: bool,
    /// SHA-256 of this backend's endpoint settings (never the values).
    endpoint_sha256: String,
}

/// The single mapping from a configured backend to its supervisor name.
/// Mirrors `config::migrate::to_legacy`, which names the `mcp-proxy`
/// backend after the validated prefix stem.
fn supervisor_name(id: &str, prefix: &str) -> String {
    crate::namespace::mcp_name_from_prefix(prefix)
        .unwrap_or(id)
        .to_string()
}

#[derive(Debug, Clone, Default)]
pub(super) struct DeferredCatalog {
    backends: Arc<Vec<ConfiguredBackend>>,
    manifests: ManifestStore,
    /// Gateway-owned skills, actions, pipelines, and file transfer.
    workbench: Arc<Workbench>,
}

impl DeferredCatalog {
    pub(super) fn from_config(config: &GatewayConfig) -> Self {
        let backends = config
            .backends
            .iter()
            .map(|backend| ConfiguredBackend {
                id: backend.id.clone(),
                prefix: backend.prefix.clone(),
                supervisor_name: supervisor_name(&backend.id, &backend.prefix),
                deferred: backend.exposure == BackendExposure::Deferred,
                // The public model reaches every backend over Streamable HTTP.
                endpoint_sha256: endpoint_fingerprint("http", &backend.url),
            })
            .collect::<Vec<_>>();
        let mut catalog = Self {
            backends: Arc::new(backends),
            manifests: ManifestStore::default(),
            workbench: Arc::new(Workbench::default()),
        };
        catalog.manifests = ManifestStore::in_memory(catalog.endpoint_fingerprints());
        catalog
    }

    fn endpoint_fingerprints(&self) -> BTreeMap<String, String> {
        self.deferred_backends()
            .map(|backend| (backend.id.clone(), backend.endpoint_sha256.clone()))
            .collect()
    }

    /// Persist deferred manifests under `state_dir` (per-user state, never
    /// the repository or deployment runtime directory).
    pub(super) fn with_state_dir(mut self, state_dir: Option<PathBuf>, port: u16) -> Self {
        if let Some(state_dir) = state_dir {
            let deferred: Vec<(String, String)> = self
                .deferred_backends()
                .map(|backend| (backend.id.clone(), backend.prefix.clone()))
                .collect();
            self.manifests = ManifestStore::open(
                manifest_path(&state_dir, port, &deferred),
                self.endpoint_fingerprints(),
            );
        }
        self
    }

    /// Attach the gateway-owned features (skills, actions, file transfer).
    pub(super) fn with_workbench(mut self, workbench: Arc<Workbench>) -> Self {
        self.workbench = workbench;
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
        id == "internal"
            || (matches!(id, "skills" | "actions" | "pipelines") && !self.workbench.is_empty())
            || self.backends.iter().any(|backend| backend.id == id)
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
                status: Status::Listed,
                manifest_seen_at: None,
                description: tool.description.clone(),
                placeholder_prefix: None,
                via: None,
                public: false,
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
                            public: false,
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
                    public: false,
                }),
            }
        }
        sources
    }

    fn index(&self, tools: &[ToolDefinition]) -> Vec<IndexEntry> {
        let mut entries = build_index(CapabilityCatalog::builtin(), self.sources(tools));
        entries.extend(self.workbench.index_entries());
        entries
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

    if let Some(body) = catalog.workbench.describe(&name, |tool| access.tool(tool)) {
        return Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?));
    }

    if let Some(internal) = CapabilityCatalog::builtin().internal_named(&name) {
        // Same rule as the search index: only a via tool listed directly in
        // this gateway's catalog right now is a route. Unknown, unlisted, and
        // unauthorized all answer identically.
        let tools = current_tools(proxy).await?;
        let listed = |candidate: &str| {
            !candidate.starts_with(META_TOOL_PREFIX)
                && catalog
                    .backend_for_name(candidate)
                    .is_none_or(|backend| !backend.deferred)
                && tools.iter().any(|tool| tool.name == candidate)
        };
        let Some(via) = internal.resolve_via(listed) else {
            return Err(not_registered(&name));
        };
        if !access.tool(via) {
            return Err(not_registered(&name));
        }
        let body = serde_json::json!({
            "name": internal.name,
            "backend": "internal",
            "invocation": Invocation::Internal,
            "status": Status::Declared,
            "via": via,
            "provider_oss": internal.provider_oss,
            "domain": internal.domain,
            "aliases": internal.aliases,
            "description": internal.summary,
            "how_to_invoke": format!(
                "Internal capability without its own MCP tool: reach it through `{via}`."
            ),
        });
        return Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?));
    }

    if let Some(backend) = catalog.placeholder_backend(&name) {
        if !access.any_with_prefix(&backend.prefix) {
            return Err(not_registered(&name));
        }
        monitor
            .activate(&backend.supervisor_name)
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
            .activate(&backend.supervisor_name)
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
            "status": Status::Listed,
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
        "status": Status::Listed,
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

const SEARCH_TOOL_DESCRIPTION: &str = "Use this first whenever you are unsure whether a capability exists, before concluding it is missing or proposing to install anything. Searches across every capability this caller may use: direct tools already in tools/list, deferred tools behind Lomway deferred exposure, and internal capabilities reached through another tool, matching upstream OSS names, English and Japanese aliases, action names, and descriptions. Searching never starts a backend; deferred results come from the last cached manifest. Each result gives invocation (direct|deferred|internal), action, provider_oss, status (listed = in tools/list now, not a health check; cached; unverified; declared), manifest_seen_at, and how_to_invoke. Call direct results directly by name (passing action when one is given), never through lomway_call_tool. Use internal results through the tool named in via. Only deferred results use lomway_describe_tool for the exact schema and then lomway_call_tool. Omit backend unless a previous result gave you its id.";

const DESCRIBE_TOOL_DESCRIPTION: &str = "Use after lomway_search_tools, or whenever arguments are uncertain. Return the exact current MCP description and input/output schema for one name from a search result: a direct tool, a deferred tool, an internal capability, or a deferred `<prefix>*` placeholder. Describing a deferred tool may start its backend. Call direct tools directly by name and internal capabilities through their via tool; only deferred tools go through lomway_call_tool.";

const CALL_TOOL_DESCRIPTION: &str = "Invoke one exact deferred Lomway tool from a lomway_search_tools result, with arguments matching the schema returned by lomway_describe_tool. Deferred tools only: call direct tools directly by name and internal capabilities through their via tool; direct and control-plane tools are rejected here.";

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
    // Tool descriptions are visible to every caller through tools/list,
    // including restricted PATs, so they stay generic: no configured backend
    // ids and no deployment instructions (those reach clients through the
    // MCP `instructions` field instead). Input schemas are published in the
    // portable subset (no type arrays, `uint` formats or `$schema`) so that
    // strict function-calling clients such as Gemini accept them; handlers
    // still accept `null` for optional fields.
    let search = ToolBuilder::new("search_tools")
        .description(SEARCH_TOOL_DESCRIPTION)
        .input_schema(portable_input_schema::<SearchInput>())
        .handler(move |input: SearchInput| {
            let catalog = search_catalog.clone();
            let proxy = search_proxy.clone();
            async move {
                if input.query.trim().is_empty() {
                    return Err(tower_mcp::Error::tool("search query must not be empty"));
                }
                let restricted = input.caller_allowed_tools.is_some();
                if let Some(backend) = input.backend.as_deref()
                    && !catalog.backend_is_configured(backend)
                {
                    // A restricted caller gets the same empty result for an
                    // unknown backend id as for a configured backend it may
                    // not see, so backend registration is not observable.
                    if restricted {
                        return Ok(CallToolResult::text(serde_json::to_string_pretty(&Vec::<
                            serde_json::Value,
                        >::new(
                        ))?));
                    }
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
                Ok(CallToolResult::text(serde_json::to_string_pretty(
                    &results,
                )?))
            }
        })
        .build();

    let describe_catalog = catalog.clone();
    let describe_proxy = proxy.clone();
    let describe_monitor = monitor.clone();
    let describe_tool = ToolBuilder::new("describe_tool")
        .description(DESCRIBE_TOOL_DESCRIPTION)
        .input_schema(portable_input_schema::<DescribeInput>())
        .handler(move |input: DescribeInput| {
            let catalog = describe_catalog.clone();
            let proxy = describe_proxy.clone();
            let monitor = describe_monitor.clone();
            async move { describe(&catalog, &proxy, &monitor, input).await }
        })
        .build();

    let workbench = catalog.workbench.clone();
    let call_catalog = catalog;
    let call_proxy = proxy.clone();
    let call_monitor = monitor;
    let call = ToolBuilder::new("call_tool")
        .description(CALL_TOOL_DESCRIPTION)
        .input_schema(portable_input_schema::<CallInput>())
        .handler(move |input: CallInput| {
            let catalog = call_catalog.clone();
            let mut proxy = call_proxy.clone();
            let monitor = call_monitor.clone();
            async move {
                // Supervisor name, not the public id: the monitor tracks
                // backends by their mcp-proxy namespace.
                let supervisor = catalog
                    .deferred_backend_for_name(&input.name)
                    .map(|b| b.supervisor_name.clone());
                if let Some(backend) = supervisor.as_deref() {
                    monitor
                        .activate(backend)
                        .await
                        .map_err(|e| tower_mcp::Error::tool(e.to_string()))?;
                }
                let tools = current_tools(&proxy).await?;
                if supervisor.is_some() {
                    catalog.observe_deferred(&tools);
                }
                if supervisor.is_none() || !tools.iter().any(|tool| tool.name == input.name) {
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
                            // Only a transport failure, never the backend's own
                            // error reply (invalid params, tool error).
                            if mcp_proxy::is_backend_transport_failure(&error.message)
                                && let Some(backend) = supervisor.as_deref()
                            {
                                monitor.transport_failure(backend);
                            }
                            Err(tower_mcp::Error::tool(error.message))
                        }
                    },
                    Err(_) => {
                        if let Some(backend) = supervisor.as_deref() {
                            monitor.transport_failure(backend);
                        }
                        Err(tower_mcp::Error::tool(
                            "internal deferred tool dispatch failed",
                        ))
                    }
                }
            }
        })
        .build();

    let mut router = McpRouter::new()
        .server_info("lomway-deferred-tools", env!("CARGO_PKG_VERSION"))
        .tool(search)
        .tool(describe_tool)
        .tool(call);
    for tool in workbench.tools() {
        router = router.tool(tool);
    }
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
        gateway_with_config(config(with_offline_deferred)).await
    }

    async fn gateway_with_config(config: GatewayConfig) -> (McpProxy, MonitorHandle) {
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
                        ("exec_command", "Run a command in a workspace."),
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
        let catalog = DeferredCatalog::from_config(&config);
        register(&proxy, catalog, monitor.clone())
            .await
            .expect("register meta-tools");
        (proxy, monitor)
    }

    struct WorkbenchFixture {
        _dir: tempfile::TempDir,
        proxy: McpProxy,
    }

    async fn workbench_gateway() -> WorkbenchFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical");
        let slash = |path: &std::path::Path| path.to_string_lossy().replace('\\', "/");
        let skills = root.join("skills");
        std::fs::create_dir_all(skills.join("deploy-helper")).unwrap();
        std::fs::write(
            skills.join("deploy-helper/SKILL.md"),
            "---\nname: deploy-helper\ndescription: Use when releasing the gateway to production.\n---\nSteps",
        )
        .unwrap();
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let definitions = root.join("actions.toml");
        std::fs::write(
            &definitions,
            format!(
                r#"schema_version = 1
[[actions]]
name = "git_version"
description = "Print the git version (read only)."
policy = ["read_only"]
effects = ["process.execute"]
program = "git"
args = ["--version"]
cwd = "{work}"
[[actions]]
name = "git_status"
description = "Show git status for a repository directory."
effects = ["process.execute", "git.read"]
program = "git"
args = ["-C", "{{repo}}", "status", "--short"]
cwd = "{work}"
[actions.parameters]
required = ["repo"]
[actions.parameters.properties.repo]
type = "string"
format = "relative-path"
[[pipelines]]
name = "versions"
description = "Run the version action twice."
effects = {{ allow = ["process.execute"], deny = ["git.push"] }}
[[pipelines.steps]]
action = "git_version"
[[pipelines.steps]]
action = "git_version"
"#,
                work = slash(&work)
            ),
        )
        .unwrap();
        let mut config = config(false);
        config.skills.enabled = true;
        config.skills.default_paths = false;
        config.skills.paths = vec![slash(&skills)];
        config.actions.enabled = true;
        config.actions.definitions = Some(slash(&definitions));
        config.actions.allowed_roots = vec![slash(&root)];
        config.actions.audit_log = Some(slash(&root.join("audit.jsonl")));
        config.file_transfer.enabled = true;
        config.file_transfer.inbox_dir = Some(slash(&root.join("inbox")));
        let workbench = std::sync::Arc::new(Workbench::from_config(&config));
        assert!(
            workbench
                .instructions_suffix()
                .unwrap()
                .contains("deploy-helper")
        );
        let proxy = McpProxy::builder("lomway-test", "1.0.0")
            .separator("_")
            .backend(
                "workbridge",
                ChannelTransport::new(backend_router("workbridge", &[("read", "Read a file.")])),
            )
            .await
            .build_strict()
            .await
            .expect("test proxy");
        let monitor = MonitorHandle::snapshot_for_proxy(proxy.clone());
        let catalog = DeferredCatalog::from_config(&config).with_workbench(workbench);
        register(&proxy, catalog, monitor).await.expect("register");
        WorkbenchFixture { _dir: dir, proxy }
    }

    #[tokio::test]
    async fn workbench_tools_are_listed_with_file_param_metadata() {
        let fixture = workbench_gateway().await;
        let tools = current_tools(&fixture.proxy).await.unwrap();
        for name in [
            "lomway_list_actions",
            "lomway_run_action",
            "lomway_action_status",
            "lomway_run_pipeline",
            "lomway_receive_file",
            "lomway_send_file",
        ] {
            assert!(tools.iter().any(|tool| tool.name == name), "{name} missing");
        }
        let receive = tools
            .iter()
            .find(|tool| tool.name == "lomway_receive_file")
            .unwrap();
        assert_eq!(
            receive.meta.as_ref().unwrap()["openai/fileParams"],
            json!(["file"])
        );
        // Default gateways publish none of them.
        let (plain, _) = gateway(false).await;
        let plain_tools = current_tools(&plain).await.unwrap();
        assert!(
            !plain_tools
                .iter()
                .any(|tool| tool.name == "lomway_run_action")
        );
    }

    #[tokio::test]
    async fn workbench_tool_schemas_are_portable_for_strict_clients() {
        let fixture = workbench_gateway().await;
        let tools = current_tools(&fixture.proxy).await.unwrap();
        let find = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("{name} missing"))
                .input_schema
                .clone()
        };
        for name in [
            "lomway_list_actions",
            "lomway_run_action",
            "lomway_action_status",
            "lomway_run_pipeline",
            "lomway_receive_file",
            "lomway_send_file",
        ] {
            let schema = find(name);
            assert_eq!(
                crate::gateway::schema_compat::non_portable_findings(&schema),
                Vec::<String>::new(),
                "{name} schema is not portable: {schema}"
            );
            let text = schema.to_string();
            for needle in ["$schema", "$ref", "$defs", "\"uint", "\"null\""] {
                assert!(!text.contains(needle), "{name} has {needle}: {text}");
            }
            assert_eq!(schema["type"], "object", "{name}");
        }
        assert_eq!(find("lomway_run_action")["required"], json!(["action"]));
        let receive = find("lomway_receive_file");
        assert_eq!(receive["required"], json!(["file"]));
        assert_eq!(receive["properties"]["file"]["type"], "object");
        assert_eq!(
            receive["properties"]["file"]["properties"]["download_url"]["type"],
            "string"
        );
        let send = find("lomway_send_file");
        assert_eq!(send["properties"]["outbox"]["type"], "integer");
        assert_eq!(send["properties"]["outbox"]["minimum"], 0);
        // Optional fields still accept null after the schema rewrite.
        let result = call(
            &fixture.proxy,
            "lomway_run_action",
            json!({
                "action": "git_version",
                "dry_run": true,
                "parameters": null,
                "working_directory": null,
                "wait_ms": null
            }),
        )
        .await;
        assert!(!result.is_error, "{result:?}");
    }

    #[tokio::test]
    async fn skills_and_actions_are_searchable_and_describable() {
        let fixture = workbench_gateway().await;
        let found = results(
            &call(
                &fixture.proxy,
                "lomway_search_tools",
                json!({"query": "deploy-helper skill"}),
            )
            .await,
        );
        assert_eq!(found[0]["name"], "skill:deploy-helper", "{found:?}");
        assert_eq!(found[0]["backend"], "skills");
        let action = results(
            &call(
                &fixture.proxy,
                "lomway_search_tools",
                json!({"query": "git_status"}),
            )
            .await,
        );
        assert_eq!(action[0]["name"], "action:git_status", "{action:?}");
        assert!(
            action[0]["how_to_invoke"]
                .as_str()
                .unwrap()
                .contains("lomway_run_action")
        );
        // Restricted caller without run_action: skills stay visible (no
        // allowlist filtering for instructions), actions do not.
        let restricted = results(
            &call(
                &fixture.proxy,
                "lomway_search_tools",
                json!({"query": "git_status deploy-helper", "_lomway_allowed_tools": ["workbridge_read"]}),
            )
            .await,
        );
        assert!(
            restricted
                .iter()
                .any(|r| r["name"] == "skill:deploy-helper")
        );
        assert!(!restricted.iter().any(|r| r["name"] == "action:git_status"));
        let described = call(
            &fixture.proxy,
            "lomway_describe_tool",
            json!({"name": "skill:deploy-helper"}),
        )
        .await;
        let body: Value = serde_json::from_str(&described.all_text()).unwrap();
        assert!(body["path"].as_str().unwrap().ends_with("SKILL.md"));
        assert!(
            body["how_to_invoke"]
                .as_str()
                .unwrap()
                .contains("coding_tools_exec_command")
        );
        let hidden = call(
            &fixture.proxy,
            "lomway_describe_tool",
            json!({"name": "action:git_status", "_lomway_allowed_tools": ["workbridge_read"]}),
        )
        .await;
        assert!(hidden.is_error);
    }

    #[tokio::test]
    async fn actions_run_validate_and_pipelines_freeze_their_plan() {
        let fixture = workbench_gateway().await;
        let listed = call(&fixture.proxy, "lomway_list_actions", json!({})).await;
        assert!(
            listed.all_text().contains("git_status"),
            "{}",
            listed.all_text()
        );
        let dry = call(
            &fixture.proxy,
            "lomway_run_action",
            json!({"action": "git_status", "parameters": {"repo": "sub dir"}, "dry_run": true}),
        )
        .await;
        let body: Value = serde_json::from_str(&dry.all_text()).unwrap();
        assert_eq!(
            body["resolved"]["args"],
            json!(["-C", "sub dir", "status", "--short"])
        );
        let bad = call(
            &fixture.proxy,
            "lomway_run_action",
            json!({"action": "git_status", "parameters": {"repo": "../.."}}),
        )
        .await;
        assert!(bad.is_error);
        let unknown = call(
            &fixture.proxy,
            "lomway_run_action",
            json!({"action": "nope"}),
        )
        .await;
        assert!(unknown.is_error && unknown.all_text().contains("git_version"));
        let ran = call(
            &fixture.proxy,
            "lomway_run_action",
            json!({"action": "git_version", "wait_ms": 20000}),
        )
        .await;
        let body: Value = serde_json::from_str(&ran.all_text()).unwrap();
        assert_eq!(body["state"], "succeeded", "{body}");
        assert!(
            body["steps"][0]["outcome"]["stdout"]
                .as_str()
                .unwrap()
                .contains("git version")
        );
        let plan = call(
            &fixture.proxy,
            "lomway_run_pipeline",
            json!({"pipeline": "versions", "dry_run": true}),
        )
        .await;
        let plan: Value = serde_json::from_str(&plan.all_text()).unwrap();
        let digest = plan["plan"]["digest"].as_str().unwrap().to_string();
        let stale = call(
            &fixture.proxy,
            "lomway_run_pipeline",
            json!({"pipeline": "versions", "expected_digest": "sha256:0"}),
        )
        .await;
        assert!(stale.is_error && stale.all_text().contains("PLAN_INVALIDATED"));
        let run = call(
            &fixture.proxy,
            "lomway_run_pipeline",
            json!({"pipeline": "versions", "expected_digest": digest, "wait_ms": 20000}),
        )
        .await;
        let run: Value = serde_json::from_str(&run.all_text()).unwrap();
        assert_eq!(run["state"], "succeeded", "{run}");
        assert_eq!(run["steps"].as_array().unwrap().len(), 2);
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
        assert_eq!(found[0]["status"], "listed");
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
    async fn restricted_caller_cannot_tell_unknown_backends_from_hidden_ones() {
        let (proxy, monitor) = gateway(true).await;
        let restricted = json!(["lomway_search_tools", "browser_navigate"]);
        let search_backend = |backend: &'static str| {
            let proxy = proxy.clone();
            let restricted = restricted.clone();
            async move {
                call(
                    &proxy,
                    "lomway_search_tools",
                    json!({"query": "Serena", "backend": backend, "_lomway_allowed_tools": restricted}),
                )
                .await
            }
        };
        let unknown = search_backend("no-such-backend").await;
        let hidden_direct = search_backend("workbridge").await;
        let hidden_deferred = search_backend("chrome").await;
        for result in [&unknown, &hidden_direct, &hidden_deferred] {
            assert!(!result.is_error, "{}", result.all_text());
        }
        assert_eq!(unknown.all_text(), hidden_direct.all_text());
        assert_eq!(unknown.all_text(), hidden_deferred.all_text());
        assert!(results(&unknown).is_empty());

        // Unrestricted callers keep the explicit configuration error.
        let owner = call(
            &proxy,
            "lomway_search_tools",
            json!({"query": "Serena", "backend": "no-such-backend"}),
        )
        .await;
        assert!(owner.is_error);
        assert!(owner.all_text().contains("not a configured Lomway backend"));
        let owner_known = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "Serena", "backend": "workbridge"}),
            )
            .await,
        );
        assert_eq!(owner_known[0]["name"], "workbridge_run_semantic_action");
        assert_eq!(owner_known[0]["status"], "listed");
        assert_eq!(monitor.activation_count(), 0);
    }

    fn definition(name: &str) -> ToolDefinition {
        serde_json::from_value(json!({
            "name": name,
            "description": "Navigate a browser page",
            "inputSchema": {"type": "object"}
        }))
        .expect("tool definition")
    }

    fn deferred_status(catalog: &DeferredCatalog) -> (String, String) {
        let source = catalog
            .sources(&[])
            .into_iter()
            .find(|source| source.backend == "browser")
            .expect("browser source");
        let status = serde_json::to_value(source.status).expect("status");
        (source.name, status.as_str().unwrap_or_default().to_string())
    }

    #[test]
    fn manifest_cache_is_bound_to_the_backend_endpoint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = config(false);
        let catalog = DeferredCatalog::from_config(&original)
            .with_state_dir(Some(dir.path().to_path_buf()), 17777);
        catalog.observe_deferred(&[definition("browser_navigate")]);

        // Same settings after a restart: the cache is reused.
        let same = DeferredCatalog::from_config(&original)
            .with_state_dir(Some(dir.path().to_path_buf()), 17777);
        assert_eq!(
            deferred_status(&same),
            ("browser_navigate".to_string(), "cached".to_string())
        );

        // Same id, prefix and port but a different endpoint: unverified.
        let mut moved = original.clone();
        moved
            .backends
            .iter_mut()
            .find(|backend| backend.id == "browser")
            .expect("browser backend")
            .url = "http://127.0.0.1:18099/mcp".to_string();
        let changed = DeferredCatalog::from_config(&moved)
            .with_state_dir(Some(dir.path().to_path_buf()), 17777);
        assert_eq!(
            deferred_status(&changed),
            ("browser_*".to_string(), "unverified".to_string())
        );

        // Neither the cache file names nor their contents carry endpoints.
        for entry in std::fs::read_dir(dir.path()).expect("state dir") {
            let entry = entry.expect("entry");
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(name.starts_with("deferred-manifest-") && !name.contains("127.0.0.1"));
            let body = std::fs::read_to_string(entry.path()).expect("cache body");
            assert!(!body.contains("127.0.0.1"), "{body}");
            assert!(!body.contains("/mcp"), "{body}");
        }
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
        assert_eq!(body["status"], "listed");
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

    /// The deployed profile shape: Coding Tools and Hindsight as direct
    /// backends, browser deferred, and no Workbridge at all.
    async fn coding_tools_only_gateway() -> (McpProxy, MonitorHandle) {
        let config: GatewayConfig = toml::from_str(
            r#"
schema_version = 1

[[backends]]
id = "coding-tools"
prefix = "coding_tools_"
url = "http://127.0.0.1:18011/mcp"

[[backends]]
id = "hindsight"
prefix = "hindsight_"
url = "http://127.0.0.1:18012/mcp"

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://127.0.0.1:18013/mcp"
exposure = "deferred"
"#,
        )
        .expect("coding-tools config");
        let proxy = McpProxy::builder("lomway-test", "1.0.0")
            .separator("_")
            .backend(
                "coding_tools",
                ChannelTransport::new(backend_router(
                    "coding_tools",
                    &[
                        (
                            "exec_command",
                            "Run a bounded command under runtime policy.",
                        ),
                        (
                            "read_file",
                            "Read a UTF-8 text file slice inside the configured workspace.",
                        ),
                        (
                            "search_text",
                            "Search UTF-8 workspace files for text or regex matches.",
                        ),
                    ],
                )),
            )
            .await
            .backend(
                "hindsight",
                ChannelTransport::new(backend_router(
                    "hindsight",
                    &[("recall", "Search memories.")],
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
        register(
            &proxy,
            DeferredCatalog::from_config(&config),
            monitor.clone(),
        )
        .await
        .expect("register meta-tools");
        (proxy, monitor)
    }

    #[tokio::test]
    async fn coding_tools_only_profile_routes_internal_capabilities_through_listed_tools() {
        let (proxy, monitor) = coding_tools_only_gateway().await;

        let browserskill = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "BrowserSkill"}),
            )
            .await,
        );
        assert_eq!(browserskill[0]["name"], "browserskill");
        assert_eq!(browserskill[0]["invocation"], "internal");
        assert_eq!(browserskill[0]["via"], "coding_tools_exec_command");
        let guidance = browserskill[0]["how_to_invoke"].as_str().unwrap();
        assert!(guidance.contains("coding_tools_exec_command"), "{guidance}");
        assert!(!guidance.contains("workbridge"), "{guidance}");

        let described = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "browserskill"}),
        )
        .await;
        assert!(!described.is_error, "{}", described.all_text());
        let described: Value = serde_json::from_str(&described.all_text()).expect("json");
        assert_eq!(described["via"], "coding_tools_exec_command");
        assert!(!described.to_string().contains("workbridge_exec_command"));

        let coding = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "Coding Tools"}),
            )
            .await,
        );
        assert!(!coding.is_empty());
        for result in &coding {
            assert_eq!(result["backend"], "coding-tools", "{result}");
            assert_eq!(result["invocation"], "direct");
            assert_eq!(result["provider_oss"], "Coding Tools MCP");
        }
        let shell = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "コマンド実行"}),
            )
            .await,
        );
        assert_eq!(shell[0]["name"], "coding_tools_exec_command");

        // Nothing in this profile reaches Serena, Graft, or Codebase Memory,
        // so none of them may be offered, directly or via another tool.
        for query in ["Serena", "Graft", "Codebase Memory", "コード解析"] {
            let found =
                results(&call(&proxy, "lomway_search_tools", json!({"query": query})).await);
            assert!(
                found.iter().all(|result| {
                    !result.to_string().contains("workbridge")
                        && result["provider_oss"] != "Serena"
                        && result["provider_oss"] != "Graft"
                        && result["provider_oss"] != "Codebase Memory"
                }),
                "{query}: {found:?}"
            );
        }
        for name in ["workbridge_run_semantic_action", "workbridge_exec_command"] {
            let described = call(&proxy, "lomway_describe_tool", json!({"name": name})).await;
            assert!(described.is_error, "{name}: {}", described.all_text());
        }

        // A restricted caller without the shell tool does not see the route.
        let limited = json!([
            "lomway_search_tools",
            "lomway_describe_tool",
            "hindsight_recall"
        ]);
        let hidden = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "BrowserSkill", "_lomway_allowed_tools": limited}),
            )
            .await,
        );
        assert!(hidden.iter().all(|result| result["name"] != "browserskill"));
        let hidden = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "browserskill", "_lomway_allowed_tools": limited}),
        )
        .await;
        assert!(hidden.is_error);
        assert_eq!(monitor.activation_count(), 0);
    }

    #[tokio::test]
    async fn internal_capability_without_a_listed_via_tool_is_not_offered() {
        // Workbridge without exec_command and no Coding Tools: no route.
        let config = config(false);
        let proxy = McpProxy::builder("lomway-test", "1.0.0")
            .separator("_")
            .backend(
                "workbridge",
                ChannelTransport::new(backend_router(
                    "workbridge",
                    &[("read", "Read a file in a workspace.")],
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
        register(&proxy, DeferredCatalog::from_config(&config), monitor)
            .await
            .expect("register meta-tools");
        let found = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "BrowserSkill"}),
            )
            .await,
        );
        assert!(
            found.iter().all(|result| result["name"] != "browserskill"),
            "{found:?}"
        );
        let unknown = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "no_such_capability"}),
        )
        .await;
        let unrouted = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "browserskill"}),
        )
        .await;
        assert!(unknown.is_error && unrouted.is_error);
        assert_eq!(
            unknown.all_text().replace("no_such_capability", "<name>"),
            unrouted.all_text().replace("browserskill", "<name>"),
        );
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

    #[tokio::test]
    async fn meta_tool_schemas_are_portable_for_strict_clients() {
        let (proxy, _) = gateway(false).await;
        let tools = current_tools(&proxy).await.expect("tools");
        for name in [
            "lomway_search_tools",
            "lomway_describe_tool",
            "lomway_call_tool",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("meta tool");
            let schema = &tool.input_schema;
            assert_eq!(
                crate::gateway::schema_compat::non_portable_findings(schema),
                Vec::<String>::new(),
                "{name} schema is not portable: {schema}"
            );
            let text = schema.to_string();
            assert!(!text.contains("$schema"), "{name}: {text}");
            assert!(!text.contains("\"uint"), "{name}: {text}");
            assert!(!text.contains("\"null\""), "{name}: {text}");
            assert_eq!(schema["type"], "object", "{name}");
        }
        let search = tools
            .iter()
            .find(|tool| tool.name == "lomway_search_tools")
            .unwrap();
        assert_eq!(search.input_schema["required"], json!(["query"]));
        assert_eq!(
            search.input_schema["properties"]["backend"]["type"],
            "string"
        );
        assert_eq!(
            search.input_schema["properties"]["limit"]["type"],
            "integer"
        );
        assert_eq!(search.input_schema["properties"]["limit"]["minimum"], 0);
        let call_tool = tools
            .iter()
            .find(|tool| tool.name == "lomway_call_tool")
            .unwrap();
        assert_eq!(call_tool.input_schema["required"], json!(["name"]));
        assert_eq!(
            call_tool.input_schema["properties"]["arguments"]["type"],
            "object"
        );
    }

    #[tokio::test]
    async fn optional_meta_tool_fields_still_accept_null() {
        let (proxy, _) = gateway(false).await;
        let with_null = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "Serena", "backend": null, "limit": null}),
            )
            .await,
        );
        let omitted =
            results(&call(&proxy, "lomway_search_tools", json!({"query": "Serena"})).await);
        assert_eq!(with_null, omitted);
        assert_eq!(with_null[0]["name"], "workbridge_run_semantic_action");
        let limited = results(
            &call(
                &proxy,
                "lomway_search_tools",
                json!({"query": "Serena", "limit": 1}),
            )
            .await,
        );
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0], with_null[0]);
        // `arguments: null` reaches the same dispatch path as an omitted
        // `arguments` (both fail the same way for a non-deferred name).
        let null_args = call(
            &proxy,
            "lomway_call_tool",
            json!({"name": "workbridge_read", "arguments": null}),
        )
        .await;
        let no_args = call(
            &proxy,
            "lomway_call_tool",
            json!({"name": "workbridge_read"}),
        )
        .await;
        assert!(null_args.is_error && no_args.is_error);
        assert_eq!(null_args.all_text(), no_args.all_text());
    }

    #[tokio::test]
    async fn meta_tool_descriptions_do_not_reveal_backend_ids() {
        let mut config = config(true);
        let mut extra: GatewayConfig = toml::from_str(
            r#"
schema_version = 1

[[backends]]
id = "zq-private-ops"
prefix = "zqops_"
url = "http://127.0.0.1:18004/mcp"
exposure = "deferred"
"#,
        )
        .expect("extra config");
        config.backends.append(&mut extra.backends);
        config.server.instructions =
            Some("Deferred backends in this profile: browser, chrome, zq-private-ops.".to_string());
        let backend_ids: Vec<String> = config
            .backends
            .iter()
            .flat_map(|backend| [backend.id.to_lowercase(), backend.prefix.to_lowercase()])
            .collect();
        let (proxy, _) = gateway_with_config(config).await;
        let tools = current_tools(&proxy).await.expect("tools");
        for name in [
            "lomway_search_tools",
            "lomway_describe_tool",
            "lomway_call_tool",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("meta tool");
            let text = format!(
                "{} {}",
                tool.description.clone().unwrap_or_default(),
                tool.input_schema
            )
            .to_lowercase();
            for id in &backend_ids {
                assert!(
                    !text.contains(id.as_str()),
                    "{name} reveals backend id or prefix '{id}'"
                );
            }
            assert!(
                !text.contains("deferred backends in this profile"),
                "{name} echoes deployment instructions"
            );
        }
    }

    #[test]
    fn supervisor_name_follows_the_prefix_stem_not_the_id() {
        let config: GatewayConfig = toml::from_str(
            r#"
schema_version = 1

[[backends]]
id = "cua-windows"
prefix = "cua_windows_"
url = "http://127.0.0.1:18005/mcp"
exposure = "deferred"

[[backends]]
id = "jev-ultrafast"
prefix = "jev_"
url = "http://127.0.0.1:18006/mcp"
exposure = "deferred"

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://127.0.0.1:18002/mcp"
exposure = "deferred"
"#,
        )
        .expect("config");
        let catalog = DeferredCatalog::from_config(&config);
        let names: Vec<(&str, &str)> = catalog
            .backends
            .iter()
            .map(|backend| (backend.id.as_str(), backend.supervisor_name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("cua-windows", "cua_windows"),
                ("jev-ultrafast", "jev"),
                ("browser", "browser"),
            ]
        );
        // Same rule as the mcp-proxy configuration the supervisor is built from.
        let legacy = crate::config::migrate::to_legacy(&config).expect("legacy config");
        let legacy_names: Vec<&str> = legacy.backends.iter().map(|b| b.name.as_str()).collect();
        for backend in catalog.backends.iter() {
            assert!(
                legacy_names.contains(&backend.supervisor_name.as_str()),
                "{} is not registered under {}",
                backend.id,
                backend.supervisor_name
            );
        }
    }

    /// Hyphenated id whose prefix stem differs (`cua-windows` /
    /// `cua_windows_`): describe and call must activate the supervisor name.
    #[tokio::test]
    async fn deferred_backend_with_hyphenated_id_can_be_described_and_called() {
        let config: GatewayConfig = toml::from_str(
            r#"
schema_version = 1

[[backends]]
id = "workbridge"
prefix = "workbridge_"
url = "http://127.0.0.1:18001/mcp"

[[backends]]
id = "cua-windows"
prefix = "cua_windows_"
url = "http://127.0.0.1:18005/mcp"
exposure = "deferred"
"#,
        )
        .expect("config");
        let proxy = McpProxy::builder("lomway-test", "1.0.0")
            .separator("_")
            .backend(
                "workbridge",
                ChannelTransport::new(backend_router(
                    "workbridge",
                    &[("read", "Read a file in a workspace.")],
                )),
            )
            .await
            .backend(
                "cua_windows",
                ChannelTransport::new(backend_router(
                    "cua_windows",
                    &[("get_screen_size", "Return the screen size")],
                )),
            )
            .await
            .build_strict()
            .await
            .expect("test proxy");
        // The supervisor knows the backend only by its mcp-proxy namespace.
        let monitor = MonitorHandle::connected_for_test(proxy.clone(), &["cua_windows"]);
        register(
            &proxy,
            DeferredCatalog::from_config(&config),
            monitor.clone(),
        )
        .await
        .expect("register meta-tools");

        let described = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "cua_windows_get_screen_size"}),
        )
        .await;
        assert!(!described.is_error, "{}", described.all_text());
        let body: Value = serde_json::from_str(&described.all_text()).expect("json");
        assert_eq!(body["backend"], "cua-windows", "public id stays the id");
        assert_eq!(body["invocation"], "deferred");
        assert!(body["inputSchema"].is_object());

        let placeholder = call(
            &proxy,
            "lomway_describe_tool",
            json!({"name": "cua_windows_*"}),
        )
        .await;
        assert!(!placeholder.is_error, "{}", placeholder.all_text());

        let called = call(
            &proxy,
            "lomway_call_tool",
            json!({"name": "cua_windows_get_screen_size", "arguments": {}}),
        )
        .await;
        assert!(!called.is_error, "{}", called.all_text());
        assert_eq!(called.all_text(), "ok");
        assert_eq!(monitor.activation_count(), 3);
    }
}
