# ADR-0005: OAuth Authority を Workbridge から分離する

- Status: Proposed
- Date: 2026-09-22

## Context

Lomway は公開 MCP の Resource Server 兼集約境界だが、現在の OAuth Authorization Authority は Workbridge が提供している。公開 ingress では `/mcp` と Protected Resource Metadata を Lomway へ送り、`/authorize`、`/token`、`/register`、`/revoke` などの Authorization Server endpoint は Workbridge 所有の OAuth Authority が処理する。Lomway は loopback-only の RFC 7662 introspection endpoint を呼び出して Bearer token を検証する。

この構成自体は動作するが、Gateway 全体の認証経路が、複数 backend のうちの1製品である Workbridge に依存する責務逆転がある。そのため Workbridge と無関係な backend を Lomway 経由で利用する場合でも、認証のためだけに Workbridge が必須になり得る。

現在の公開 Lomway source は Resource Server 側、すなわち Protected Resource Metadata、Bearer challenge、resource/scope 検証、token introspection をすでに所有している。未整理なのは Authorization Server 側の所有権である。

## Proposed decision

公開 Lomway endpoint の OAuth Authority は、Workbridge の責務ではなく **Lomway authentication stack の責務**とする。

これは所有権の方針であり、process 構成まではまだ決定しない。Lomway の thin gateway 方針を維持できる最小の実装を選ぶ。

1. 既存のレビュー済み library で必要な OAuth/MCP behavior を満たせ、巨大な独自 security subsystem を持たずに済む場合のみ、Lomway process 内への統合を優先する。
2. それが難しい場合は、汎用的な Lomway 所有の Authorization Authority を loopback-only の別 process として抽出または実装する。process lifecycle は引き続き Swibo が所有する。
3. Workbridge の OAuth 実装を Lomway へ単純コピーして重複させない。実績のある処理を可能な限り再利用・抽出する。

移行後、Workbridge は通常の optional backend として扱えること。Workbridge の停止・削除が、他の Lomway backend の認証を停止させてはならない。

## Target responsibility split

### Lomway Resource Server

Lomway は引き続き次を所有する。

- `/mcp` の Bearer authentication / authorization
- `/.well-known/oauth-protected-resource/mcp`
- resource binding と required scope の強制
- 公開 `WWW-Authenticate` challenge
- 選択された Lomway-owned Authority との連携

### Lomway Authentication Authority

選択された Authority 実装は次を所有する。

- Authorization Server Metadata
- 対応 MCP client が必要とする Dynamic Client Registration
- PKCE S256 を使用する Authorization Code flow
- access token / refresh token 発行
- refresh token rotation
- token revocation
- authorization approval と owner authentication
- registered client / token の永続化
- redirect URI 検証と authorization attempt rate limit

Authority を sidecar とする場合、token introspection は loopback-only のままとする。in-process の場合は HTTP introspection を介さず内部 interface で検証する。

### Workbridge

Workbridge は Gateway 全体の OAuth を所有しない。Workbridge MCP backend の lifecycle と製品固有機能は Lomway authentication から独立させる。

## Deployment shape options

### Option A: Workbridge Authority を維持

直近の変更量は最小だが、現在の製品間依存と failure coupling が残る。これは rollback 用構成であり、target 構成とはしない。

### Option B: Lomway process 内に Authority を統合

runtime 構成を最小にでき、introspection の network hop も消せる。必要な OAuth behavior をレビュー済み upstream/library で提供でき、Lomway 側 interface を小さく保てる場合のみ採用する。Gateway 自体が大規模な独自 Identity Server になる場合は採用しない。

### Option C: Lomway-owned sidecar Authority

1 process 増える代わりに、thin gateway を維持しつつ Workbridge 依存を解消できる。sidecar は最小限の公開 Authorization endpoint と loopback-only introspection interface のみを提供し、start/stop/restart は Swibo が所有する。

## Migration plan

