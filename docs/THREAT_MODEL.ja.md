# Lomway 脅威モデル

Status: ACTIVE (LMG-G8-01)
Updated: 2026-09-14

本文書は公開版 Lomway v1 の正式な脅威モデルである。ゲートウェイの 2 つの構造的
セキュリティ特性、すなわち**ループバック分離**と**認証なしローカル境界**を対象とする。
すべての脅威には、少なくとも 1 つの実施済み緩和策を、具体的な実施箇所と
リグレッション証拠とともに対応付ける。

関連文書: `SECURITY.ja.md`(運用上のセキュリティ概要)、`ARCHITECTURE.ja.md`
(システム構成)、`PUBLIC_GENERALIZATION_DESIGN.md`(公開 v1 設計基準)。

## 1. 対象範囲

対象:

- `lomway` バイナリ(設定解析、検証、集約ルーター、ヘルス);
- ループバックリスナーで公開される北向き HTTP サーフェス;
- 設定済み MCP バックエンドへの南向き HTTP 接続;
- セキュリティ姿勢を拡大・縮小しうる入力としての設定ファイル。

対象外(所有者は他、§9 参照):

- バックエンドプロセスのセキュリティとライフサイクル(外部スーパーバイザー);
- リモート侵入経路の認証(OpenAI Secure MCP Tunnel 統合);
- シークレット保管(外部 SOPS ストア);
- OS・ファイアウォールおよびその他のローカルソフトウェア;
- ChatGPT や任意の MCP クライアント側のセキュリティ。

## 2. 資産

| ID | 資産 | 機密度 |
|---|---|---|
| A-01 | ループバック MCP エンドポイント(`/mcp`) | 設定済みバックエンドに対する任意のツール実行 |
| A-02 | バックエンドツールカタログとスキーマ | どのツールが存在するかの露出。書き換えられてはならない |
| A-03 | 設定ファイル | 到達可能なエンドポイントとポリシー上限を定義 |
| A-04 | ゲートウェイログ | シークレット値を含んではならない |
| A-05 | ゲートウェイ可用性 | ローカル単一テナントサービス。外部 SLA なし |

## 3. 信頼境界とアクター

```text
                        信頼されない
  MCP クライアント(ChatGPT または任意のローカル MCP クライアント)
      |  (認証は Secure MCP Tunnel 統合が所有。ゲートウェイは所有しない)
      v
====== ループバックリスナー 127.0.0.1:<port> ======================
      |  認証なしローカル境界(受容、T-03 参照)
      v
  Lomway ゲートウェイ(本リポジトリ)
      |  (ループバック HTTP、認証情報なし、厳密な /mcp パス)
      v
====== ループバック ==============================================
      v
  設定済み MCP バックエンド(信頼されたローカルサービス)
```

- **B-1 北向き:** マシン上の任意のプロセスがリスナーへ TCP 接続できる。
  ゲートウェイはこの境界で**認証を行わない**。
- **B-2 南向き:** ゲートウェイはバックエンドに対して**認証を行わず**、
  認証情報を転送しない。
- **B-3 設定:** 設定ファイルを書ける人間/オペレーターはバックエンド選択を
  信頼される。ただし設定によってリスナーまたはバックエンドのネットワーク
  スコープを拡大することはできない(T-01/T-02/T-09 参照)。

## 4. 前提

1. マシン自体は攻撃者に掌握されていない(敵対的なローカル管理者や
   カーネルレベル攻撃者は対象外)。
2. 設定ファイルに記載されたバックエンドは、オペレーターが選択した信頼済み
   ローカルサービスである。
3. リモートアクセスが必要な場合は、ゲートウェイより手前で認証を終端する
   統合(例: OpenAI Secure MCP Tunnel)経由でのみ到達する。
4. 本書でいうループバックは厳密に `127.0.0.1`(IPv4)を意味する。
   リンクローカル・LAN・未指定アドレスは非ループバックとして扱う。

## 5. 脅威と緩和策の対応表

STRIDE 分類: S なりすまし、T 改ざん、R 否認、I 情報漏えい、D サービス拒否、
E 権限昇格。

