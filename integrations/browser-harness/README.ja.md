# Browser Harness MCP の Lomway 統合（Windows）
`browser-harness[mcp]` を Python 3.12 の uv tool として導入し、公式 `browser-harness-mcp` の stdio 通信を `mcp-proxy` でローカル Streamable HTTP に変換します。

## 運用
- 起動: `powershell -NoProfile -File integrations/browser-harness/start.ps1`
- 停止: `powershell -NoProfile -File integrations/browser-harness/stop.ps1`
- 常駐管理: Swibo ターゲット `browser-harness-mcp`（起動ポリシー auto）
- Lomway: バックエンド `browser-harness`、接頭辞 `bh_`、`http://127.0.0.1:17685/mcp`、deferred 公開
- 接続検証: `scripts/browser-harness-mcp-smoke.py`

Chrome 144以降の実ブラウザ操作には `chrome://inspect/#remote-debugging` の許可設定と初回の接続承認が必要です。ブラウザ録画は無効のまま維持し、ポート17685はループバックのみで待ち受けます。既存BrowserSkillの優先方針は維持します。
