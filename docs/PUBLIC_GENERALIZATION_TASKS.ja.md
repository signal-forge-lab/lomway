# Lomway — 公開・汎用化タスク分解

Status: READY FOR MULTI-AI EXECUTION

機械可読な割当・進捗管理には `PUBLIC_GENERALIZATION_TASKS.yaml` を使用します。このMarkdownは判断理由と人間向け詳細の正本です。

この backlog は、会話履歴を持たない別AIでも作業できるように、依存関係、変更境界、受け入れ条件を明示します。

## 共通実行ルール

1. 現行 production behavior を baseline として保持。
2. private/local config、secret、tunnel ID、workspace ID を公開ファイルへ書かない。
3. backend process lifecycle を Core に持ち込まない。
4. OpenAI/Swibo/SOPS を Core の必須依存にしない。
5. mutation request を retry しない。
6. 変更単位ごとに fmt/clippy/test を実行。
7. unrelated refactor を混ぜない。
8. 既存 six-backend real regression を壊さない。

## 並列 lane

- Lane A — Config/Core model
- Lane B — Backend registry/namespace
- Lane C — Host/router/health
- Lane D — CLI/platform independence
- Lane E — Optional integrations
- Lane F — Test/release/docs

Lane A の public config model 確定後、B/C/D/E は大部分を並列化可能です。

## G0 — Baseline freeze

### LMG-G0-01 Baseline evidence

- fmt/clippy/unit/mock/real-backend baseline を記録。
- current tool/namespace counts を regression evidence として記録。
- public/private operational files の境界確認。

Acceptance: baseline failure 0、secret 値を artifact に含めない。

### LMG-G0-02 Branch/worktree policy

generalization 用 branch/worktree と rollback point を文書化。

Acceptance: stable rollback point が一意。

## G1 — Config schema

### LMG-G1-01 Public config structs

Depends: G0-01

`schema_version`, `server`, `policy`, `Vec<BackendConfig>`, `{id,prefix,url,required}` を実装。

Acceptance: 0/1/N parse、unknown critical key reject、serde tests。

### LMG-G1-02 Validation decomposition

Depends: G1-01

listener/backend URL/id/prefix/max-size/unsupported-feature validator を分離。

Acceptance: current `validate_policy` の安全性後退なし、focused unit tests。

### LMG-G1-03 Legacy migration adapter

Depends: G1-01

current local config -> public schema の一時 compatibility path。

Acceptance: private data を公開せず現環境を移行可能。

## G2 — Backend registry

### LMG-G2-01 Backend descriptor

Depends: G1-01

validated config からのみ runtime descriptor を生成。Swibo/OpenAI/SOPS type 禁止。

### LMG-G2-02 Registry 0..N

Depends: G2-01

arbitrary backend collection、deterministic ordering、lookup by id、required/optional classification。

Acceptance: hard-coded six backend name 0。

### LMG-G2-03 Startup probe semantics

Depends: G2-02

required down -> fail、optional down -> degraded/skipped、all down -> default fail。

## G3 — Namespace/collision

### LMG-G3-01 Prefix policy

Depends: G1-02

allowed character/reserved-prefix rules を確定。unsafe ambiguity は reject。

### LMG-G3-02 Tool collision preflight

Depends: G2-02, G3-01

serve 前に最終 public tool 名を計算。

Acceptance: collision 時に両 source を示して deterministic fail-fast。

### LMG-G3-03 Schema preservation regression

Depends: G3-02

representative description/input schema が namespacing 後も意味的に不変。

## G4 — Gateway host

### LMG-G4-01 Gateway builder extraction

Depends: G2-02, G3-02

`src/lib.rs` から mcp-proxy composition を `gateway/build.rs` へ分離。

Acceptance: upstream `proxy` control backend は引き続き除去。

### LMG-G4-02 Router surface

Depends: G4-01

公開 route は `/mcp`, `/healthz`, `/readyz` のみ。

Acceptance: `/admin/*`, `/mcp/admin/*` inaccessible、unknown route 404。

### LMG-G4-03 Health state

Depends: G2-03, G4-02