| ID | STRIDE | 脅威 | 緩和策(実施箇所) | 証拠 |
|---|---|---|---|---|
| T-01 | I, E | リスナーが非ループバックアドレス(LAN/`0.0.0.0`)にバインドされ、認証なしツールサーフェスがリモートから到達可能になる。 | `server.host` は厳密に `127.0.0.1` でなければならない(`src/config/validate.rs` `validate_listener`);`policy.allow_non_loopback_listener = true` は本リリースでは明示的に拒否される;既定リッスンホストは `127.0.0.1`(`src/config/model.rs` `DEFAULT_LISTEN_HOST`)。 | `tests/policy.rs` `rejects_non_loopback_listener`;`src/config/validate.rs` `rejects_non_loopback_listener_and_backends` |
| T-02 | I, E | バックエンド URL がマシン外を指し、ゲートウェイがツール引数を意図しないリモートホストへ中継する踏み台になる。 | loopback HTTP が既定。remote southbound は `policy.allow_non_loopback_backends = true` の明示opt-inが必要で、その場合も既定TLS portの厳密な `https://<machine>.<tailnet>.ts.net/mcp` のみ許可する。任意Internet host、非TLS、userinfo、query、fragment、custom port、別pathは拒否(`src/config/validate.rs`)。 | `src/config/validate.rs` `tailnet_https_backends_require_explicit_policy_and_stay_narrow`;`src/backend/registry.rs` `explicit_policy_allows_tailnet_https_backend` |
| T-03 | S, E, I | 別のローカルプロセス(または別 OS ユーザーセッション)が認証なし北向き境界に到達し、ツールを呼び出す。 | **受容済みの残存リスク、設計により制約:** リスナーはループバックのみ(T-01)のため、境界は構成上マシンローカルに限定される。HTTP サーフェスは `POST/GET/DELETE /mcp`、`GET /healthz`、`GET /readyz` に最小化(`src/gateway/router.rs`)。管理プレーンもメトリクスもない。管理トークンは提供されないため設定も不要。マルチテナント強化(ローカル認証)は v1 の対象外であり、新たなセキュリティプロファイルを要する。 | `tests/proxy.rs` `external_http_surface_exposes_mcp_and_health_but_not_admin`;`src/gateway/policy.rs` の北向き認証拒否;`docs/SECURITY.ja.md` §3 |
| T-04 | S, E | リモート攻撃者が認証なしで正規クライアントになりすます。 | ゲートウェイは構成上リモートから到達不能(T-01);リモート侵入はゲートウェイ手前で認証を終端する統合経由のみ;ゲートウェイ自身による北向き MCP 認証や認証情報転送の設定は拒否される(`src/gateway/policy.rs` が空でない認証設定を失敗させる)。 | `tests/policy.rs` `rejects_admin_token_when_admin_plane_is_not_served`、`rejects_schema_rewriting_and_auth_forwarding` |
| T-05 | E | 上流 `mcp-proxy` の管理プレーン(`proxy` MCP バックエンドの設定/登録ツール、`/admin/*` ルート)がクライアントに露出する。 | 起動時に上流 `proxy` コントロールプレーンバックエンドを削除し、削除を証明できなければ**フェイルクローズ**(`src/lib.rs`);プロジェクト所有ルーターは `/mcp`、`/healthz`、`/readyz` のみ提供。トップレベル `/admin/*` はルートなし、ネストした `/mcp/admin/*` は上流ディスパッチ前にミドルウェアが拒否(`Authorization` ヘッダの有無に依存しない)。 | `tests/proxy.rs` `build_removes_upstream_proxy_control_plane_backend`、`external_http_surface_exposes_mcp_and_health_but_not_admin`;`src/lib.rs`、`src/gateway/router.rs` |
| T-06 | T | ミューテーション系ツール呼び出しが複数回実行される(リトライ・ヘッジ・フェイルオーバー・コアレッシング・レスポンスキャッシュによる副作用の重複)。 | ゲートウェイはリトライ・ヘッジ・ファンアウト・フェイルオーバー・キャッシュを**一切**実装しない。呼び出し単位のラッパーはタイムアウトのみ。これらを有効化する設定はバックエンド単位で検証に失敗する。ツールレベルの冪等性がモデル化・テストされない限り自動リトライは禁止のまま。 | `tests/policy.rs` `rejects_retry_hedging_and_cache_per_backend`、`rejects_fanout_failover_and_request_coalescing`;`tests/proxy.rs` `timeout_does_not_retry_a_mutating_tool` |
| T-07 | S | 名前空間の混乱: ツール名が誤ったバックエンドに解決される(プレフィックス衝突、曖昧な正規化、`proxy_` などの予約プレフィックス偽装)。 | バックエンド `id` と `prefix` は一意かつ小文字制約。`_` 区切りは固定。最終ツール名は事前計算され、衝突は起動時に**フェイルファスト**し両ソースを特定。予約プレフィックス(`proxy_`、`lomway_`、旧 `lmg_`)はポリシー管理。 | `src/namespace/collision.rs`;`tests/policy.rs` `rejects_wrong_namespace_separator`;`tests/proxy.rs` `same_named_backend_tools_are_namespaced_without_collision` |
| T-08 | D | クライアントが過大な引数や長時間呼び出しでゲートウェイを飽和させる。 | ツール引数サイズは上限付き(既定 1 MiB、`security.max_argument_size`);バックエンドごとにタイムアウト設定。リクエストは複製されないためファンアウト増幅も存在しない(T-06)。 | `docs/SECURITY.ja.md` §5;ポリシー検証(`src/gateway/policy.rs`) |
| T-09 | T, E | 改ざんされた設定がセキュリティ姿勢を拡大する(listener escape hatch、remote backend scope、hot reload、未知キーの既定値すり抜け)。 | トップ・`[server]`・backend各レベルで `deny_unknown_fields`;`schema_version` gate;`allow_non_loopback_listener` は常に拒否;`allow_non_loopback_backends` はT-02の狭いTailscale HTTPS profileだけを開く;hot reloadは無効でstartup policy validationを迂回できない。 | `src/config/validate.rs`;`tests/policy.rs` `rejects_hot_reload_until_gateway_policy_is_revalidated_on_reload`;`src/config/model.rs` 厳密パースのテスト |
| T-10 | I | シークレットがログ・設定ファイル・プロセス環境から公開成果物へ漏えいする。 | ゲートウェイ実行時にシークレットは不要;`${VAR}` 参照は起動時にプロセス環境のみへ解決;バックエンドへの認証情報転送なし;構造化ログはシークレット値を除外;公開リリースのハイジーンテストが私的トンネルパスとローカルビルド成果物を阻止。 | `tests/policy.rs` `public_release_hygiene_excludes_private_tunnel_paths_and_local_build_artifacts`;`src/config/load.rs` 環境変数解決のテスト |
| T-11 | T, S | サプライチェーン侵害: 依存クレート(またはその新解決)がコントロールプレーンや挙動変化を持ち込む。 | `mcp-proxy` は厳密ピン留め(`=0.4.3`)かつ `default-features = false` でプロトコル機能のみ;`tower-mcp` も厳密ピン留め(`=0.18.2`);`Cargo.lock` は追跡対象。解決変更には設定/スキーマレビュー、コントロールプレーン抑止リグレッション、全テストゲートを要求。 | `Cargo.toml`、`Cargo.lock`;`docs/SECURITY.ja.md` §7 |
| T-12 | S, T | 悪意あるローカルプロセスがloopback backend portを捕捉する、または侵害されたtrusted tailnet peerが設定済みremote MCP hostnameをserveする。 | **受容済みの残存リスク、明示設定で制約:** loopback endpointは厳密な固定 `/mcp`;remote endpointは狭いTailscale HTTPS profileとoperator選択のmachine/tailnet hostnameが必要。dynamic backend登録は存在しない(T-05)。backend回復はtransport再接続で行い、tool call自体はretryしない。 | `tests/proxy.rs` のbackend再起動回復テスト;`src/gateway/reconnect.rs`;§4 前提 2 |
| T-13 | S, D | `/mcp` におけるセッション悪用(期限切れ・他者のセッション ID、想定外メソッド)。 | Streamable HTTP のセマンティクスはピン留めされた上流スタックが所有。期限切れ初期セッションはプロキシクライアント経路で拒否。未知ルートは 404。 | `tests/proxy.rs` `reject_expired_initial_session`、`external_http_surface_exposes_mcp_and_health_but_not_admin` |

