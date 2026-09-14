# Final Review

Review date: 2026-09-14（独立再review。元のcertification evidenceは2026-09-13）

Verdict: **PASS — public generalization / Final Review / 初回public push complete (100%)**

## Public generalization recovery — 現在の正本結果

LMG public-generalization backlogは **35/35 done、todo 0、blocked 0** です。この節が以降のpre-generalization時点の古い件数より優先されます。

- `cargo fmt --check --all` — PASS
- `cargo clippy --all-targets --locked -- -D warnings` — PASS
- `cargo test --locked` — 45 lib + 5 fixtures + 11 policy + 13 proxy = **74 PASS**。machine-localなreal-backend testはdefaultでは意図的にignoredです。削除された5 unit testは未使用だった `HealthState` / `HealthTracker` snapshot実装専用で、live readinessはproxy E2Eで引き続き検証されています。
- `cargo test --locked --test real_backends -- --ignored --nocapture` — **1/1 PASS**。baseline 6 namespace（Workbridge / Memory Gateway / Microsoft UFO / Stealth Browser / XMind Workboard / Praxiom）はすべて利用可能です。追加のChrome namespaceは許容し、6-backend baselineの必須条件にはしません。
- schema preservation、required/optional startup failure、collision、timeout、mutation error-path exactly-once regressionはすべてPASSです。
- `python scripts/release_privacy_scan.py` — PASS
- `python scripts/dependency_gate.py` — 309 exact locked packages、license metadata missing 0、unresolved OSV 0でPASS
- `pwsh -NoProfile -File scripts/clean-machine-smoke.ps1` — private deployment backendなしでcopied release binary / copied public fixture / public mock backendを使い、`check` / `/healthz` / `/readyz` PASS
- 2026-09-14 live aggregate smoke — **213 tools**、`proxy_* = 0`、代表call **7/7 PASS**。元の6 namespaceをregression baselineとして維持し、review済みworkstationには任意追加のChrome DevTools 30 toolsがあります。
- Secure Tunnel — Lomway runtime aliasは正確に1件だけREADY、local health/readiness 200、`tunnel-client` processも1本だけです。
- `git diff --check` — PASS（line-ending warningのみ）
- public main履歴privacy — push-readiness reviewでPASS。current treeはcleanでしたが、pre-publicの旧main commitにmachine-specific pathが残っていたため、review済みclean treeからfresh public rootとして`main`を再構築しました。現在の`main`履歴は公開対象だけを含み、禁止generated/private path・privacy findingとも0です。旧履歴はheadではないlocal backup refにのみ退避し、push対象外です。

Core portabilityは`PORTABILITY.md` / `PORTABILITY.ja.md`に記録し、Windows / Linux / macOSをCIでrelease-blockingにしました。PowerShell、SOPS、OpenAI、Swiboは任意integrationのままでCore dependencyではありません。

publish / push / tagは実施していません。元のcertification後、ユーザーが明示承認した運用migrationとして、旧個別Secure MCP Tunnelと旧local runtime aliasを削除し、残すremote tunnelを **Lomway** へrenameしました。現在のremote ingressはLomway Secure MCP Tunnel 1本だけです。このmigrationはworkstation runtime stateを変更しますが、public Core contractやrelease evidenceは変更しません。

### 独立再reviewでの修正 — 2026-09-14

orchestratorのcompletion flagをそのまま採用せず再reviewし、以下のdocumentation/release-record不一致を修正しました。

- public README / Architectureで製品を固定6 backend / 183 toolsとして扱わず、public v1は0..N、6-backend inventoryはregression baselineであることを明示。
- northbound HTTP surfaceを列挙する全対象箇所へ `/readyz` を反映。
- release checklistへ `scripts/dependency_gate.py` を明示追加し、CI / supply-chain policyと一致。
- Testingを実数のpolicy 11、proxy E2E/fault 13、current live smoke 213 tools / 7 callsへ同期。
- Tunnel運用記録を完了済みsingle-Tunnel migrationへ同期。
- live Windows release buildの競合を発見して手順修正。実行中 `target/release/lomway.exe` はWindows上で上書きできないため、release checklistをfresh staging `CARGO_TARGET_DIR` 推奨へ変更し、live serviceを維持したままisolated release build PASSを確認。
- push-readiness reviewで、current treeがcleanでもpre-public `main`履歴にmachine-local path文字列が残る問題を発見。`main`をreview済みclean treeからfresh public rootへ再構築し、repeatable privacy gateを`HEAD`到達可能な全履歴blobまでscanするよう強化。
- source全体のarchitecture reviewで残存していた構造重複も解消。`src/lib.rs`を薄いpublic facadeとし、proxy構築を`gateway/build.rs`、backend再接続を`gateway/reconnect.rs`、唯一のHTTP/readiness surfaceを`gateway/router.rs`へ分離しました。未使用のsnapshot型health subsystemは削除し、既にwiredだったlive backend-health semanticsへ一本化しています。internal API/data surfaceもruntimeで実際に必要な範囲へ縮小しました。

