# Testing

更新日: 2026-09-14

## 1. Release test layers

### Static / compiler gate

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

### Production-policy tests

11 policy testsでloopback-only、HTTP local backend限定、stable namespace、hot reload拒否、direct exposure、auth-forwarding/schema rewrite禁止、retry/hedge/cache禁止、fan-out/failover/coalescing禁止、public-release hygiene、argument-size境界を検証します。

### Mock MCP E2E

13 E2E/fault testsでcontrol-plane抑止、HTTP surface制限、namespace/schema保持、startup failure/collision、runtime recovery/readiness、session failure recovery、mutation exactly-onceを検証します。代表case:

- control-plane `proxy` backend removal
- `/mcp` + `/healthz` + `/readyz` と `/admin/*` 404
- 同名tool namespace collision
- startup failed backend isolation
- timeoutしたmutationのbackend受信回数が正確に1回

### Portable fixture regression

`tests/fixtures.rs` は、private deploymentなしのクリーンなmachineでregression suiteが再現できることを証明します。`test-fixtures/configs/` 配下の再配布可能なpublic設定fixtureがzero/one/manyのbackend populationをcoverします:

- `zero-backends.toml` — public schemaでは空のbackend listが有効。一方でdeployment gateは最低1 backendを要求し続ける（安全性の後退なし）
- `one-backend.toml` — required backend 1件
- `many-backends.toml` — required + optionalのmix。到達不能なoptional backendはstartupを失敗させずdegrade

coverage: 3 populationすべてに対する厳格なpublic schemaのload/validate、保存された「最低1 backend」deployment gate込みのpublic↔legacy migration、再配布可能なinventoryの固定scan（private service名/credential/machine pathを含まない）、one/many fixtureでの実gateway E2E（mock MCP backendに対する名前空間化 `tools/list` とtool call、到達不能なoptional backendのdegrade-on-outage含む）。使い方と保証は `test-fixtures/README.md` を参照してください。

### Real backend compatibility

`tests/real_backends.rs` はportableなdefault suiteではignoredにし、設定済みworkstationで明示実行します。defaultの `cargo test --locked` では `1 ignored` として報告され、private deploymentがなくてもCIがgreenを保てるのはこのためです。`tests/fixtures.rs` がこの `--ignored` gatingとmachine-local configのbindingをguardします。検証結果:

```text
workbridge 12
memory     32
ufo        19
browser    97
xmind      21
praxiom     2
```

6 backendすべてが同じpinned `tower-mcp` HTTP client stackでinitialize/tools list成功しています。

### Aggregate integration smoke

2026-09-14の再reviewで `scripts/integration-smoke.ps1` は **213 tools**、`proxy_tool_count = 0`、代表call **7/7 PASS** を確認しました。元のbaseline 6 namespaceはprivate regressionの必須条件として維持し、追加のChrome DevTools 30 toolsは許容され、review済みworkstationに存在します。

### Secure Tunnel

OpenAI Secure MCP Tunnel native runtimeがREADYであること、MCP probeがGateway initialize成功することを必須にします。検証済みruntimeでは、upstream `protocol-2026-07-28` supportでbuildしたGatewayとMCP `2025-11-25` をnegotiationしました。

これはsecure tunnelからaggregate MCP endpointまでの経路検証です。ChatGPT UI上でconnectorを追加/削除する操作はuser migration actionなので自動testでは行いません。

### Swibo

live target lifecycle:

```text
READY -> restart -> READY -> stop -> STOPPED -> start -> READY
```

加えてSwibo自身のregression gateもgreenであることを確認します。

## 2. Dependent backend regression

aggregation互換のために変更した箇所はowner project側でも確認します。

- UFO wrapper: Python compile + HTTP lifecycle/status
- Stealth Browser: Python compile/section inventory + HTTP tool discovery
- XMind Workboard: capability drift review後のfull unit/typecheck/build
- Swibo: live target登録後のNode/Tauri regression gate

## 3. Required negative coverage

- non-loopback gateway listener
- non-loopback/non-HTTP backend URL
- wrong namespace separator
- `hot_reload = true`
- search/discovery exposure
- retry/hedging/cache/fan-out/failover/coalescing
- schema/argument/visibility rewrite
- upstream control-plane MCP tool exposure
- top-level/nested admin HTTP path access
- startup backend failure
- backend timeoutでmutation replayなし

v1はadmin plane自体を公開せずadmin tokenも不要なので、「missing/wrong gateway admin token」testは存在しません。

## 4. Current release evidence

- Rust fmt: PASS
- Rust clippy `-D warnings`: PASS
- lib: 45/45 PASS
- fixture: 5/5 PASS
- policy: 11/11 PASS
- proxy E2E/fault: 13/13 PASS
- real backend integration: 1/1 PASS、6 backend inventory確認
- real backend observation: baseline 6 namespaceを維持し、追加Chrome toolsはbaseline必須にはしません
- isolated release artifact build + clean-machine smoke: PASS
- 2026-09-14独立release-build再確認: live Windows Lomway processを動かしたままfresh staging `CARGO_TARGET_DIR` へのrelease buildがPASS。実行中 `target/release/lomway.exe` の直接上書き失敗はWindows file lockによるものでcompile failureではないことを確認
- XMind Workboard: 14 files / 72 tests PASS、typecheck/build PASS
- Swibo: Node 38/38、Tauri 2/2、clippy PASS
- Secure Tunnel runtime/MCP probe: PASS

最終releaseにはpublic repository secret/privacy/local-path scanとfinal code/security reviewも必須です。現在は以下でrepeatableに実行できます。

```powershell
python scripts/release_privacy_scan.py
python scripts/dependency_gate.py
pwsh -NoProfile -File scripts/clean-machine-smoke.ps1
```