## 6. Listener / southbound分離モデル

listenerはloopback-onlyを維持し、southbound remote accessは別の明示trust profileとして扱う。強制は複数層で行う:

1. **既定値:** `DEFAULT_LISTEN_HOST = "127.0.0.1"`。backend URLもremote policyを明示有効化しない限りloopback。
2. **スキーマ検証:** listener hostは厳密に `127.0.0.1`。backend URLは厳密な `http://127.0.0.1:<port>/mcp`、または明示opt-in後だけ既定portの `https://<machine>.<tailnet>.ts.net/mcp`。
3. **ポリシー検証:** `allow_non_loopback_listener` は拒否継続。`allow_non_loopback_backends` は任意hostを許可せず、T-02のTailscale HTTPS shapeだけを有効化。hot reloadも無効。

非loopback **listener** 運用は今後も将来の明示security profileが必要。対応済みの非loopback **southbound** は上記Tailscale HTTPS profileだけ。

## 7. 認証なしローカル境界の根拠

北向き・南向き両境界に認証はない。これは見落としではなく、
文書化された意図的な決定である:

- **信頼される者:** すべてのローカルユーザーのすべてのローカルプロセス。
  ループバックは境界をマシン内に限定するが、マシン内のユーザーは分離しない。
- **ローカル認証を付けない理由:** ゲートウェイはシークレットを保持せず、
  どのローカルプロセスもバックエンドに直接到達できる範囲を超える権限を
  付与しない。管理トークンを追加しても境界は閉じず(リモートはもともと
  ループバックに届かない)、シークレット保持コンポーネントを新たに
  作るだけになる。
