# 移行とロールバックガイド

Updated: 2026-09-13

既存のlegacyデプロイ構成（`mcp-proxy` の `[proxy]` schema）を公開gateway schema（`schema_version = 1`）へ移行する手順と、そのロールバック方法です。どの時点でも破壊的なステップはありません。

English: [MIGRATION.md](MIGRATION.md)

## 1. 2つの構成フォーマット

| | legacyデプロイschema | 公開schema |
|---|---|---|
| 識別 | トップレベルの `[proxy]` テーブル | トップレベルの `schema_version = 1` |
| backend | `[[backends]]` の `name` / `transport` / `url` / `[backends.timeout]` | `[[backends]]` の `id` / `prefix` / `url` / `required` / `timeout_seconds` |
| population規則 | 1つ以上のbackend（デプロイゲート。pin済みupstreamローダーと一致） | 0 / 1 / N backend |
| required/optional | 暗黙: 到達不能なbackendは起動をdegrade | backendごとに明示。`required = true` は到達不能時に起動失敗 |
| 検出 | ロード時に自動 | ロード時に自動 |

両フォーマットとも第一級です: `serve` / `check` / `list-backends` はディスク上のフォーマットを自動判定し、どちらを使ったかを報告し、legacyファイルは **メモリ内で** 移行します。いかなるコマンドもlegacyファイル自体を変更しません。

## 2. 公開schemaへの1回限りの移行

```powershell
lomway migrate --config config/proxy.local.toml --output gateway.public.toml
```

保証（すべて強制され、ユニットテストで検証済み）:

- **上書きしません。** 入力は決して変更されず、出力先に既存ファイルがあれば拒否します（上書きしません）。再実行するには出力ファイルを明示的に削除してください。移行はmutationであり、mutationは自動retryも強制もされません。
- **`${VAR}` 参照は解決済みの値として焼き込まず、そのまま書き出します。** 解決済みの形は書き込み前にend to endで検証され、2回目の解決パスで同一URLにならなければ書き込みを拒否します。
- **degrade-on-outageの挙動は保持されます**: 移行されたbackendはすべて `required = false` で書き出され、到達不能なbackendはlegacyデプロイと同様に起動をdegradeします。fail closedな起動を明示的に望む場合だけ `required = true` に上げてください。
- **移行は純粋なメモリ内変換です。** 非公開の値が公開ファイルに入ることはありません。出力には構成内容だけが含まれます（マシンローカルのendpoint一覧は従来どおりgitignore対象のファイルに置きます）。

## 3. フィールド対応

| legacy | 公開 | 備考 |
|---|---|---|
| `proxy.listen.host` / `proxy.listen.port` | `server.host` / `server.port` | loopback-only policyは不変 |
| `proxy.instructions` | `server.instructions` | そのまま引き継ぎ |
| `proxy.shutdown_timeout_seconds` | `server.shutdown_timeout_seconds` | そのまま引き継ぎ |
| `proxy.separator` | `_` 固定 | 他のseparatorは移行を拒否 |
| `security.max_argument_size` | `policy.max_argument_size_bytes` | 既定1 MiB |
| `backends[].name` | `backends[].id` と `prefix = "<name>_"` | prefixがnamespace契約 |
| `backends[].timeout.seconds` | `backends[].timeout_seconds` | 既定30秒 |
| `backends[].transport` | HTTPのみ | 非HTTP backendは移行を拒否 |
| `backends[].url` | `backends[].url` | `http://127.0.0.1:<port>/mcp` のみ |
| — | `backends[].required` | 公開schemaの新フィールド。移行時は `false` |
| — | `policy.allow_non_loopback_listener` / `allow_non_loopback_backends` / `hot_reload` | 常に `false`。escape hatchはfail closed |

legacy専用のmiddlewareフィールド（retry / hedging / cache / mirror / canary / aliases / 引数注入 / expose・hideフィルタ / auth転送）は意図的に **引き継ぎません**: 公開schemaに対応フィールドはなく、gateway policyは起動時にこれらを拒否します。

## 4. 移行後の構成を検証

```powershell
lomway check --config gateway.public.toml --probe
lomway list-backends --config gateway.public.toml
lomway serve --config gateway.public.toml
```

`check --probe` は各backendに正確に1回接触し、最終tool名を検証します。レポートにはロードされたフォーマットも表示されます。serveされるtool catalogはlegacyと同一のはずです（`<name>_<tool>` 名、descriptionとinput schemaは無変更）。

## 5. ロールバック

移行はadditiveなので、ロールバックは単純です:

1. **構成のロールバック** — 元のlegacyファイルは一切変更されていません。公開ファイルでのserveをやめてlegacyファイルから再開してください（または `gateway.public.toml` を削除するだけ）。legacyフォーマットのサポートは恒久であり、廃止予定の互換shimではありません。
2. **フォーマットの挙動** — legacyファイルを見つけたコマンドはそのまま動作します。legacyパスに留まるのにflagも変換も不要です。
3. **ソースのロールバック** — 汎用化前のbaselineは `main` のroot commitとして保存されています（LMG-G0ロールバックポイント）。そのcommitからバイナリをrebuildすれば、記録されたbaselineの挙動を再現できます。worktree/ロールバックpolicyの全文（reset・discard・cleanが許されない範囲を含む）は [PUBLIC_GENERALIZATION_ROLLBACK.ja.md](PUBLIC_GENERALIZATION_ROLLBACK.ja.md)（[en](PUBLIC_GENERALIZATION_ROLLBACK.md)）に規定されています。
4. **オプション統合は無影響** — OpenAI Secure MCP Tunnel・Swibo・SOPS・PowerShellラッパー統合はcoreの外部にあり、移行は不要です。

## 6. 安全性の要約

- 移行・ロールバックのどのステップもbackendプロセスに触れません。backendは独立して監督されており、期間中ずっと稼働し続けます。
- いかなるコマンドも自動retryしません。失敗した移行は実行可能なerrorで終了し、原因を修正してから手動で再実行します。
- デプロイゲート（legacy policy）は1つ以上のbackendを要求し続けます。zero-backend構成を受け入れるのは公開schemaだけです。
