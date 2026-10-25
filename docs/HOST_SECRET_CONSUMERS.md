# Host-local secret consumers / ホスト内での秘密情報利用

## Scope / 対象

The canonical encrypted source remains the external SOPS store.
`scripts/Invoke-HostSops.ps1` owns **status/generation/rotation** and is unchanged.
`scripts/Invoke-HostSecretConsumer.ps1` is a separate, **read-only**
preflight boundary for explicitly registered consumers. No secret value is
decrypted, copied, printed or delivered by this new interface.

正本は既存のSOPS暗号化ファイルです。既存の管理スクリプトは変更しません。
新しいスクリプトは**許可済み利用先の事前確認だけ**を行います。
秘密情報の復号・入力は実装していません。

## Registered consumer / 登録済みの利用先

| Consumer | Fixed SOPS key | Fixed endpoint | Viewer guard |
|---|---|---|---|
| `MacVncLogin` | `MAC_MINI_ADMIN_PASSWORD` | `100.100.47.83:5900` | TigerVNC executable, one exact authentication dialog, one established connection |

Only `Status` and `Plan` are supported:

```powershell
pwsh -NoProfile -File scripts/Invoke-HostSecretConsumer.ps1 -Action Status -Consumer MacVncLogin
pwsh -NoProfile -File scripts/Invoke-HostSecretConsumer.ps1 -Action Plan -Consumer MacVncLogin
```

Both return sanitized JSON only. `viewer_ready=true` means the network and
window guards matched. It **does not** mean login succeeded, and it is not
permission to retrieve the password.

## Security contract / セキュリティ契約

- No caller-supplied key, executable, command, destination or script.
- No plaintext return through MCP, logs, process arguments or clipboard.
- Consumers must be reviewed and explicitly registered in source code.
- No generic secret-to-shell command, no arbitrary executable injection.
- A real credential-delivery adapter, if approved and safely implementable,
  must be a separate locally trusted component with targeted tests, no
  automatic retries, and explicit success/error reporting.
- A PowerShell script in the user's account is **not** an OS security boundary:
  an independently authorized shell running as the same user may have broader
  SOPS permissions. This contract limits the intended public interface only.

実際のMacのロック解除とWDA再認可には、別途認証操作が必要です。
現時点では自動ログイン機能が完成したとは扱いません。