### Push readiness — 2026-09-14

初回public pushは完了です。公開branchは`main`のみで、履歴privacyはclean、local `main` は `https://github.com/signal-forge-lab/lomway` の `origin/main` をtrackしています。今後もpush対象は **mainのみ**とし、local tooling refは非公開のため `--all` / `--mirror` は使用しません。

---

## Historical pre-generalization review evidence

以下は以前のgateway integration reviewの履歴証拠です。件数が現在値と異なる場合は、上記public-generalization recovery結果を正本とします。

## 1. Review scope

今回のFinal Review対象:

- architecture / responsibility boundary
- Rust host / production policy
- 6 real MCP backend integration
- aggregationに必要だったMicrosoft UFO / Stealth Browser互換変更
- XMind upstream capability drift対応とtask verification強化
- OpenAI Secure MCP Tunnel setup/runtime
- Swibo lifecycle integration
- unit / mock E2E / fault / real backend tests
- release build / script parse
- secret/privacy/local-path hygiene
- dependency/advisory/license review
- 英語/日本語public docs

Google Driveは意図どおり独立維持です。既存の個別ChatGPT connector削除はuser migration stateを変える別作業なので実施していません。

## 2. Architecture review

### Cohesion — PASS

Gateway責務はconnection aggregation / namespace routing / timeout / startup policy validation / `/healthz` / structured logging / northbound surface制限だけです。

backend process、domain logic、LLM routing、schema translation、retry/cache/failover、GUI、Google Driveは所有しません。

### Coupling — PASS

backendとの境界はloopback MCP/HTTPです。aggregationによるbackend codeへのcompile-time couplingはありません。

aggregation互換に必要な変更だけowner projectへ保持しました。

- UFO / Stealth Browser: FastMCP公式 `FASTMCP_INCLUDE_FASTMCP_META=false` でprivate `_fastmcp` metadataを無効化。
- XMind Workboard: official `xmind_get_topic` がtask read-backを返すようになったため、task mutation verificationをsemantic化。

### Simplicity / over-engineering — PASS

generic dispatcher、reconciliation loop、custom MCP protocol、dynamic router、application admin API、speculative search-mode layerは追加していません。不要だったadmin plane/secretとhot reloadを削除したため、初期設計より最終構成の方が小さくなっています。

## 3. Security review

### Network / authority — PASS

- listenerは `127.0.0.1` のみ。
- southboundはexact `http://127.0.0.1:<numeric-port>/mcp` のみ。
- remote ingressはOpenAI Secure MCP Tunnelだけ。
- upstream `proxy` MCP管理backendをserve前に削除。
- top-level `/admin/*` をrouteしない。
- nested `/mcp/admin/*` はupstream dispatch前に拒否。
- admin planeが存在しないため、不要な `security.admin_token` 指定もproduction policyでreject。
- clientからbackend登録/能力rewrite不可。

### Side-effect safety — PASS

retry / hedging / cache / mirror / canary / failover / composite / request coalescingはproduction policyでreject。forced timeout mutationのbackend受信は正確に1回でした。

### Secrets — PASS

Gateway runtime自体はsecret-freeです。Secure Tunnel setup/runtimeだけが正本SOPS storeから `OPENAI_ADMIN_KEY` / `CONTROL_PLANE_API_KEY` をprocess environmentへ読み込みます。plaintext valueはrepositoryへ保存せず、literal command-line valueとして渡しません。

### Supply chain — PASS

- `mcp-proxy` 0.4.3 exact pin
- `Cargo.lock` tracked + `--locked`
- resolved `tower-mcp` 0.18.2
- lockfile package数 309
- 309 exact name/versionをOSV querybatchで照合し、2026-09-13時点 **vulnerability record 0件**
- 309 packageすべてlicense metadataあり、missing 0件
- current RustSec spot-checkもlockと整合。例: `anyhow` 1.0.104はRUSTSEC-2026-0190 fix以降、`event-listener` 5.4.2はRUSTSEC-2026-0221修正版。

