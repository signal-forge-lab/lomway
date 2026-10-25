# Host-local secret consumers / ホスト内での秘密情報利用

## Scope / 対象

The canonical encrypted source remains the external SOPS store.
`scripts/Invoke-HostSops.ps1` owns **status / generation / rotation** and is unchanged.
`scripts/Invoke-HostSecretConsumer.ps1` is a separate, **read-only** preflight
boundary for explicitly registered consumers. No secret value is decrypted,
copied, printed or delivered by this interface.

正本は既存のSOPS暗号化ファイルです。既存の管理スクリプトは変更しません。
新しいスクリプトは**許可済み利用先の事前確認だけ**を行います。秘密情報の復号・入力・配送は実装していません。

> This document is a security/design review and operator guide. It does **not**
> define new requirements: `spec.md` / `plan.md` / `tasks.md` remain the Spec Kit
> authority, and code/tests are the evidence for what is implemented.
> 本書は設計・セキュリティ評価と運用手順であり、第二の要件正本ではありません。

## Registered consumer / 登録済みの利用先

| Consumer | Fixed SOPS key | Fixed endpoint | Viewer guard |
|---|---|---|---|
| `MacVncLogin` | `MAC_MINI_ADMIN_PASSWORD` | `100.100.47.83:5900` | TigerVNC executable path, one exact `VNC authentication` window, one established connection to the exact endpoint |

Only `Status` and `Plan` are supported:

```powershell
pwsh -NoProfile -File scripts/Invoke-HostSecretConsumer.ps1 -Action Status -Consumer MacVncLogin
pwsh -NoProfile -File scripts/Invoke-HostSecretConsumer.ps1 -Action Plan -Consumer MacVncLogin
```

Both return sanitized JSON only. Unknown parameters, unknown actions and unknown
consumers are rejected by PowerShell parameter binding.

`Status` と `Plan` だけを提供し、返すのは秘匿情報を含まないJSONのみです。
未知のパラメーター・アクション・ConsumerはPowerShellのパラメーターバインディングで拒否されます。

| Field / フィールド | Meaning / 意味 | Not proven / 証明しないもの |
|---|---|---|
| `present` | 暗号化JSONに同名のトップレベルキーが存在する | 復号可否、暗号文の真正性、鍵の正しさ |
| `viewer_ready` | 一致するviewerプロセスと一致する確立済みTCP接続が各1件観測できた | 接続先が意図したMacであること、サーバー認証、パスワードの正しさ、ログイン成功 |
| `destination` | ソースに固定された宛先文字列 | 現在の接続がその宛先であること |
| `secret_exposed` / `credential_delivery` | 常に `false` / `not-implemented` | — |

## Trust boundary / 信頼境界

- **No caller authentication or authorization exists inside the script.**
  Anything that can start `pwsh.exe` as this Windows user can run it, and can
  read the same SOPS store directly. The script only limits the *intended*
  public interface; it is not an OS security boundary.
  スクリプト内に呼び出し元の認証・認可はありません。このWindowsユーザーとして
  `pwsh.exe` を起動できる主体は誰でも実行でき、SOPSストアも直接読めます。
  このスクリプトが制限するのは「想定された公開インターフェース」だけで、OSの
  セキュリティ境界ではありません。
- Access today rests on (a) Windows user identity and file ACLs on the SOPS
  store and age key, and (b) whatever authorization the outer MCP client /
  coding-tools layer applies. **That outer MCP exposure is not verified here**
  and must be treated as unknown until checked separately.
  現在の防御は、WindowsユーザーとSOPSストア・age鍵のACL、(b) 外側のMCPクライアント/
  coding-tools層の認可に依存します。**外側のMCP公開設定は本書では未検証**であり、
  別途確認するまで不明として扱ってください。
- A caller with arbitrary code execution as the same Windows user cannot be
  isolated by a PowerShell script: it can decrypt the store, read process
  memory of an authenticating client, or inject input itself. No adapter can
  fix that; only a separate identity (different user/service account) with
  OS-level ACLs and an authorization step can.
  同一Windowsユーザーで任意コード実行ができる呼び出し元は、PowerShellスクリプトでは
  隔離できません(ストア復号、認証中クライアントのメモリ参照、入力の直接注入が可能)。
  これを解消できるのは、別ユーザー/サービスアカウントとOSのACL、明示的な認可だけで、
  どのAdapterでも埋められません。
- `viewer_ready=true` is an **observation hint**, not authentication: it neither
  proves who is on the remote side nor that a human authorized the login.
  `viewer_ready=true` は**観測ヒント**であり認証ではありません。接続相手が誰かも、
  本人がログインを承認したことも証明しません。

### Findings, risk-ranked / 危険度順の指摘

