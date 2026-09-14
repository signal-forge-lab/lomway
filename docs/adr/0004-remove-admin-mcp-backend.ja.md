# ADR-0004: upstream control planeをnorthbound surfaceから除去する

- Status: Accepted / Implemented
- Date: 2026-09-13

## Context

`mcp-proxy` 0.4.3にはChatGPT aggregation用途では不要なcontrol planeが2種類あります。

1. config確認・dynamic backend登録等の管理toolを持つ `proxy` MCP backend
2. management operationを含むHTTP `/admin/*`

どちらもnorthboundへ残すと、aggregation目的に不要なauthorityを増やします。

## Decision

- `Proxy::from_config()` 後、serve前に `proxy.mcp_proxy().remove_backend("proxy")` を実行し、成功しなければfail closed。
- project-owned listenerからupstream `/admin/*` を公開しない。
- nested `/mcp/admin/*` もupstream route dispatch前に拒否。
- `/mcp` とproject-owned read-only `/healthz` process checkだけ公開。

したがってv1にはGateway admin token不要です。

## Consequences

- ChatGPTからaggregate MCP経由でbackend追加/削除/config変更不可。
- MCP client侵害時もupstream proxy control planeへ到達不可。
- local monitoringはupstream admin APIではなく `/healthz` + Swiboを利用。
- upstream updateごとにMCP管理tool不在testとadmin-path negative testを再実行。
