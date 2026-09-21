# ADR-0005: OAuth Authority を Workbridge から分離する

- Status: Accepted
- Date: 2026-09-22

## Context

Lomway は公開 MCP の Resource Server 兼集約境界だが、現在の OAuth Authorization Authority は Workbridge が提供している。公開 ingress では `/mcp` と Protected Resource Metadata を Lomway へ送り、`/authorize`、`/token`、`/register`、`/revoke` などの Authorization Server endpoint は Workbridge 所有の OAuth Authority が処理する。Lomway は loopback-only の RFC 7662 introspection endpoint を呼び出して Bearer token を検証する。

この構成自体は動作するが、Gateway 全体の認証経路が、複数 backend のうちの1製品である Workbridge に依存する責務逆転がある。そのため Workbridge と無関係な backend を Lomway 経由で利用する場合でも、認証のためだけに Workbridge が必須になり得る。

現在の公開 Lomway source は Resource Server 側、すなわち Protected Resource Metadata、Bearer challenge、resource/scope 検証、token introspection をすでに所有している。未整理なのは Authorization Server 側の所有権である。

## Decision

公開 Lomway endpoint の OAuth Authority は、Workbridge の責務ではなく **Lomway authentication stack の責務**とする。

初回実装は **Lomway-owned / loopback-only の Authorization sidecar** とする。process lifecycle は Swibo が所有する。これにより Rust gateway 自体は thin に保ちつつ、Gateway 全体の認証所有権を Workbridge から外す。

初回 migration では、新しい Authorization Server を Lomway Rust process 内へ直接入れない。将来、成熟したレビュー済み Rust Authorization Server dependency が現れ、MCP/OAuth 契約を大規模な独自 security subsystem なしで満たせることを証明できた場合のみ、別ADRで in-process 化を再検討する。

Accepted boundary は次のとおり。

1. Lomway は引き続き公開 MCP Resource Server / aggregation boundary を所有する。
2. Lomway-owned sidecar が Authorization Server role とローカル token authority を所有する。
3. 公開 OAuth issuer と公開 endpoint shape は cutover 前後で原則維持する。内部 process を変えるためだけに client から見える OAuth identity を変更しない。
4. Workbridge は通常の optional backend に戻り、Gateway 全体の認証を供給しない。
5. sidecar の supervision は Lomway ではなく Swibo が所有する。
6. 1つの公開 issuer に対して active Authorization Authority は常に1つだけとする。
7. Workbridge の実績済み behavior は compatibility baseline として利用するが、実装を恒久的に Lomway へ複製しない。

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
- MCP `2026-07-28` に沿った client identification / registration。該当時の pre-registration、open client の優先方式としての Client ID Metadata Documents (CIMD)、後方互換 fallback としての Dynamic Client Registration (DCR)
- PKCE S256 を使用する Authorization Code flow
- Authorization / token request における RFC 8707 `resource` 検証と resource-bound token 発行
- RFC 9207 Authorization Response `iss` と、利用時の metadata advertisement
- access token / refresh token 発行
- refresh token rotation
- token revocation
- authorization approval と owner authentication
- registered client / token の永続化
- redirect URI 検証と authorization attempt rate limit
- CIMD の安全な取得・検証。SSRF 対策、client document identity の完全一致、redirect URI の完全一致を含む

Authority を sidecar とする場合、token introspection は loopback-only のままとする。in-process の場合は HTTP introspection を介さず内部 interface で検証する。

### Workbridge

Workbridge は Gateway 全体の OAuth を所有しない。Workbridge MCP backend の lifecycle と製品固有機能は Lomway authentication から独立させる。

## Deployment shape options

### Option A: Workbridge Authority を維持

直近の変更量は最小だが、現在の製品間依存と failure coupling が残る。これは rollback 用構成であり、target 構成とはしない。

### Option B: Lomway process 内に Authority を統合

runtime 構成を最小にでき、introspection の network hop も消せる。**初回 migration では不採用。** 現時点でレビューした Rust の選択肢では、この deployment に必要な Authorization Server behavior を十分成熟した再利用可能componentとして提供できず、Lomway が security-sensitive な application code を大きく所有することになる。

### Option C: Lomway-owned sidecar Authority

1 process 増える代わりに、thin gateway を維持しつつ Workbridge 依存を解消できる。sidecar は必要最小限の Authorization surface と loopback-only introspection interface のみを提供し、start/stop/restart は Swibo が所有する。**初回実装として採用。**

## Implementation research

このADRをAcceptedにする前に実施した調査結果は次のとおり。

