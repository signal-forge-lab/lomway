use std::path::Path;

use mcp_proxy::ProxyConfig;
use tower_mcp::client::{HttpClientTransport, McpClient};

#[tokio::test]
#[ignore = "requires the machine-local backend set to be running"]
async fn all_local_backends_expose_tools_to_the_upstream_client_stack() {
    let config = ProxyConfig::load(Path::new("config/proxy.local.toml"))
        .expect("load ignored machine-local config");

    let mut failures = Vec::new();
    for backend in config.backends {
        let Some(url) = backend.url else {
            failures.push(format!("{}: missing URL", backend.name));
            continue;
        };
        let transport = HttpClientTransport::new(&url);
        let client = match McpClient::connect(transport).await {
            Ok(client) => client,
            Err(error) => {
                failures.push(format!("{}: connect: {error:#}", backend.name));
                continue;
            }
        };
        if let Err(error) = client.initialize("lomway-real-test", "0.1.0").await {
            failures.push(format!("{}: initialize: {error:#}", backend.name));
            continue;
        }
        match client.list_all_tools().await {
            Ok(tools) if !tools.is_empty() => {
                eprintln!("{}: {} tools", backend.name, tools.len());
            }
            Ok(_) => failures.push(format!("{}: tools/list returned zero tools", backend.name)),
            Err(error) => failures.push(format!("{}: tools/list: {error:#}", backend.name)),
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
