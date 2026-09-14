# ADR-0001: `mcp-proxy` libraryを基盤にする

- Status: Accepted
- Date: 2026-09-13

## Context

複数local MCPを1 endpointへ集約するには、MCP client/server両面、namespace、backend接続、health、config reload等が必要になる。これを自前実装するとprotocol追従と保守負荷が大きい。

## Decision

v1は `joshrotenberg/mcp-proxy` v0.4.3をlibraryとしてpinして利用する。MCP `2026-07-28` featureを明示的に有効化する。

repository独自コードは薄いhostに限定する。

## Consequences

Positive:

- protocol/routingの再発明を避ける。
- upstreamのfailure isolation/observabilityを再利用できる。
- Windows利用可能。

Negative:

- upstream API/config変更の影響を受ける。
- admin MCP toolsなど、用途に合わないupstream defaultを安全に抑制する必要がある。

## Rejected alternatives

- Full custom Rust MCP proxy: 不要な再実装。
- Workbridge自体へaggregation追加: Workbridgeの責務肥大化とsingle failure domain化。
- 各MCPを現状の個別connectorのまま維持: 本プロジェクトの運用簡素化目的を満たさない。
