# 設計根拠 / Sources

確認日: 2026-09-13

設計・実装reviewで確認した主要source:

1. **Model Context Protocol — 2026-07-28 specification announcement**
   - https://blog.modelcontextprotocol.io/posts/2026-07-28/
   - Stateless core、header routing、cacheable list results等を確認。

2. **OpenAI — Developer mode and MCP apps in ChatGPT**
   - https://help.openai.com/en/articles/12584461
   - private/on-prem/developer-machine MCPで利用するSecure MCP Tunnel経路を確認。

3. **`joshrotenberg/mcp-proxy`**
   - https://github.com/joshrotenberg/mcp-proxy
   - aggregation、namespace、timeout、control-plane behavior、Windows support、library APIを確認。

4. **`mcp-proxy` v0.4.3 release**
   - https://github.com/joshrotenberg/mcp-proxy/releases/tag/v0.4.3
   - v1でexact pinしたupstream version。

5. **`mcp-proxy` source: proxy/admin implementation**
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/src/proxy.rs
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/src/admin_tools.rs
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/src/admin.rs
   - construction、`mcp_proxy()` access、admin MCP backend自動登録、HTTP admin routeを確認。

6. **`mcp-proxy` source: Cargo feature/MSRV definition**
   - https://github.com/joshrotenberg/mcp-proxy/blob/main/Cargo.toml
   - Rust 1.90 MSRV、`protocol-2026-07-28` featureを確認。

7. **Official Rust MCP SDK**
   - https://github.com/modelcontextprotocol/rust-sdk
   - Rust ecosystemのprotocol supportをcross-check。

8. **RustSec Advisory Database / OSV**
   - https://rustsec.org/
   - https://osv.dev/
   - 2026-09-13のFinal Reviewでexact lockfile setをcurrent advisory dataと照合。

upstream behaviorは変化し得るため、dependency/protocol version bumpごとに再確認します。
