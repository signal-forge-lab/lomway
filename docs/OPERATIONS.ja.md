# Operations

更新日: 2026-10-07

## 1. Ownership

backend MCPはそれぞれ独立したSwibo targetのままです。`Lomway` はGatewayに加えて
OAuth authority sidecarを所有します。Swiboではsidecarを独立した `OAuth` componentとして
監視し、Gatewayとは別にhealthを確認できます。

```text
start: OAuth sidecar -> Gateway -> Secure MCP Tunnel
stop:  Secure MCP Tunnel -> Gateway -> OAuth sidecar
```

Gatewayがbackend MCP processをstart/stopすることはありません。
OAuth sidecarは `127.0.0.1:7677` へ直接bindし、定常構成ではforwarderを使用しません。

## 2. 正常状態

Swibo上の期待値:

```text
state  = READY
server = READY
tunnel = READY
error  = empty
```

実地検証済みlifecycle:

```text
READY -> restart -> READY -> stop -> STOPPED -> start -> READY
```

## 3. Direct runtime commands

初期設定・診断時に利用します。通常lifecycleはSwiboから操作できます。

```powershell
pwsh -NoProfile -File scripts/check.ps1
pwsh -NoProfile -File scripts/oauth-sidecar.ps1 -Action status
pwsh -NoProfile -File scripts/start.ps1
pwsh -NoProfile -File scripts/status.ps1
pwsh -NoProfile -File scripts/integration-smoke.ps1
pwsh -NoProfile -File integrations/openai-secure-tunnel/tunnel.ps1 -Action status
```

Tunnel one-time作成/reuse（オプション統合）:

```powershell
pwsh -NoProfile -File integrations/openai-secure-tunnel/configure-tunnel.ps1 -WorkspaceId <workspace-id>
```

## 4. Start / Stop order

集約target起動前に、通常は6 backend serviceをREADYにします。

```text
Start:
  backend services -> OAuth sidecar -> Gateway -> Secure MCP Tunnel

Stop aggregate target only:
  Secure MCP Tunnel -> Gateway -> OAuth sidecar
```

aggregate target停止でbackendは停止しません。

## 5. Health / Incident isolation

### Gateway process

`GET http://127.0.0.1:17777/healthz` はGateway process readinessのみを返します。backend healthとは意図的に分離します。

### 1 backend failure

- Gateway/Tunnelは止めない。
- 対象backend targetだけ復旧。
- Gateway構築時にskipされていた場合はbackend復旧後にGateway restart。
- `scripts/integration-smoke.ps1` でcatalogを再確認。

### Gateway failure

- Swibo `Lomway` targetだけrestart。
- backend全再起動は既定で行わない。

### Tunnel failure

- Gateway/backendは維持。
- `integrations/openai-secure-tunnel/tunnel.ps1 -Action ensure` またはSwibo restartでTunnelだけ復旧。

### XMind schema drift

XMind Workboardのfail-closed設計を維持します。旧/新fingerprintとschema差分を確認し、内容を理解した後だけcacheを更新します。集約Gateway側でupstream driftを隠す変換は行いません。

## 6. Config change

v1はhot reloadしません。

1. local-only config/environment sourceを変更。
2. `scripts/check.ps1`。
3. Swiboの `Lomway` targetをrestart。
4. `/healthz` 確認。
5. `scripts/integration-smoke.ps1`。
6. Swibo server/tunnel READYを確認。

## 7. Rollback

aggregation導入自体はbackend domain state/sourceを変更しません。aggregate pathを戻す場合:

1. aggregate Gateway target/connectorをstopまたはdisable。
2. 必要に応じて以前の個別connector/Tunnel設定を再利用。
3. backend processとGoogle Driveはそのまま維持。

旧connectorの削除は別のmigration判断であり、本実装の動作完了条件には含めません。

## 8. Logs / state

- Gateway logs/PIDはGit ignored local directory。
- OAuth sidecarのlogs/PID/stateは `%LOCALAPPDATA%\Lomway\oauth-sidecar`。
- OAuth signing material、cookie key、owner credentialはprocess起動時に
  `~/.config/sops/secrets/global.sops.json` からのみ読み込みます。
- Tunnel profile/health/logは`tunnel-client`がrepository外へ保存。
- どちらもcommitしない。
- 通常診断でtool argument全面dumpやsecret-bearing config dumpを有効にしない。

## 9. Upgrade gate

`mcp-proxy` / `tower-mcp` / FastMCP interoperability / protocolを変更するとき:

1. source/release notes/config schema review
2. `Cargo.lock` を意図的にのみ更新
3. fmt/clippy/unit/mock/fault tests
4. 6 real backend test
5. aggregate integration smoke
6. Secure Tunnel READY/MCP probe
7. Swibo lifecycle

