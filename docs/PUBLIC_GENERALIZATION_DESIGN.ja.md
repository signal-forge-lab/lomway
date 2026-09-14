# Lomway — 公開・汎用化設計

Status: DESIGN BASELINE

この文書は、Lomway（旧 Local MCP Gateway）を壊さずに、任意の MCP backend を束ねられる公開配布向け製品へ一般化するための正本設計です。既存の `LMG-G*` タスクIDは追跡性のため保持します。

## 1. 目的

公開版 Lomway は、特定のユーザー環境、特定の6 backend、Swibo、SOPS、OpenAI Secure MCP Tunnel に依存せず、設定だけで 0..N 個の MCP backend を集約できることを目的とします。

公開版の最小価値は次です。

- 任意の HTTP Streamable MCP backend を 0..N 個登録できる。
- backend ごとに明示的な namespace prefix を持てる。
- tool 名衝突を起動前に検出し、曖昧な状態では起動しない。
- northbound MCP endpoint は1つだけ公開する。
- Lomway Core 単体で動作し、OpenAI/Swibo/SOPS は任意 integration とする。
- 安全側の default を維持する。

## 2. 非目標

公開 v1 では次を Core の責務にしません。

- backend process の起動・停止。
- OpenAI Secure MCP Tunnel の必須化。
- Swibo の必須化。
- SOPS の必須化。
- backend 間の failover / mirror / canary / fan-out。
- mutation tool の自動 retry。
- tool schema の意味的書き換え。
- remote internet-facing listener の既定有効化。
- dynamic hot reload。
- generic `call_tool(name, args)` 1個への縮退。

## 3. アーキテクチャ境界

```text
MCP Client
   |
   v
Lomway Host
   |
   +-- Core Config
   +-- Backend Registry
   +-- Namespace / Collision Guard
   +-- MCP Aggregation Router
   +-- Health / Readiness
   |
   +--> Backend A
   +--> Backend B
   +--> Backend C ...

Optional integrations
   +-- OpenAI Secure MCP Tunnel
   +-- Swibo
   +-- SOPS
   +-- PowerShell helpers
```

Core は integration が1つも存在しなくても起動できなければなりません。

## 4. 推奨モジュール境界

現在 `src/lib.rs` に集中している責務を、最終的に次へ分離します。

```text
src/
  lib.rs
  config/{mod.rs,model.rs,load.rs,validate.rs}
  backend/{mod.rs,registry.rs,descriptor.rs,probe.rs}
  namespace/{mod.rs,normalize.rs,collision.rs}
  gateway/{mod.rs,build.rs,router.rs,policy.rs}
  health/{mod.rs,state.rs}
  cli/{mod.rs,args.rs}
  main.rs

integrations/
  openai-secure-tunnel/
  swibo/
  sops/
  powershell/
```

`integrations/` は Core crate のコンパイル必須依存にしないことを原則とします。

## 5. 公開設定モデル

公開版では backend 固有名を一切 hard-code しません。

```toml
[server]
host = "127.0.0.1"
port = 17777

[policy]
allow_non_loopback_backends = false
allow_non_loopback_listener = false
hot_reload = false
max_argument_size_bytes = 1048576

[[backends]]
id = "filesystem"
prefix = "fs_"
url = "http://127.0.0.1:8001/mcp"
required = true

[[backends]]
id = "browser"
prefix = "browser_"
url = "http://127.0.0.1:8002/mcp"
required = false
```

必須規則:

- `backend.id` は一意。
- `prefix` は公開 v1 では明示指定かつ一意。
- 空 prefix は公開 v1 では禁止。
- 最終 public tool 名は全 backend 横断で一意。
- listener/backend URL は既定で exact loopback のみ。
- `required=true` は unavailable 時に startup failure。
- optional backend は unavailable 時に degraded/skipped を許可。
- v1 では backend 復旧後の自動再 discovery は行わず、gateway restart で再 discovery。

## 6. Namespace と tool identity