- MCP `2026-07-28` は Dynamic Client Registration より Client ID Metadata Documents を優先する。DCR は後方互換 mechanism であり、唯一の client registration path として設計してはならない。
- 同revisionでは RFC 9207 issuer validation と Authorization Server credential binding が強化されている。したがって public issuer は見た目だけのURLではなく migration contract の一部である。
- 公式 Rust MCP SDK は OAuth client 機能と Authorization Server の参考exampleを持つが、Lomway の persistence、owner approval、rate limit、CIMD fetch hardening、migration 要件までを含む drop-in production Authorization Server component は提供していない。example を in-process 採用しても、Lomway が大量の Authorization Server application code を所有することになる。
- 新興の Rust Authorization Server crate は、このsecurity boundaryの初回採用基準としては成熟度が不足している。将来の再評価対象にはできるが、初回 migration のdependencyにはしない。
- MCP TypeScript SDK v2 は v1 の Authorization Server helper を deprecated な `server-legacy` package へ移し、dedicated OAuth/IdP library への migration を明示的に推奨している。このため現在の Workbridge SDK v1 Authority をそのまま恒久的な Lomway 実装にしてはならない。
- sidecar では成熟した dedicated Authorization Server library を利用する形を優先する。`oidc-provider` は RFC 8414 discovery、PKCE、DCR、revocation、introspection、RFC 8707 Resource Indicators、RFC 9207、experimental CIMD support を持つ候補である。ただし pin 前に MCP `2026-07-28` compatibility/security spike を通すこと。ADR が受理するのは sidecar boundary であり、未レビューversionのlibrary採用ではない。
- 現在の Workbridge 実装は behavioral / migration baseline として価値がある。single-owner approval、強いrandom opaque token、resource/scope binding、refresh-token rotation、revocation、rate limiting、OAuth state永続化は現行deploymentで実績がある。

以上により Option C を現在のAccepted decisionとする。Authority libraryのpinは、MCP conformance、ChatGPT interoperability、security review、通常のdependency adoption gateを通した実装taskとする。

## Current Workbridge Authority との compatibility contract

今回の ownership migration は、user-facing login flow の再設計を目的としない。

- 公開 MCP URL は変更しない。
- 公開 OAuth issuer は原則変更しない。
- 既存の公開 Authorization endpoint shape (`/authorize`, `/token`, `/revoke`、互換性が必要な期間のDCR `/register`) は ingress 経由で維持する。
- single-user approval は owner-password approval のまま維持する。branding は Workbridge-owned authentication から Lomway-owned authentication へ変わるが、interaction model は維持する。
- canonical owner credential は Workbridge-owned state から Lomway authentication の secret boundary へ移す。値を安全に移行して同じpasswordを使い続けることは可能だが、Workbridgeを正本にはしない。
- CIMD support と RFC 9207 `iss` は protocol hardening として追加する。対応clientがDCRを必要とする間はDCRを削除しない。
- existing client registration / token state は安全性を証明できる場合は移行する。seamless token migration を証明できない場合は、一度だけ明示的に再認証させることを許容するが、silent credential breakage は許容しない。

## Migration plan

1. **現行契約とspec gapをcharacterizeする。** Protected Resource Metadata、Authorization Server Metadata、現行DCR、PKCE Authorization、token発行、refresh、revocation、introspection、resource binding、scope enforcement、restart behavior、現行HTTP status/challenge behaviorをintegration test化する。現在存在するという理由だけでobsolete behaviorを固定しない。
2. **sidecar dependency spikeを実施する。** Authority libraryをpinする前に、MCP `2026-07-28` discovery、CIMD、DCR fallback、PKCE S256、RFC 8707 resource binding、RFC 9207 `iss`、refresh/revocation/introspection、persistence hook、owner approval integrationを検証する。
3. **state migrationを定義する。** registered client、access token、refresh token、client-registration integrity key、owner credentialを明示的に扱う。短命なauthorization codeは移行しない。durable credentialを意図的に無効化する場合は明示的な再認証planを必須とする。
4. **Lomway-owned state / secret boundaryを作る。** OAuth stateはWorkbridge state directory外へ置き、長期secretはLomwayのcanonical external secret mechanismから解決する。
5. **同じpublic issuerでsidecarを導入する。** 現行Workbridge Authorityは一時的rollback pathとしてのみ残す。sidecarのloopback introspection endpointを公開しない。
6. **互換性試験は並列で行うがproduction dual writerにはしない。** 1つのpublic issuerにactive Authorityは常に1つだけとする。
7. **ingress routingをcutoverする。** stable public issuer/endpointsを維持したまま、内部Authorization routeだけをWorkbridgeからLomway-owned sidecarへ切り替える。実ingress経由でclient identification/registrationからauthorizationまで確認する。
8. **backend independenceを証明する。** Workbridgeを停止し、少なくとも1つの非Workbridge backendへ認証済みアクセスできることを確認する。
9. **refresh continuityを証明する。** 選択したclient flowがrefresh tokenを利用する場合、実ChatGPT connectionがaccess-token expiry後も手動再認証なしにrefreshできることを確認する。
10. **互換pathを削除する。** rollback window終了後、WorkbridgeのGateway Authority couplingを削除し、恒久的な二重分岐を残さない。