このreviewだけのためにglobal `cargo-audit` / `cargo-deny` を新規installせず、exact lockfileをOSV + Cargo metadataで直接確認しました。

## 4. Reliability review

### Failure isolation — PASS

- startup構築時に1 backend失敗しても、他backend成功ならskip可能。
- healthy namespace継続利用。
- initial constructionで全backend失敗ならGateway fail closed。
- skip backend復旧はGateway restartでrediscoveryし、新reconciliation subsystemは追加しない。

### Runtime ownership — PASS

SwiboがGateway/Tunnel lifecycleを所有し、backendは独立targetのままです。

```text
READY -> restart -> READY -> stop -> STOPPED -> start -> READY
```

最終release rebuild/restart後:

```text
Lomway: READY
server: READY
tunnel: READY
error: empty
```

### PID/process identity — PASS

PID fileのprocessがexpected release executableか確認してからrunning扱いします。startupではprocess exitを `/healthz` より先に確認するため、別processが同じportで200を返してもfalse READYになりません。

## 5. Test / release evidence

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| policy tests | 10/10 PASS |
| mock proxy/fault E2E | 5/5 PASS |
| `cargo build --release --locked` | PASS |
| PowerShell全script parser validation | PASS |
| local production config check | PASS、6 backends |
| explicit real backend test | PASS |
| aggregate tools | 183 |
| `proxy_*` | 0 |
| representative aggregate calls | 6/6 PASS |
| Gateway `/healthz` | READY |
| OpenAI Secure MCP Tunnel | READY、MCP initialize成功 |

Real catalog:

```text
Workbridge       12
Memory Gateway   32
Microsoft UFO    19
Stealth Browser  97
XMind Workboard  21
Praxiom           2
Total           183
```

Owner-project evidence:

- UFO: Python compile PASS、HTTP MCP READY。
- Stealth Browser: Python compile PASS、section inventory PASS、gateway discovery 97 tools。
- XMind Workboard: 14 test files / 72 tests PASS、typecheck/build PASS。
- Swibo: Node 38/38、Tauri 2/2、clippy `-D warnings` PASS。

## 6. Public repository hygiene

tracked/staged fileだけでなく、ignoreされていない実装候補もpublic gate対象にしています。

local config / runtime / logs / build outputはignore済み。final scanではreal username、user固有absolute Windows path、live tunnel ID、setup時workspace identifier、personal email pattern、OpenAI key pattern、private-key PEM blockを確認します。

最終scan結果は**全category 0 findings**です。今回remote pushは行っていません。

## 7. Review中に解決した主なfindings

- stale/invalid `tool_exposure` valueをupstream `direct`へ修正。
- UFOが元々Tunnel-owned stdioだったため、stdio互換を壊さず独立HTTP lifecycleを追加。
- FastMCP `_fastmcp` incompatibilityを公式source-side settingで解決。
- 全backend down時はupstream proxyが構築不能である実挙動にfailure model修正。
- upstream standalone `/` と設計 `/mcp` の差をhost-owned pathで解消。
- nested admin routeは競合route追加では遮断できずmiddleware拒否へ修正。
- project policy reload問題からhot reload削除。
- 不要authorityだったadmin plane/tokenを完全削除。
- broad `*.lock` で除外されていた `Cargo.lock` をtracking対象へ修正。
- Windows path spaceによる`Start-Process` config引数分割を修正。
- backend URL validationをprefix/suffixからexact loopback port + `/mcp` へ強化。
- stale/reused PIDとunrelated-port false positiveを防ぐprocess identity/readiness検証を追加。
- XMind schema driftをreviewしtask semantic verificationを強化。

## 8. Remaining work

Blocking implementation/review work: **none**。

それぞれ別の明示的migration/release actionなので未実施です。初回public push自体は完了しています:

- 既存個別ChatGPT connectorのdisable/delete
- Google Drive変更
- 初回public push後のrelease tag / release artifact公開

これらは任意の後続migration/release actionであり、Gateway実装の未完了ではありません。

## 9. Final verdict

**PASS — requested design / implementation / tests / operational integration / single-Tunnel migration state / security & hygiene review / Final Review / 初回public push scope を100%完了。**