| # | Risk / 危険度 | Finding / 指摘 | Status / 状況 |
|---|---|---|---|
| P0 | Highest / 最高 | **A prohibited behaviour, not an existing defect.** Allowing automatic credential input into a GUI (the previously refused behaviour) would defeat the viewer guard and hand an admin password to whatever sits on that endpoint. / **既存の脆弱性ではなく、禁止している行為の危険度。** GUIへの自動資格情報入力(過去に安全拒否された挙動)を許せば、viewerガードが無効化され、その宛先に居る何かに管理者パスワードを渡してしまう。 | Blocked by policy; must not be implemented via another AI, helper script, GUI driver or alternate transport. / 方針として禁止。別AI・helperスクリプト・GUIドライバ・別transportでの迂回も禁止。 |
| P1 | High / 高 | No caller identity/authorization in the consumer script; same-user code execution has equal access to the SOPS store. / Consumerスクリプトに呼び出し元の識別・認可がない。同一ユーザーの任意コード実行は同等にストアへ到達できる。 | Documented; unresolved by design of a per-user script. / 記載済み。per-userスクリプトの性質上未解決。 |
| P1 | High / 高 | Actual credential delivery does not exist, and `viewer_ready` proves neither endpoint identity, server authentication nor login success. / 実際の資格情報配送は未実装で、`viewer_ready` は宛先識別・サーバー認証・ログイン成功のいずれも証明しない。 | Blocked pending a reviewed adapter design (see below). / レビュー済みAdapter設計までblocked(下記)。 |
| P2 | Medium / 中 | `Status`/`Plan` read the encrypted JSON top-level keys without proving ciphertext authenticity, so a file swapped by a same-user process is reported as `present`. / 暗号文の真正性を検証せずトップレベルキーだけを読むため、同一ユーザーのプロセスが差し替えたファイルでも `present` と表示される。 | Accepted limitation of read-only preflight. / read-only事前確認の許容済み制約。 |
| P2 | Medium / 中 | MCP exposure of the preflight is not verified in this repository. / 事前確認のMCP公開範囲は本リポジトリでは未検証。 | Remaining operator check. / 運用側の残確認。 |
| P3 | Low / 低 | `present` reflects a key name only, so a renamed or stale key looks identical to a valid rotation. / `present` はキー名だけを見るため、改名・古いキーも有効なローテーションと区別できない。 | Accepted; rotation stays with `Invoke-HostSops.ps1`. / 許容。ローテーションは既存スクリプトの責務。 |

## Security contract / セキュリティ契約

- No caller-supplied key, executable, command, destination, hostname,
  environment-variable name or script. Adding one is a security change.
  呼び出し元が指定できるキー・実行ファイル・コマンド・宛先・ホスト名・環境変数名・
  スクリプトはありません。追加すること自体がセキュリティ上の変更です。
- No plaintext return through MCP, logs, stdout/stderr, process arguments or
  clipboard. Failures **inside the script's own `try` block** print only
  `HOST_SECRET_CONSUMER_PREFLIGHT_FAILED` and exit non-zero. A parameter
  binding failure (unknown action, consumer or parameter) is rejected by
  PowerShell **before** that block, so it prints PowerShell's own error text
  instead; it never includes secret material, because no store read happens.
  平文はMCP・ログ・標準出力/標準エラー・引数・クリップボードに返しません。
  **スクリプト自身の `try` ブロック内**の失敗は `HOST_SECRET_CONSUMER_PREFLIGHT_FAILED`
  のみを出力して非0で終了します。パラメーターバインディング失敗(未知のアクション・
  Consumer・パラメーター)はそのブロックより前にPowerShellが拒否するため、PowerShell
  自身のエラー文が出ますが、ストア読取りも行わないため秘密情報は含まれません。
- No unattended retry loops and no guessed credentials: an authentication
  attempt must be a single human-initiated action. Repeated attempts risk
  account lockout on the target Mac.
  無人リトライループと資格情報の推測は禁止です。認証試行は本人が開始する1回だけにし、
  反復は対象Macのアカウントロックアウトを招きます。
- A submitted request is not a verified authentication; only the human can
  confirm success on the target machine.
  依頼の投入は認証成立ではありません。成功の確認は本人が対象機で行います。
- Consumers must be reviewed and registered explicitly in source.
  Consumerはレビュー済みで、ソースに明示登録されている必要があります。