8. public repository hygiene review

## 10. OAuthログイン不可のAI向けPAT

ブラウザOAuth認証ができない外部AIには、既存OAuthを維持したまま、
**個別に失効できるPAT**を発行します。

```powershell
# Windows実機で実行。秘密値はクリップボードへコピーし、コンソールには出しません。
pwsh -NoProfile -File scripts/pat.ps1 -Action issue -Label external-ai -Days 30 -Tools 'lomway_search_tools,lomway_describe_tool'
# Deferred呼び出しはwrapperと対象ツール名の両方が許可必須です。
pwsh -NoProfile -File scripts/pat.ps1 -Action issue -Label browser-ai -Days 30 -Tools 'lomway_call_tool,chrome_devtools_list_pages'
pwsh -NoProfile -File scripts/pat.ps1 -Action list
pwsh -NoProfile -File scripts/pat.ps1 -Action revoke -Id '<発行時のID>'
```

PATの秘密値は相手AIの**認証情報専用入力欄**にのみ登録し、
チャット・コマンド引数・共有ログには貼り付けないでください。
audienceは `https://mcp.maiteneru.com/mcp`、scopeは `devspace`、
許可ツールは完全一致、期限上限は90日です。Git管理外の
`%LOCALAPPDATA%/Lomway/oauth-sidecar/runtime/pat-tokens.json`
には秘密値でなくSHA-256照合値と管理情報だけを保存します。
失効は次のintrospection対象リクエストから有効で、
実行中の処理の強制中断ではありません。PATはrefreshできず、
期限前に必要なら再発行します。

PAT対応Gatewayの本番反映は、Windows端末でリポジトリから
`pwsh -NoProfile -File scripts/deploy-gateway-isolated.ps1 -PreflightOnly`
で事前検証後、
`pwsh -NoProfile -File scripts/deploy-gateway-isolated.ps1` を実行します。
`scripts/deploy-pat-gateway.ps1` も同じ安全な実装に委譲します。
事前に独立したWorkbridge管理経路（7680）とGateway/OAuth/公開Issuerの健全性を検証します。
Gatewayだけを短時間停止して、`runtime/bin/lomway-gateway-<sha256>.exe` という
独立した不変のバイナリへ切り替えます。デスクトップ用17778と共有している
`target/release/lomway.exe` は上書きせず、デスクトップ用プロセスも停止しません。
起動先は `runtime/gateway-binary-path.txt` で指定し、失敗時は元の起動先に戻して
Gatewayを再起動します。結果は `runtime/pat-deployment.json` に保存します。
**`phase=success` とPAT権限制御のlive検証を確認するまでPATは発行しないでください。**

検証用PATは `pwsh -NoProfile -File scripts/test-pat-live.ps1`
と `pwsh -NoProfile -File scripts/test-pat-live.ps1 -Public` で確認できます。
このスクリプトは1日有効・読み取り専用ツール1件だけの一時PATをプロセス内で発行し、
実際のMCP initialize／初期化完了通知／ツール一覧と呼び出し／拒否・失効を検証し、
最後に必ず失効します。秘密値は出力・クリップボードへ転送しません。

全ツール許可PATは `pwsh -NoProfile -File scripts/pat.ps1 -Action issue -Label hark -Days 7 -AllTools`
で発行できます。通常のMCPツール（Deferred経由を含む）すべてにアクセスできますが、
内部管理ツール（`proxy/config`、`proxy/add_backend`等）は許可しません。
`-AllTools` は将来追加される通常ツールにも適用される強い権限です。
発行値はチャットやログへ出力せず、WindowsクリップボードからHarkの
認証情報専用入力欄にのみ登録してください。利用終了後はPAT IDを指定して失効させます。
固定の権限が必要な場合は引き続き `-Tools` で個別の許可リストを設定します。

外部AI実機の追加試験には `scripts/test-pat-codex-client.ps1` を使用します。
Codex CLIに1ツールだけの一時PATを子プロセスの環境変数で渡し、実際の
AIツール呼び出しを確認します。期限内にCodexから結果が来なかった場合も
子プロセスを終了し、PATを必ず失効させます。
検証前にCodex自身の最小モデル応答を確認し、応答がない場合はPATを発行しません。

## 11. OAuth sidecar cutover（offline準備済み・live gateあり）

Lomway-owned Authorization sidecarはloopback-onlyで `127.0.0.1:7677` へ直接bindし、
lifecycleはSwiboがsuperviseします。
同じ公開issuerに対してsidecarを二重起動しません。
同じissuer向けforwarderも定常構成では起動しません。

OAuth lifetime policy:

