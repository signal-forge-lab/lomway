# Architecture

更新日: 2026-09-14

## 1. 責務境界

`Lomway` は接続集約層です。process supervisor、workflow orchestrator、semantic router、public multi-tenant gatewayにはしません。

```text
ChatGPT
  | OpenAI Secure MCP Tunnel
  v
127.0.0.1:17777/mcp
  |
  v
Lomway
  |- namespace + route
  |- timeout only
  |- 起動時policy validation
  |- /healthz + /readyz + structured logs
  `- retry / hedge / cache / semantic rewriteなし
       |
       +-> 任意のlocal MCP backend A
       +-> 任意のlocal MCP backend B
       `-> ...

Process lifecycle: Swibo
Remote ingress: OpenAI Secure MCP Tunnel
Google Drive: 独立connector
```

設定されたsouthbound connectionはすべてloopback HTTP MCPです。Gatewayがbackend processをspawnすることはありません。

## 2. Upstream library境界

v1は `mcp-proxy = 0.4.3` をexact pinし、`default-features = false` + `protocol-2026-07-28` でbuildします。commit対象の `Cargo.lock` で依存解決全体を固定し、現在の検証済みlockでは `tower-mcp` 0.18.2です。

Thin Rust hostが追加する責務はupstreamだけでは表現できないproject固有の安全境界だけです。2つの構成entry point（公開 `schema_version = 1` とlegacy `[proxy]`）は、同じfail-closedな起動pipelineに合流します。

```text
config load（フォーマット自動判定。legacyはメモリ内で移行）
  -> environment reference解決
  -> project policy validation
  -> backend registry（検証済みdescriptorのみ。構成順どおり）
  -> startup probe（requiredは起動必須。optionalはdegrade。全欠損は失敗）
  -> tool collision preflight（最終tool名をserve前に確定）
  -> startup acceptance（required backendはhealthy必須、optionalはdegrade可、最低1 backendはhealthy必須）
  -> Proxy::from_config(...)
  -> remove_backend("proxy") 成功を必須化
  -> upstream routerの/admin拒否
  -> loopbackへ/mcpと/healthzと/readyzだけ公開
```

hot reloadはv1で明示的に無効です。upstream validationだけ通過し、project固有policyを再検証していないconfigがreloadされることを防ぎます。

## 3. 外部HTTP surface

project所有listenerが公開するのは以下だけです。

- Streamable HTTP MCP用の `POST/GET/DELETE /mcp`
- liveness用の `GET /healthz`
- readiness用の `GET /readyz`（required backendの起動が必須。optionalのdegradeは報告。状態はbackend healthから都度再計算）

upstream `/admin/*` は公開しません。top-level `/admin/*` と nested `/mcp/admin/*` はどちらも404で、それ以外の未定義経路も404です。このためGateway admin secret自体が不要です。

## 4. Namespace contract

separatorは `_` 固定です。最終tool名は `<prefix><upstream tool名>` で、起動時に事前計算され、衝突時は両sourceを特定してfail fastします。prefix規則: 空でない小文字stem + `_`、backend間で一意、暗黙の正規化はしない — ambiguousな入力は拒否します。reserved prefix（`proxy_`、`lomway_`、旧 `lmg_`）はpolicy管理下にあり、backendはclaimできません。

元の6-backend regression baseline（公開製品の上限ではありません）:

| Backend | Prefix | 検証済みtool数 |
|---|---|---:|
| Workbridge | `workbridge_` | 12 |
| Memory Gateway | `memory_` | 32 |
| Microsoft UFO | `ufo_` | 19 |
| Stealth Browser | `browser_` | 97 |
| XMind Workboard | `xmind_` | 21 |
| Praxiom | `praxiom_` | 2 |

元のbaseline集約数は183 toolsです。2026-09-14の再reviewでは、workstationに任意追加のChrome DevTools 30 toolsがあり、合計213 toolsでした。どちらの件数も公開contractではなく、Lomwayは検証済みpublic 0..N構成modelをsupportします。prefix変更はbreaking changeです。backendのtool descriptionとinput schemaはそのまま通ります。

## 5. Module map

