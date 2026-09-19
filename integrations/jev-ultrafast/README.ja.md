# Jev Ultrafast MCP backend

このオプション統合は、独立管理されるJev Ultrafast MCP serverをLomwayへ登録します。
Lomwayはbackendのbrowser操作やresultを起動・停止・retry・cache・変換しません。

English: [README.md](README.md)

## Backend

`signal-forge-lab/jev-ultrafast` の `feature/lomway-mcp-recovery` branchで実行します。

```powershell
uv sync
$env:JEV_MCP_HOST = "127.0.0.1"
$env:JEV_MCP_PORT = "18766"
uv run jev-mcp
```

常時運用では既存の外部supervisorを使用します。Lomwayへprocess lifecycle管理を追加しません。

backendが公開するtoolは次の6個だけです。

- `jev_browser_start`
- `jev_browser_step`
- `jev_browser_run`
- `jev_browser_resume_text`
- `jev_browser_inspect`
- `jev_browser_close`

追跡対象の例は [`config/jev-ultrafast.example.toml`](../../config/jev-ultrafast.example.toml) です。
Lomwayの `jev_` namespaceにより、例えばbackendの `jev_browser_start` は集約後に
`jev_jev_browser_start` として公開されます。Lomwayはbackend schema/resultを書き換えません。

## model設定

通常のJev判断には `TYPESAFE_API_KEY` を使います。internal text modeは `TEXT_MODEL_*`、
Recoveryはprovider-neutralな `RECOVERY_MODEL_*` を使い、未設定時は対応する
`TEXT_MODEL_*` へfallbackします。実値は既存の外部SOPS/supervisor経路から注入し、
Lomway/Jev repositoryへ書きません。

## 検証

backend起動中に次を実行します。

```powershell
cargo run -- check --config config/jev-ultrafast.example.toml --probe
cargo run -- serve --config config/jev-ultrafast.example.toml
```

`http://127.0.0.1:17777/mcp` に接続したMCP clientから、namespace付きの6 toolが見えることを
確認します。Jev停止時にLomwayはmutating callをretryしません。backendをoptionalとしているため、
既存のmixed healthy/unhealthy startup policyも変わりません。