liveness/readiness を分離し required/optional semantics を反映。

## G5 — CLI/platform independence

### LMG-G5-01 CLI

Depends: G1-01

`serve --config`, `check --config`, `list-backends --config`, `version`。

Acceptance: actionable exit codes、binary execution に PowerShell 不要。

### LMG-G5-02 Config search policy

Depends: G5-01

明示的・deterministic な config path precedence。private path を Core default にしない。

### LMG-G5-03 Cross-platform path audit

Depends: G5-01

Core に required Windows-only path/process behavior がないことを確認。

## G6 — Optional integrations

### LMG-G6-01 OpenAI Secure MCP Tunnel

Depends: G5-01

tunnel scripts/docs を optional integration 化。

Acceptance: Core test/install に OpenAI credential/client 不要、現環境は維持。

### LMG-G6-02 Swibo

Depends: G5-01

machine-specific absolute path のない target template を提供。

Acceptance: Core は Swibo code に依存しない。

### LMG-G6-03 SOPS

Depends: G5-01

SOPS workflow を optional helper 化。

Acceptance: public Core は ordinary env/config のみでも動作、secret values 0。

### LMG-G6-04 PowerShell helpers

Depends: G5-01

PowerShell は Core CLI の convenience wrapper のみにする。

## G7 — Portable tests

### LMG-G7-01 Public mock fixtures

Depends: G2-03, G4-02

1/N backend、optional/required failure、collision、timeout を private service なしで再現。

### LMG-G7-02 Exactly-once mutation regression

Depends: G7-01

forced timeout/error でも mutation counter exactly 1。

### LMG-G7-03 Portable real-backend test

Depends: G5-01

redistributable/public fixture server を最低2種類。private credential 不要。

### LMG-G7-04 Private deployment regression

Depends: G1-03, G6-01..03

current six namespaces と OpenAI tunnel/Swibo 運用を維持。destructive migration は別承認なしに行わない。

## G8 — Security/supply chain

### LMG-G8-01 Threat model

untrusted metadata/schema、oversized args、SSRF、non-loopback exposure、credential leakage、namespace spoofing、retry/replay mutation risk を code/test mitigation に対応付ける。

### LMG-G8-02 Secret/privacy scan

公開候補に username path、email、live tunnel/workspace ID、API/private keys がないこと。

### LMG-G8-03 Dependency/license/vulnerability

exact lockfile を scan。未解決 high/critical は release block。

## G9 — Packaging/CI

### LMG-G9-01 Windows artifact

clean Windows で binary + example config だけから `check`/`serve` PASS。

### LMG-G9-02 CI matrix

locked fmt/clippy/test/build。Core portability 確認後 Linux/macOS を追加。

### LMG-G9-03 Release metadata

version/changelog/checksums/license/source archive/example configs。

## G10 — Documentation

### LMG-G10-01 Quickstart

Swibo/OpenAI/SOPS なしで2 arbitrary backends を設定し `/mcp` まで到達。

### LMG-G10-02 Integration guides

OpenAI Secure MCP Tunnel / Swibo / SOPS / Windows helpers を別 guide にする。

### LMG-G10-03 Migration/rollback guide

current deployment -> generic config と rollback を文書化。

## G11 — Final release gate

### LMG-G11-01 Full review

PASS 条件:

- functional regression 0。
- portable clean-machine setup PASS。
- current private deployment regression PASS。
- security/privacy gate PASS。
- docs と実 CLI/config が一致。
- Core に mandatory integration 0。
- intended release tree が clean。

## Dependency summary

```text
G0
 -> G1
    -> G2 -> G3 -> G4
    -> G5
       -> G6
       -> G9
G2/G4 -> G7
G4 -> G8
G6/G7/G8/G9 -> G10 -> G11
```

## AI handoff template

```text
Task ID:
Status: PASS | BLOCKED | PARTIAL
Files changed:
Behavior changed:
Tests run + result:
Security/privacy impact:
Compatibility impact:
Known follow-ups:
Do not infer / unresolved decisions:
```

次のAIは会話履歴ではなく repository state とこの handoff を正本として継続します。