- Access Tokenは短命（通常1時間）。
- Refresh Tokenは30日で、refresh時にrotationします。
- Grantも初期30日とし、Refresh Tokenが新規発行またはrotationされるたび、そのRefresh Tokenのexpiryまで自動延長します。
- 新規Refresh TokenはブラウザSession期限に紐づけません。旧Tokenについては有効なSessionとGrantを検証できた場合だけ初回使用時に移行します。
- したがって利用が継続しているclientはslidingで維持され、30日以上利用されないclientは自然失効します。
- 既にGrantが失効しRefresh Tokenも削除済みのclientは自動復活させず、1回だけ再認証します。

認証の安全対策:

- 公開DCR POSTはSidecarプロセス全体で60秒20回までとし、永続Client登録数は
  128件までに制限します。未承認・未参照の新規Clientは48時間経過後の次の登録で
  整理します。作成時刻の不明な旧Clientは推測で消去しません。
- 新規OAuthクライアントは `lomway_search_tools` と
  `lomway_describe_tool` のみ許可します。既存の承認済みClientを維持する
  ため、Gateway更新前に `scripts/provision-legacy-oauth-clients.ps1` を
  `-VerifyOnly`、次に通常モードで一度だけ実行します。
  `client-tool-policy.json` はGit外の所有者専用ランタイムにSHA-256の
  Client IDだけを保存します。新規Clientを自動的に旧Client扱いにはしません。
- 旧Sidecar実装を `runtime/oauth-rollback/server.pre-004.js` に保存してから
  `scripts/deploy-oauth-csp.ps1 -Hardening` でOAuthのみ再起動します。
  その後、独立したWorkbridge経路を保持したまま
  `scripts/deploy-gateway-isolated.ps1` で不変のGatewayバイナリへ切り替えます。
- Gatewayは認証主体のハッシュ・メソッド・ツール・許可/拒否を構造化監査します。
  トークン本文、ツール引数、平文client_idは記録しません。
- 期限切れの一時的レコードを5分の猶予後に清掃します。refresh再利用検知に
  必要なGrant、RefreshToken、失効マーカーは削除しません。
  失効・期限切れPATは次のPAT発行時に整理します。
- interactionを確認してから非同期でパスワードを検証し、uidに依存しない所有者単位の試行回数制限を適用します。
- ログインと同意を分離し、client ID・未検証の名称・redirect URI・scopeを表示して明示承認を求めます。
- OAuthのPOST後にChromiumが登録済みredirect URIへの移動を遮断しないよう、
  CSPのform-actionにはselfとOIDCで検証されたredirect URIのオリジンだけを
  許可します。合成資格情報・独立したstateで実ブラウザの認可コード交換まで行う
  `python scripts/test-oauth-browser.py` を検証に使用してください。
- AccessToken/RefreshToken/AuthorizationCodeのBearer IDをハッシュ化して保存し、旧形式のstateは初回読み込み時に移行します。
- 起動時にOAuthランタイムディレクトリとファイルのACLを所有者・SYSTEM・Administratorsのみに制限します。
- 起動時に行うのは既存ACLの検査だけです。ACLの初期設定は
  `scripts/private-oauth-state-acl.ps1` をWindowsの所有者権限で別途実行します。
  Swiboの起動処理からSet-Aclを再実行しません。
- 内部用 `/oauth/introspect` はCloudflare経由の公開リクエストを拒否し、公開版provider introspectionは無効化します。
- 並行refreshの再利用を検知するとGrant系列を失効させ、その後の保存でも復活できないようにします。

cutover前はsynthetic stateだけでsidecarを検証し、公開issuerとendpoint mappingが安定して
いることを確認します。repositoryのoffline fmt、clippy、test、build、diff gateも実行し、
owner credential、token、state、SOPS materialはrepository外に置きます。

後日のlive cutover手順:

1. Workbridgeのauthority routeをstop/disableします。必要ならWorkbridgeのMCP backend routeは残します。
2. Swibo経由でsidecarをloopback listenerに起動し、metadata、CIMD/DCR registration、PKCE S256、
   resource-bound token、refresh rotation、revocation、loopback introspectionを確認します。
3. 公開issuerを変更せず、実際の `/auth`、`/token`、`/reg`、
   `/token/revocation` をsidecarへrouteします。
   内部用 `/oauth/introspect` は公開トンネル側でも遮断します。
4. Lomwayがmissing/invalid credentialにprotected-resource challenge付き`401`、required scope不足の
   有効tokenに`insufficient_scope`付き`403`を返すことを確認します。
5. 非Workbridge backendへの認証済みcallを確認し、その後real clientのrefreshとTunnelをlive検証します。
   これらのlive gateはrepository test suiteでは実施しません。

rollbackもauthorityは1つだけです。旧Workbridge routeを先に戻してからrestartし、同じissuerで両方を動かしません。