公開版でも tool schema を generic tool に潰さず、backend の tool を直接公開します。

```text
upstream tool: navigate
prefix: browser_
public tool: browser_navigate
```

必須 invariant:

1. backend id は一意。
2. prefix は一意。
3. prefix + upstream tool name の最終名は全 backend 横断で一意。
4. reserved prefix (`proxy_`, `lmg_` など) は policy で管理する。
5. upstream tool description と input schema は原則保存する。
6. framework-private metadata を互換性目的で意味的に書き換えない。

## 7. Failure model

startup:

- config parse error -> fail closed。
- policy violation -> fail closed。
- duplicate backend id/prefix -> fail closed。
- resulting tool collision -> fail closed。
- required backend unavailable -> fail closed。
- optional backend unavailable -> degraded 起動可。
- usable backend 0件 -> 明示 empty mode がない限り fail closed。

runtime:

- mutation call は自動 retry しない。
- read call も Core では retry しない。
- backend timeout/error は該当 tool call の error として返す。
- backend recovery の自動再 discovery は v1 では行わない。

## 8. Health model

- `/healthz`: Lomway process/router の liveness。
- `/readyz`: config valid かつ startup policy を満たし northbound MCP を提供可能。
- HTTP admin surface は Core v1 では追加しない。

## 9. Security baseline

- listener: `127.0.0.1` only by default。
- backend URL: `127.0.0.1` only by default。
- auth forwarding: off。
- secret のログ出力禁止。
- admin HTTP endpoint: none。
- hot reload: off。
- retry/hedging/coalescing: off。
- argument size upper bound: 1 MiB default。

非 loopback を将来許可する場合は、別 security profile と threat model を要求します。

## 10. Integration 境界

### OpenAI Secure MCP Tunnel

Core は tunnel の存在を知らない。integration は Lomway `/mcp` を target にし、credential/profile/runtime state は repository 外で管理します。

### Swibo

Core は Swibo API/registry に依存しない。integration は start/stop/status と、必要なら tunnel との ordered lifecycle を構成します。

### SOPS

Core に SOPS dependency を持ち込まない。SOPS は optional helper とし、Core は通常の env/config だけでも動作可能にします。

### PowerShell

Windows helper として残してよいが、Core binary 自体は PowerShell を必要としない状態を目標にします。

## 11. CLI 契約

```text
lomway serve --config <path>
lomway check --config <path>
lomway list-backends --config <path>
lomway version
```

`check` は process を起動せず parse + policy + collision preflight を行います。

## 12. Compatibility policy

- config schema に `schema_version` を導入。
- unknown top-level keys は typo 検出のため原則 reject。
- minor release では既存 field の意味を変更しない。
- breaking config change は major/schema version を上げる。
- current operational config には migration path を残す。

## 13. Testing contract

Unit:

- config parse/defaults。
- loopback validation。
- duplicate id/prefix。
- namespace/reserved prefix/collision。
- required/optional backend semantics。

Mock E2E:

- 1 backend / N backend / 0 backend。
- optional down / required down。
- identical upstream names with different prefix。
- collision after composition。
- mutation call exactly once on timeout/error path。
- `/admin/*` not exposed。

Real backend:

- public fixture MCP を最低2種類使う portable integration suite。
- 現在の six-backend suite は machine-local regression suite として維持。

Release gate:

- fmt / clippy `-D warnings` / locked build+test。
- secret/privacy scan。
- license/dependency vulnerability review。
- clean example config from scratch。

## 14. Distribution

公開 v1 の優先順:

1. GitHub source release。
2. Windows x86_64 prebuilt binary。
3. checksums。
4. example configs。
5. optional integration docs。

Linux/macOS CI は Core が PowerShell 非依存になった後に追加します。

## 15. 現行環境

現在の6 backend 構成は削除せず、公開 generic schema の reference deployment/regression target として維持します。

公開化の完了条件は、現在の環境を維持しながら、別ユーザーが private 固有情報なしで任意 backend を設定して起動できることです。

