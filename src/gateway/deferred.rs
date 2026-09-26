//! Safe discovery and invocation of low-frequency backend tools.
//!
//! Deferred tools are hidden by the outer mcp-proxy capability filter. This
//! module publishes exactly three Lomway-owned meta-tools and permits calls
//! only to tools currently listed by configured deferred backends.

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
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

const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 20;

#[derive(Debug, Clone)]
struct DeferredBackend {
    id: String,
    prefix: String,
}

#[derive(Debug, Clone)]
struct DeferredTool {
    backend: String,
    public_name: String,
    definition: ToolDefinition,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DeferredCatalog {
    backends: Arc<Vec<DeferredBackend>>,
}

impl DeferredCatalog {
    pub(super) fn from_config(config: &GatewayConfig) -> Self {
        let backends = config
            .backends
            .iter()
            .filter(|backend| backend.exposure == BackendExposure::Deferred)
            .map(|backend| DeferredBackend {
                id: backend.id.clone(),
                prefix: backend.prefix.clone(),
            })
            .collect();
        Self {
            backends: Arc::new(backends),
        }
    }

    fn backend_for_name(&self, name: &str) -> Option<&DeferredBackend> {
        self.backends
            .iter()
            .find(|backend| name.starts_with(&backend.prefix))
    }

    fn backend_is_deferred(&self, id: &str) -> bool {
        self.backends.iter().any(|backend| backend.id == id)
    }

    fn filter_tools(&self, definitions: Vec<ToolDefinition>) -> Vec<DeferredTool> {
        definitions
            .into_iter()
            .filter_map(|definition| {
                let backend = self.backend_for_name(&definition.name)?;
                Some(DeferredTool {
                    backend: backend.id.clone(),
                    public_name: definition.name.clone(),
                    definition,
                })
            })
            .collect()
    }

    fn search(
        &self,
        tools: &[DeferredTool],
        query: &str,
        backend: Option<&str>,
        limit: usize,
    ) -> Vec<SearchResult> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| term.to_ascii_lowercase())
            .filter(|term| !term.is_empty())
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }

        let mut ranked = tools
            .iter()
            .filter(|tool| backend.is_none_or(|wanted| tool.backend == wanted))
            .filter_map(|tool| {
                let description = tool.definition.description.as_deref().unwrap_or_default();
                let schema =
                    serde_json::to_string(&tool.definition.input_schema).unwrap_or_default();
                let haystack =
                    format!("{} {} {}", tool.public_name, description, schema).to_ascii_lowercase();
                let score = terms
                    .iter()
                    .filter(|term| haystack.contains(term.as_str()))
                    .count();
                (score > 0).then(|| {
                    (
                        score,
                        SearchResult {
                            name: tool.public_name.clone(),
                            backend: tool.backend.clone(),
                            description: tool.definition.description.clone(),
                        },
                    )
                })
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        ranked
            .into_iter()
            .take(limit.min(MAX_LIMIT))
            .map(|(_, result)| result)
            .collect()
    }
}

