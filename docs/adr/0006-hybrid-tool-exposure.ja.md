# ADR-0006: Direct / Deferred のHybrid Tool Exposureを採用する

- Status: Accepted
- Date: 2026-09-27

## Context

ADR-0003では、まずnative full tool catalogで開始し、実測で問題を確認してから
exposure方式を変更する方針としていました。Lomway WindowsのChatGPT可視toolは
259件まで増え、そのうちStealth Browserだけで97件になりました。

Web Chat実機probeでは、97-toolのbrowser backendを一時的に1-tool MCPへ差し替えました。
Lomway自身の tools/list はbrowser 1件へ変化した一方、同じWeb Chat応答では既に
読み込まれていた97件のbrowser schemaが残りました。このため、同一応答内の
tools/list_changed追従をDeferred Loadingの必須条件にはできません。

pinned mcp-proxyにはglobal search exposureがありますが、search/call toolは
proxy/configやproxy/add_backend等の管理toolと同じupstream proxy backendに属します。
Lomwayはこのcontrol-plane backendを意図的に削除しています。

## Decision

Hybrid exposureを採用します。

- 常用backendはdirect exposureとし、native schemaをtools/listへ出します。
- 低頻度backendはdeferred exposureとし、通常のtools/listとdirect callから隠します。
- Lomwayは安全なmeta-toolを3本だけ公開します:
  lomway_search_tools / lomway_describe_tool / lomway_call_tool。
- search/describeはraw proxyの現在登録済みcatalogから正確なtool definitionを取得し、設定上のdeferred prefixで絞り込みます。
- lomway_call_toolは現在catalogに存在し、設定上のdeferred prefixに属する正確なtool名だけを許可し、
  retryせずbackend resultを転送します。
- 起動時に停止していたconfigured optional backendもreconnect monitorの監視対象に残り、外部Supervisorがendpointを起動した後に自動採用します。
- upstream proxy admin backendは引き続き削除し、meta-tool経由でも呼べません。

backend process lifecycleは引き続きSwiboが所有します。Deferred exposureはbackendを
起動・停止しません。

## Consequences

- Web Chatは安定した小さいschema集合だけを保持し、同一turnのtool refreshに依存しません。
- 常用toolはnative typingとdirect selectionを維持します。
- 低頻度toolはsearch/describeを経てgeneric callする1段階が増えます。
- Discoveryは独立したhealth oracleではなく登録状態を扱います。登録後にofflineになったbackendはcall時に通常の失敗として表面化します。
- 任意tool-name routerではなく、明示的なdeferred allowlistで保護します。
- 公開schemaへbackend単位の exposure = "direct" | "deferred" を追加し、
  互換性のため既定値はdirectとします。
