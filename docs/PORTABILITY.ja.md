# Core portability audit

公開Lomway Coreはplatform-neutralなRust実装です。監査対象は`src/`と`Cargo.toml`で、PowerShellやworkstation固有integrationはCore外の`scripts/`と`integrations/`に分離します。

## 確認結果

- `src/`には`cfg(windows)`、`cmd.exe`、PowerShell、`.exe`、`USERPROFILE`、`APPDATA`、`LOCALAPPDATA`への必須依存がありません。
- config pathは`std::path::{Path, PathBuf}`とcheckout/CWD相対defaultを使い、Windows固有separatorやuser-home layoutを要求しません。
- runtime networkはTokio/Axumとpin済みMCP libraryによるloopback TCP/HTTPです。backend process lifecycleはCore外です。
- `serve`、`check`、`list-backends`、`migrate`、`version`はRust binary単体で動作し、shellを要求しません。
- Windows / Linux / macOSは同一のlocked fmt/clippy/test/release-build CI gateを実行します。本監査完了後は3 platformすべてrelease-blockingです。

Windows PowerShell wrapperは任意のconvenience/integration surfaceであり、Core runtime dependencyではありません。Windows clean-machine artifact smokeは`scripts/clean-machine-smoke.ps1`で別途検証します。