- **被害を限定するもの:** サーフェス最小化(`/mcp`、`/healthz`、`/readyz` のみ)、
  コントロールプレーン削除(T-05)、認証情報の非保持・非転送(T-10)、
  フェイルクローズ検証(T-09)。
- **残存リスク:** 悪意あるローカルプロセスは設定済みの任意のツールを
  呼び出し、ツール結果を観測できる(T-03/T-12 のとおり)。より強い分離が
  必要なオペレーターは、OS レベル(プロセス/ユーザー分離)で供給するか、
  将来の認証付きプロファイルを待つ必要がある。

## 8. 非目標

- マルチテナントまたはインターネット向け提供;
- ゲートウェイ側の認証・認可・テナント分離;
- ループバックホップのゲートウェイ側暗号化;
- バックエンドプロセスの監督やサンドボックス化;
- 敵対的ローカル管理者・カーネル攻撃者・物理アクセスへの対策。

## 9. 対象外の所有者

| 領域 | 所有者 |
|---|---|
| HTTP 以外でのバックエンドプロセスのライフサイクルと健全性 | 外部スーパーバイザー(例: Swibo 統合) |
| リモート侵入経路の認証 | Secure MCP Tunnel 統合 |
| シークレットの保管と復号 | 起動時に解決される外部 SOPS ストア |
| OS レベルのユーザー/プロセス分離 | OS の設定 |

## 10. レビュープロセス

以下のいずれかが変更された場合は本脅威モデルを改訂しなければならない:

- リスナーバインドポリシーまたはループバック検証ルール;
- 外部 HTTP サーフェス(`/mcp`、`/healthz`、`/readyz` の命名や範囲);
- ピン留めされた `mcp-proxy` / `tower-mcp` のバージョンやフィーチャーセット;
- リトライ/キャッシュ/ファンアウト禁止;
- 到達可能性を変えうる設定スキーマの変更(ホスト、URL、トランスポート、
  ポリシーを超えるタイムアウト)。

各改訂では §5 を完全に維持すること: すべての脅威 ID が最新の証拠を持つ
実施済み緩和策を少なくとも 1 つ保持するか、明示的に再トリアージし、
受容済み残存リスクを再記述する。