1. **現行契約を固定する。** Protected Resource Metadata、Authorization Server Metadata、Dynamic Registration、PKCE Authorization、token 発行、refresh、revocation、introspection、resource binding、scope enforcement、restart behavior を integration test 化する。
2. **Authority の物理構成を選ぶ。** まず pin 済み MCP/OAuth dependency を調査し、新規 OAuth 実装より既存実装の再利用を優先する。
3. **state migration を決める。** registered client / refresh token を移行するか、一度だけ明示的な再認証を要求するかを決定する。永続 credential を暗黙に無効化しない。
4. **Authority ownership を移す。** Lomway-owned Authority を導入し、現行 Workbridge Authority は一時的な rollback path としてのみ残す。
5. **互換性試験は並列で行うが、production の dual writer にはしない。** 1つの公開 endpoint に対して active Authority は常に1つだけとする。
6. **公開 endpoint を切り替える。** 実 ingress 経由で client registration から authorization まで end-to-end で確認する。
7. **backend independence を証明する。** Workbridge を停止し、少なくとも1つの非Workbridge backendへ認証済みアクセスできることを確認する。
8. **互換 path を削除する。** rollback window 終了後、Workbridge の Gateway Authority coupling を削除し、恒久的な二重分岐を残さない。

## Acceptance criteria

- 対応する remote MCP client が、安定した HTTPS public URL 経由で register、authorize、token取得、refresh、revoke、`/mcp` 呼び出しまで完了できる。
- Workbridge が停止または存在しなくても、他の Lomway backend の OAuth が壊れない。
- 未認証 `/mcp` は正しい Protected Resource Metadata を示す `401` を返す。
- expire、revoke、resource mismatch、required scope 不足の token を拒否する。
- Authorization Code flow では PKCE S256 を必須とする。
- redirect URI を allowlist に基づき検証する。
- owner authentication 失敗を rate limit する。
- 承認済み migration 方針で再認証を要求する場合を除き、通常 restart 後も OAuth 永続 state を維持する。
- secret / token material を commit・log しない。長期 secret の正本は repository 外に置き、既存の secret mechanism から解決する。
- core authentication design を tunnel vendor 非依存にする。Tunnel は HTTPS ingress を提供するだけで、OAuth semantics を所有しない。
- Authority が別 process の場合も、その process supervision は Lomway ではなく Swibo が所有する。

## Security constraints

- Lomway および Authority/introspection listener はローカルでは loopback-only を維持する。
- introspection endpoint を外部公開しない。
- token issuance / refresh / revocation に、idempotency を明示せず自動 retry を追加しない。
- bearer token、authorization code、refresh token、owner credential、client secret をlogしない。
- authorization state を Git working tree に置かない。
- introspection が `active=true` という理由だけで許可せず、resource と scope の binding を維持する。

## Open questions for the implementation task

- pin 済み MCP/OAuth stack だけで、現在実績のある Workbridge 実装を抽出するより低リスクに完全な Authorization Server を提供できるか。
- sidecar を選ぶ場合、この repository の別binaryにするか、Lomway product scope の別public repositoryにするか。
- 既存 registered client / refresh token を移行するか、明示的な一度限りの再認証を許容するか。
- 現在の Workbridge OAuth setting のうち、汎用 Gateway policy と Workbridge 固有policyをどう分離するか。
- 現在の `openai-secure-mcp-tunnel` connection mode 名は特定transportを表しており、実際の公開経路と一致しない場合がある。移行時に transport-neutral な名称へ置き換えるか。

## Non-goals

- multi-user identity management
- social login / enterprise identity federation
- 外部 HTTPS tunnel provider の置換
- backend lifecycle ownership の Lomway への移動
- 同一公開 endpoint に対する2つの active Authorization Authority の恒久運用

## Consequences

- Gateway authentication が特定backend製品へ依存しなくなる。
- Lomwayから見てWorkbridgeを通常の optional backend に戻せる。
- 公開Gateway seamとauthentication ownershipが一致し、物理process構成は最小のものを選択できる。
- 切替前にsecurity-sensitiveな互換性・永続化試験が必要になる。
