# Design Sources

Reviewed: 2026-09-13

Primary sources used for this design and implementation review:

1. **Model Context Protocol — 2026-07-28 specification announcement**
   - https://blog.modelcontextprotocol.io/posts/2026-07-28/
   - Reviewed stateless core, header routing, cacheable list results, and related changes.

2. **OpenAI — Developer mode and MCP apps in ChatGPT**
   - https://help.openai.com/en/articles/12584461
   - Reviewed the Secure MCP Tunnel path used for private/on-prem/developer-machine MCP servers.

3. **`joshrotenberg/mcp-proxy`**
   - https://github.com/joshrotenberg/mcp-proxy
   - Reviewed aggregation, namespaces, timeout, control-plane behavior, Windows support, and library API.

4. **`mcp-proxy` v0.4.3 release**
   - https://github.com/joshrotenberg/mcp-proxy/releases/tag/v0.4.3
   - Exact upstream version used by v1.

5. **`mcp-proxy` source: proxy/admin implementation**
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/src/proxy.rs
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/src/admin_tools.rs
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/src/admin.rs
   - Confirmed construction, `mcp_proxy()` access, automatic admin MCP backend registration, and HTTP admin routes.

6. **`mcp-proxy` source: Cargo feature/MSRV definition**
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/Cargo.toml
   - Confirmed Rust 1.90 MSRV and the `protocol-2026-07-28` feature.

7. **Official Rust MCP SDK**
   - https://github.com/modelcontextprotocol/rust-sdk
   - Cross-checked Rust ecosystem protocol support.

8. **RustSec Advisory Database / OSV**
   - https://rustsec.org/
   - https://osv.dev/
   - Final dependency review checked the exact lockfile set against current advisory data on 2026-09-13.

Upstream behavior can change. Revalidate these sources on every dependency/protocol version bump.
