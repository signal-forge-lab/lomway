# Operations

更新日: 2026-09-13

## 1. Ownership

6つのbackend MCPはそれぞれ独立したSwibo targetのままです。`Lomway` は1つのSwibo targetとして、内部lifecycleで次の順序を管理します。

```text
start: Gateway -> Secure MCP Tunnel
stop:  Secure MCP Tunnel -> Gateway
```

Gatewayがbackend MCP processをstart/stopすることはありません。

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
  backend services -> Gateway -> Secure MCP Tunnel

Stop aggregate target only:
  Secure MCP Tunnel -> Gateway
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
