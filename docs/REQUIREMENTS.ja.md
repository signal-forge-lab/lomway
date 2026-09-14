# Requirements

更新日: 2026-09-13

## 1. Functional requirements

| ID | 要件 | Status |
|---|---|---|
| FR-01 | `127.0.0.1:17777/mcp` に単一Streamable HTTP MCP endpointを提供 | PASS |
| FR-02 | Workbridge / Memory Gateway / Microsoft UFO / Stealth Browser / XMind Workboard / Praxiomを集約 | PASS |
| FR-03 | `<backend>_<tool>` namespaceでtool衝突回避 | PASS |
| FR-04 | 1 backend失敗時もhealthy namespaceを継続利用 | PASS |
| FR-05 | backend schema/resultをsemantic rewriteせず維持 | PASS |
| FR-06 | Gateway再installなしでlocal config + restartによりendpoint/config変更を反映 | PASS |
| FR-07 | `/healthz` でGateway process healthを提供し、backend healthは独立Supervisor側に維持 | PASS |
| FR-08 | 集約endpoint用Local MCP connector/Tunnel 1組を利用可能 | PASS |
| FR-09 | Google Driveは独立維持 | PASS |
| FR-10 | upstream control-plane MCP toolsをclient surfaceから除去 | PASS |
| FR-11 | backend lifecycleをGatewayへ持ち込まない | PASS |
| FR-12 | unsafe production policyをfail closedで拒否 | PASS |

## 2. Security requirements

- listenは `127.0.0.1` のみ。
- 外部公開される `/admin/*` management surfaceは存在しない。
- serve前にupstream `proxy` MCP backendを削除。
- ChatGPTからbackend URLを動的登録できない。
- source / example config / logs / tool resultへsecret valueを残さない。
- Secure Tunnel credentialは外部の正本SOPS storeからprocess内だけへ読み込む。
- public Gitへuser固有absolute path、live tunnel ID、machine-local endpoint inventory、local config、runtime logを含めない。

## 3. Reliability requirements

- automatic retry / hedging禁止。
- tool-call result cache禁止。
- fan-out / failover / composite dispatch禁止。
- timeout後にmutationを再送しない。
- startup時に1 backend失敗しても、最低1 backend成功ならfailed backendをskip可能。
- 初期構築時に全backend失敗ならfail closedで起動失敗。
- skip後に復旧したbackendはv1ではGateway restartで再discovery。

## 4. Maintainability requirements

- backend domain logicはbackend側に保持。
- backend process ownershipはSwibo/各target wrapperに保持。
- `mcp-proxy` はforkせずexact pin。
- resolved dependencyをlockfile固定。
- 通常のbackend追加はconfiguration-first。
- public docsは英語/日本語を対で維持。

## 5. Acceptance criteria / evidence

| Criterion | Evidence | Result |
|---|---|---|
| Single endpoint | Gateway `/mcp` + 6実backend initialize | PASS |
| Namespace collision | 同名`status` mock E2E | PASS |
| Failure isolation | startup時failed backend skip + healthy backend call | PASS |
| Control-plane absence | aggregate `proxy_*` count = 0 | PASS |
| Admin-plane isolation | `/admin/*` と `/mcp/admin/*` が404 | PASS |
| No duplicate mutation | forced timeout counter = 1 | PASS |
| Real backend parity | 12 + 32 + 19 + 97 + 21 + 2 = 183 tools | PASS |
| Representative routing | 6 namespaceすべてsafe call成功 | PASS |
| Secure Tunnel | native runtime READY + MCP probe/initialize成功 | PASS |
| Supervisor lifecycle | Swibo READY/restart/STOPPED/start/READY | PASS |
| Static quality | fmt / clippy `-D warnings` / full tests / release build | PASS |
| Public hygiene | final secret/path/runtime scan | Final Reviewで必須 |

## 6. Explicit non-requirements

- LLM tool routerなし。
- generic `call_tool(name,args)` dispatcherなし。
- 新GUIなし。
- Google Drive proxyなし。
- public/LAN listenerなし。
- v1にadmin/metrics control planeなし。
- v1にhot reload/reconciliation loopなし。