- Future consumers: only fixed, source-reviewed entries may be added. An
  approved delivery adapter is a design condition, not implemented, and must
  satisfy all of the following before any delivery:
  将来のConsumerは固定・レビュー済みのエントリのみ追加できます。配送Adapterは
  設計条件であり未実装で、配送前に次をすべて満たす必要があります。
  1. separate identity / OS ACLs instead of the interactive user; / 対話ユーザーではなく別アイデンティティとOSのACL
  2. an explicit human authorization step per attempt; / 試行ごとの明示的な本人認可
  3. one-shot retrieval with no caching, retries or logging of the value; / 使い捨て取得。キャッシュ・リトライ・値のログ出力なし
  4. secret-free results (only success/failure and non-secret metadata); / 結果は秘密情報を含まない(成否と非秘匿メタデータのみ)
  5. no arbitrary command, executable, hostname or environment injection; / 任意のコマンド・実行ファイル・ホスト名・環境変数の注入禁止
  6. targeted tests with synthetic fixtures only. / 合成fixtureのみを使った対象限定テスト

### Design conditions only / 設計条件のみ(未実装)

No **delivery dispatcher** (the `@{ ... }[$Consumer]` profile table stays the
only registration point), no delivery adapter, no transport abstraction and no
authentication automation exists or is proposed for this change. If a
compliant adapter cannot be verified, the result is: design + safe preflight +
human-authorized handoff, nothing more.
**配送用のdispatcher**(登録は `@{ ... }[$Consumer]` のprofile表のみ)・配送Adapter・
transport抽象化・認証の自動化は存在せず、今回の変更でも提案していません。
要件を満たすAdapterを検証できない場合の成果物は「設計・安全な事前確認・
本人認可による手動引き継ぎ」までです。

## Operator handoff / 運用手順(本人が行う)

1. Run `-Action Plan -Consumer MacVncLogin` and confirm `present=true` and
   `viewer_ready=true`.
   `-Action Plan -Consumer MacVncLogin` を実行し、`present=true` と
   `viewer_ready=true` を確認します。
2. In the trusted TigerVNC window, confirm the destination is the intended
   Mac (`100.100.47.83:5900`); `viewer_ready` alone does not prove it.
   信頼できるTigerVNCのウィンドウで、宛先が意図したMac (`100.100.47.83:5900`)
   であることを確認します。`viewer_ready` だけでは証明になりません。
3. Type the password yourself in that window. Never give the credential to a
   Bot, CLI, MCP tool, script argument or clipboard, and never ask an agent to
   look it up or type it.
   パスワードは本人がそのウィンドウで入力します。資格情報をBot・CLI・MCPツール・
   スクリプト引数・クリップボードに渡さず、エージェントに取得や入力を依頼しません。
4. On failure, **stop**. Do not retry automatically, do not guess variants and
   do not ask an agent to retry; a lockout is worse than a delayed login.
   失敗したら**停止**します。自動再試行・候補の推測・エージェントへの再試行依頼は
   行いません。ロックアウトはログイン遅延より悪い結果です。
5. Confirm success yourself, then continue any separate Mac/iPhone/WDA
   authorization flow under its own approval.
   成功は本人が確認し、Mac/iPhone/WDAの認可が必要なら別途その認可手続きに従います。

## Status / 状況

| Item / 項目 | State / 状況 |
|---|---|
| Read-only `Status` / `Plan` preflight / read-only事前確認 | implemented / 実装済み |
| Sanitized failure output, unchanged store on failure / 秘匿化した失敗出力・失敗時にストア不変 | implemented / 実装済み |
| Rejection of unknown parameters / consumers / actions / 未知のパラメーター・Consumer・アクションの拒否 | implemented / 実装済み |
| Viewer and endpoint guard decisions (`$ready` for 0/1/多件) / viewer・endpointガードの判定(0/1/複数件の`$ready`) | implemented and verified / 実装・検証済み |
| Caller authentication / authorization / 呼び出し元の認証・認可 | blocked (no separate identity) / blocked(別アイデンティティなし) |
| Credential delivery to the VNC client / VNCクライアントへの資格情報配送 | blocked (design conditions above) / blocked(上記設計条件) |
| Automatic credential input, GUI driving, retry loops / 自動入力・GUI操作・リトライ | blocked by policy / 方針としてblocked |
| MCP exposure of the preflight / 事前確認のMCP公開範囲 | unverified (outside this repository) / 未検証(本リポジトリ外) |
| Mac / iPhone / WDA authorization / Mac・iPhone・WDAの認可 | blocked pending separate authorization / 別認可待ちでblocked |

## Secret hygiene / 秘密情報の扱い

Tests use disposable synthetic SOPS/age fixtures under a temporary directory
and remove them afterwards. No real credential is used in development tests.
The read-only consumer script never writes to the SOPS store; only the test
fixture setup substitutes `USERPROFILE` in the child process.
テストは一時ディレクトリの合成SOPS/age fixtureだけを使い、終了時に削除します。
開発テストに実資格情報は使いません。read-onlyのConsumerスクリプトはSOPSストアへ
書き込みません(`USERPROFILE` の差し替えはtest fixtureの子プロセス内のみ)。