```text
src/
|- config/      公開schema model・厳格validation・フォーマット判定・
|               `${VAR}` 解決・パス優先順・legacy <-> 公開schema移行
|- backend/     検証済みbackend descriptor・決定論的registry・startup probe
|- namespace/   prefix policy・reserved prefix・collision preflight
|- gateway/     policy validation（分解済みvalidator）・gateway builder
|               （起動pipeline）・router surface
|- health/      livenessとreadinessの状態model
|- cli/         引数定義とcommand dispatch（serve/check/list-backends/
|               migrate/version）
`- lib.rs       proxy構成・control-plane除去・backend再起動復旧monitor
```

`lib.rs` はbackend再起動復旧monitorも実行します: loopback port probeと安定性ヒステリシスで、offline/unhealthyになったbackendのtransport置換时机を決め、復旧したbackendはGateway再起動なしで再接続されます。

## 6. Request flow

### `tools/list`

1. clientが `/mcp` でinitialize。
2. `mcp-proxy` がinitialize成功backendから取得済みcapabilityを集約。
3. backend namespaceを付与。
4. `proxy` control-plane backendは削除済みなのでtoolを提供しない。
5. namespace以外は意味変更せずcatalogを返す。

### `tools/call`

1. prefixからbackendを決定。
2. namespaceを外し、元backend toolをcall。
3. backendごとのtimeoutだけ適用。
4. retry / hedging / failover / cacheは行わない。
5. backend result/errorをsemantic rewriteせず返す。

## 7. Protocol動作

binaryはupstream `protocol-2026-07-28` featureを有効化してbuildします。実際のprotocol versionはclientとのnegotiationで決まります。検証済みOpenAI Secure MCP Tunnel sessionはGatewayとMCP `2025-11-25` をnegotiationし、6-backend regression baselineもpinned client stackでinitialize成功しています。追加backendはこのminimum regression contractの必須条件ではありません。project code側でMCP JSON-RPC bodyの独自deserialize/rebuildは行いません。

## 8. Backend互換

Microsoft UFO / Stealth BrowserはFastMCP serverです。FastMCP既定のtool metadataに含まれる `_fastmcp` keyをpinned `tower-mcp` clientが拒否したため、backend HTTP起動側でFastMCP公式設定 `FASTMCP_INCLUDE_FASTMCP_META=false` を使いました。Gateway側にschema変換を追加せず、framework-private metadataを発生元で無効化しています。

XMind Workboardは公式upstream capability fingerprint変化時にfail closedします。今回の差分は `xmind_get_topic` descriptionへのtask read-back追加だけで、tool名/数とinput schemaは不変でした。差分review後にcacheを更新し、Workboard側のtask mutation検証もsemantic read-backへ強化しました。

## 9. Failure model

- **call中に1 backend失敗:** そのcallだけ失敗し、他namespaceは継続利用可能。
- **起動時にrequired backendが到達不能:** fail closedで起動失敗。
- **起動時にoptional backendが到達不能:** 起動はdegradeし、healthyなbackendだけでserve。
- **起動時に全backend失敗:** startup probeが失敗し、fail closedで起動失敗。
- **起動後にbackendがダウンし、その後復旧:** 再起動復旧monitorが自動で再接続します（port probe + 安定性ヒステリシス、その後transport置換）。Gateway再起動は不要。短時間のrestartと持続的なoutageの両方をE2Eで確認済み。
- **Gateway失敗:** backend processは生存したまま。SwiboがGateway/Tunnel targetだけ復旧。
- **Tunnel失敗:** Gateway/backendはlocalで生存。Tunnelだけ復旧。

## 10. Lifecycle

通常順序:

```text
1. backend MCP services
2. Lomway
3. Lomway Secure MCP Tunnel
```

Swiboでは2と3を1つのdeclarative targetとして管理し、backend targetはそれぞれ独立させます。

## 11. Observability

- `/healthz`: Gateway processのliveness。
- `/readyz`: readiness。backend healthから上限2秒で都度再計算。required backendの障害がreadyを報告することはありません。
- structured gateway logs: backend initialize / route / audit / errorを記録。secret valueは記録しない。
- Swibo: Gateway server/Tunnelを別checkし、各backendも独立health管理。
- v1ではpublic/admin metrics APIを持たない。

## 12. Upgrade境界

`mcp-proxy` または `tower-mcp` のdependency resolutionを変更するときは、config/schema review、control-plane suppression regression、fault tests、6実backend smoke、Secure Tunnel smokeを通してからlockfileを更新します。