## Acceptance criteria

- 対応する remote MCP client が、MCP `2026-07-28`互換の方式でidentify/registerし、安定したHTTPS public URL経由でauthorize、token取得、refresh、revoke、`/mcp`呼び出しまで完了できる。
- CIMDをopen clientの優先registration mechanismとしてsupportし、後方互換が必要な間はDCRをfallbackとして維持する。
- WorkbridgeからLomway Authorityへのcutover前後でpublic Authorization Server issuerを維持する。変更が不可避な場合は別の明示的migration decisionを必要とする。
- Workbridge が停止または存在しなくても、他の Lomway backend の OAuth が壊れない。
- 未認証 `/mcp` は正しい Protected Resource Metadata を示す `401` を返す。
- expire / revoke / resource mismatch tokenを拒否する。missing/invalid authenticationは正しい`401` challenge、insufficient scopeは現行MCP/RFC 6750に沿った`403 insufficient_scope`とし、legacy `401` behaviorを固定しない。
- Authorization Code flow では PKCE S256 を必須とする。
- Authorization / token requestでRFC 8707 resource bindingを強制する。
- Authorityが`authorization_response_iss_parameter_supported`をadvertiseする場合、Authorization ResponseにRFC 9207 `iss`を含める。
- redirect URI を allowlist に基づき検証する。
- owner authentication 失敗を rate limit する。
- 承認済み migration 方針で再認証を要求する場合を除き、通常 restart 後も OAuth 永続 state を維持する。
- 実ChatGPT OAuth connectionがWorkbridgeへ依存せずaccess-token expiry後にrefreshできる。
- secret / token material を commit・log しない。長期 secret の正本は repository 外に置き、既存の secret mechanism から解決する。
- core authentication design を tunnel vendor 非依存にする。Tunnel は HTTPS ingress を提供するだけで、OAuth semantics を所有しない。
- Authority が別 process の場合も、その process supervision は Lomway ではなく Swibo が所有する。

## Security constraints

- Lomway および Authority/introspection listener はローカルでは loopback-only を維持する。
- introspection endpoint を外部公開しない。
- CIMD retrievalをSSRF boundaryとして扱う。HTTPS限定、strict URL validation、credential-bearing URL禁止、unsafe redirect禁止、DNS/private-address defense、response size/time上限、`client_id`とdocument URLの完全一致、redirect URI完全一致を必須とする。
- token issuance / refresh / revocation に、idempotency を明示せず自動 retry を追加しない。
- bearer token、authorization code、refresh token、owner credential、client secret をlogしない。
- authorization state を Git working tree に置かない。
- introspection が `active=true` という理由だけで許可せず、resource と scope の binding を維持する。
- issuer bindingを維持する。DCR / pre-registered credentialを異なるAuthorization Server issuerへ暗黙再利用しない。

## Implementation details still to resolve

- focused MCP `2026-07-28` spikeとrepository dependency-review gateを通過するdedicated Authorization Server library/versionはどれか。
- sidecarをこのrepositoryのsibling package/binaryとして置くか、Lomway product scopeの別public repositoryに置くか。どちらでもownershipはLomway、lifecycle ownershipはSwiboとする。
- existing registered client、access token、refresh token、registration-integrity stateを、Workbridgeをruntime dependencyとして残さず移行または再確立する具体方式。
- 現在の Workbridge OAuth setting のうち、汎用 Gateway policy と Workbridge 固有policyをどう分離するか。
- 現在の `openai-secure-mcp-tunnel` connection mode 名は特定transportを表しており、実際の公開経路と一致しない場合がある。移行時に transport-neutral な名称へ置き換えるか。

これらはimplementation detailであり、Accepted済みのownership/process boundaryを妨げない。public issuer変更、2つのactive Authority導入、またはprocess supervisionのLomway本体への移動を行う場合は、新規または改訂ADRを必要とする。

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
- 初回実装ではsupervised local processが1つ増えるが、Lomway Rust gateway自体をIdentity Server codebaseへ変質させずに済む。

## References

- MCP Authorization Specification `2026-07-28`: https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization
- MCP `2026-07-28` release notes: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- Official Rust MCP SDK OAuth support: https://github.com/modelcontextprotocol/rust-sdk/blob/main/docs/OAUTH_SUPPORT.md
- MCP TypeScript SDK v1-to-v2 auth migration: https://ts.sdk.modelcontextprotocol.io/v2/migration/upgrade-to-v2
- `oidc-provider` implemented specifications: https://github.com/panva/node-oidc-provider
