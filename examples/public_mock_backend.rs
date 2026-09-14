use anyhow::Result;
use axum::serve;
use schemars::JsonSchema;
use serde::Deserialize;
use tower_mcp::{CallToolResult, HttpTransport, McpRouter, ToolBuilder};

#[derive(Debug, Deserialize, JsonSchema)]
struct StatusInput {}

#[tokio::main]
async fn main() -> Result<()> {
    let address = std::env::var("LOMWAY_SMOKE_BACKEND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:18701".to_string());
    let tool = ToolBuilder::new("status")
        .description("Return public clean-machine smoke status")
        .handler(|_: StatusInput| async move { Ok(CallToolResult::text("ok")) })
        .build();
    let router = McpRouter::new()
        .server_info("lomway-public-smoke", env!("CARGO_PKG_VERSION"))
        .tool(tool);
    let app = HttpTransport::new(router).into_router_at("/mcp");
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("public mock backend listening on {address}");
    serve(listener, app).await?;
    Ok(())
}
