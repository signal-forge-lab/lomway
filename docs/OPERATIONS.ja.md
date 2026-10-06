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

## 10. OAuth sidecar cutover（offline準備済み・live gateあり）

Lomway-owned Authorization sidecarはloopback-onlyで `127.0.0.1:7677` へ直接bindし、
lifecycleはSwiboがsuperviseします。
同じ公開issuerに対してsidecarを二重起動しません。
同じissuer向けforwarderも定常構成では起動しません。

OAuth lifetime policy:

- Access Tokenは短命（通常1時間）。
- Refresh Tokenは30日で、refresh時にrotationします。
- Grantも初期30日とし、Refresh Tokenが新規発行またはrotationされるたび、そのRefresh Tokenのexpiryまで自動延長します。
- したがって利用が継続しているclientはslidingで維持され、30日以上利用されないclientは自然失効します。
- 既にGrantが失効しRefresh Tokenも削除済みのclientは自動復活させず、1回だけ再認証します。

cutover前はsynthetic stateだけでsidecarを検証し、公開issuerとendpoint mappingが安定して
いることを確認します。repositoryのoffline fmt、clippy、test、build、diff gateも実行し、
owner credential、token、state、SOPS materialはrepository外に置きます。

後日のlive cutover手順:

1. Workbridgeのauthority routeをstop/disableします。必要ならWorkbridgeのMCP backend routeは残します。
2. Swibo経由でsidecarをloopback listenerに起動し、metadata、CIMD/DCR registration、PKCE S256、
   resource-bound token、refresh rotation、revocation、loopback introspectionを確認します。
3. 公開issuerを変更せず、公開 `/authorize`、`/token`、`/register`、`/revoke` をsidecarへrouteします。
4. Lomwayがmissing/invalid credentialにprotected-resource challenge付き`401`、required scope不足の
   有効tokenに`insufficient_scope`付き`403`を返すことを確認します。
5. 非Workbridge backendへの認証済みcallを確認し、その後real clientのrefreshとTunnelをlive検証します。
   これらのlive gateはrepository test suiteでは実施しません。

rollbackもauthorityは1つだけです。旧Workbridge routeを先に戻してからrestartし、同じissuerで両方を動かしません。