async fn current_deferred_tools(
    proxy: &McpProxy,
    catalog: &DeferredCatalog,
) -> Result<Vec<DeferredTool>, tower_mcp::Error> {
    let mut proxy = proxy.clone();
    let request = RouterRequest {
        id: RequestId::Number(0),
        inner: McpRequest::ListTools(Default::default()),
        extensions: Extensions::new(),
    };
    let response = proxy
        .call(request)
        .await
        .map_err(|_| tower_mcp::Error::tool("internal deferred tool discovery failed"))?;
    match response.inner {
        Ok(McpResponse::ListTools(result)) => Ok(catalog.filter_tools(result.tools)),
        Ok(_) => Err(tower_mcp::Error::tool(
            "deferred discovery returned an unexpected response type",
        )),
        Err(error) => Err(tower_mcp::Error::tool(error.message)),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchInput {
    /// Terms describing the capability needed.
    query: String,
    /// Optional exact backend id, for example browser.
    backend: Option<String>,
    /// Maximum results, capped at 20.
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SearchResult {
    name: String,
    backend: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DescribeInput {
    /// Exact fully-qualified deferred tool name returned by search.
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CallInput {
    /// Exact fully-qualified deferred tool name returned by search.
    name: String,
    /// Arguments forwarded to the selected backend tool.
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Register the safe deferred-discovery backend under the reserved lomway
/// namespace. Upstream proxy/admin tools remain removed.
pub(super) async fn register(proxy: &McpProxy, catalog: DeferredCatalog) -> Result<()> {
    let search_catalog = catalog.clone();
    let search_proxy = proxy.clone();
    let search = ToolBuilder::new("search_tools")
        .description(
            "Search currently registered low-frequency Lomway tools that are intentionally hidden from the normal tool list.",
        )
        .handler(move |input: SearchInput| {
            let catalog = search_catalog.clone();
            let proxy = search_proxy.clone();
            async move {
                if input.query.trim().is_empty() {
                    return Err(tower_mcp::Error::tool("search query must not be empty"));
                }
                if let Some(backend) = input.backend.as_deref()
                    && !catalog.backend_is_deferred(backend)
                {
                    return Err(tower_mcp::Error::tool(format!(
                        "backend '{backend}' is not configured for deferred exposure"
                    )));
                }
                let tools = current_deferred_tools(&proxy, &catalog).await?;
                let results = catalog.search(
                    &tools,
                    &input.query,
                    input.backend.as_deref(),
                    input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
                );
                Ok(CallToolResult::text(serde_json::to_string_pretty(&results)?))
            }
        })
        .build();

    let describe_catalog = catalog.clone();
    let describe_proxy = proxy.clone();
    let describe = ToolBuilder::new("describe_tool")
        .description(
            "Return the exact currently registered MCP definition for one deferred Lomway tool.",
        )
        .handler(move |input: DescribeInput| {
            let catalog = describe_catalog.clone();
            let proxy = describe_proxy.clone();
            async move {
                let tools = current_deferred_tools(&proxy, &catalog).await?;
                let Some(tool) = tools.iter().find(|tool| tool.public_name == input.name) else {
                    return Err(tower_mcp::Error::tool(format!(
                        "tool '{}' is not a currently registered deferred Lomway tool",
                        input.name
                    )));
                };
                let body = serde_json::json!({
                    "name": tool.public_name,
                    "backend": tool.backend,
                    "description": tool.definition.description,
                    "inputSchema": tool.definition.input_schema,
                    "outputSchema": tool.definition.output_schema,
                    "annotations": tool.definition.annotations,
                });
                Ok(CallToolResult::text(serde_json::to_string_pretty(&body)?))
            }
        })
        .build();

    let call_catalog = catalog;
    let call_proxy = proxy.clone();
    let call = ToolBuilder::new("call_tool")
        .description(
            "Invoke one exact currently registered deferred Lomway tool. Direct/control-plane tools are rejected.",
        )
        .handler(move |input: CallInput| {
            let catalog = call_catalog.clone();
            let mut proxy = call_proxy.clone();
            async move {
                let tools = current_deferred_tools(&proxy, &catalog).await?;
                if !tools.iter().any(|tool| tool.public_name == input.name) {
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
                        Err(error) => Err(tower_mcp::Error::tool(error.message)),
                    },
                    Err(_) => Err(tower_mcp::Error::tool(
                        "internal deferred tool dispatch failed",
                    )),
                }
            }
        })
        .build();

    let router = McpRouter::new()
        .server_info("lomway-deferred-tools", env!("CARGO_PKG_VERSION"))
        .tool(search)
        .tool(describe)
        .tool(call);
    proxy
        .add_backend("lomway", ChannelTransport::new(router))
        .await?;
    Ok(())
}
