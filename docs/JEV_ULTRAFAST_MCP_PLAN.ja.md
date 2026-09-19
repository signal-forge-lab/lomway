# Jev Ultrafast MCP統合・リカバリ計画

状態: 設計 / 実装マイルストーン  
統合用ブランチ: `feature/jev-ultrafast-mcp-recovery`  
上流: `browser-use/jev-ultrafast`

## 目的

Jev Ultrafastを独立したMCPバックエンドとして公開し、Lomwayはそれを集約するだけにします。ブラウザエージェント固有の判断・実行・リカバリ処理をLomway本体へ持ち込みません。

初期実装は意図的に軽量にします。

- 通常時はJevが高速に操作種別と対象を選ぶ。
- 文字入力は「呼び出し元生成」と「既存の内部小型LLM」の2モードを持つ。
- 最初のRecovery LLMは、現在のJev Ultrafastがすでに保持している情報だけを使う。
- 明確な進展停止基準を超えた場合だけReasoning LLMへ上げる。
- Recoveryの上限を超えたらJevでの自動操作を止め、別Browser MCPが同じブラウザタブを即座に引き継げる情報を返す。

Lomwayは薄い集約境界のままです。Jev loop、browser lifecycle、recovery policy、browser executionの所有者にはしません。

## 目標構成

```text
呼び出し元 / オーケストレーションLLM
        |
        | MCP
        v
Lomway
        |
        v
Jev Ultrafast MCP backend
        |
        +--> Jev: 操作種別 + 対象選択
        |
        +--> text_mode=caller
        |       |
        |       +--> NEED_TEXT -> 呼び出し元が文字生成 -> resume_text
        |
        +--> text_mode=internal
        |       |
        |       +--> 現行の小型文字生成LLM
        |
        v
deterministicなfreshness / visibility / occlusion検証
        |
        v
Browser Harness / CDP
        |
        +--> 正常進行 -> Jev loop継続
        |
        +--> BLOCKED / 反復する進展なし
                |
                v
          Recovery LLM
                |
                +--> 修正したsubgoal / avoid指示
                |
                v
             Jev loop
                |
                +--> 同等blockが規定回数に到達
                        |
                        v
                HANDOFF_REQUIRED
                        |
                        v
                別の直接操作Browser MCP
```

Recovery LLMはselector、座標、任意JavaScript、直接のbrowser mutationを返してはいけません。返せるのは原因説明、修正subgoal、Jev loop向けの限定的な回避指示までです。

## 軽量Recoveryに現在すでに使える情報

現行Jev Ultrafastは、最初のRecoveryに十分な情報をすでに保持しています。

- 現在ページ: URL、title、visible text、scroll、操作候補、値/状態、fingerprint、guards。
- 操作履歴: step、action label、kind、choice、probability、confidence、入力文字、operation/target、page_changed、URL、latency、usage。
- 判断履歴: Jevのoperation/target判断、probabilities、confidence、raw answers、model、request body、latency、usage。
- 明示的に有効化した場合のみ screenshot / recording。

最初のRecovery実装はこれらだけを使います。新しいpersistent loggerや全DOM履歴は追加しません。

## 別Browser MCPへの引継ぎ契約

直接操作へ切り替える際、URLから推測せず対象タブを一意に特定できる必要があります。

Jev UltrafastはBrowser HarnessのCDP `targetId` を内部の `Browser.target` に保持しています。Browser Harness MCP側にも `browser_list_tabs` と `browser_switch_tab(targetId)` があります。

したがってRecoveryを使い切った場合、MCP backendは次のようなhandoff packetを返します。

```json
{
  "handoff": {
    "required": true,
    "reason": "repeated_block",
    "browser_backend": "browser-harness",
    "browser_connection": {
      "name": "default",
      "mode": "default"
    },
    "target_id": "<cdp targetId>",
    "url": "<current url>",
    "title": "<current title>",
    "fingerprint": "<current semantic fingerprint>",
    "status": "blocked",
    "block_count": 2,
    "recovery_attempts": 1
  }
}
```

ルール:

- タブの主識別子は `target_id`。
- 同一URLの複数タブがあり得るためURLだけで引き継がない。
- raw CDP WebSocket URLはデフォルトでは返さない。
- fallback MCPが同じbrowser instanceへ接続していることを確認できるよう、Browser Harnessのconnection name/modeを返す。
- 両MCPが同じBrowser Harness/CDP endpointへ設定済みなら、portをMCP responseへ毎回返す必要はない。
- 同じendpointである保証がない場合は、起動/config段階で接続先を一致させる必要がある。別Chrome instanceでは同じtargetIdを利用できない。
- CDP `sessionId` はconnection scopeなのでportableなhandoff IDとして扱わない。

## 文字入力の2モード

### `text_mode="caller"` — MCP利用時の推奨

Jevが `TYPE_TEXT` を選んだら、別LLMを内部から呼ばず、MCP結果として必要情報を返します。

```json
{
  "status": "need_text",
  "field": {
    "label": "Where from?",
    "role": "textbox",
    "current_value": ""
  },
  "context": {
    "goal": "...",
    "page_title": "...",
    "visible_text": "...",
    "recent_actions": []
  },
  "resume_token": "<opaque freshness-bound token>"
}
```

呼び出し元が文字を生成し、resume toolで渡します。入力直前に必ずfreshnessを再検証し、古いresume tokenでは画面を変更しません。

### `text_mode="internal"`

完全自動run、CLI、途中で呼び出し元へ戻せないclientのために、現行の小型text helper経路を残します。

## 初期Recoveryの明確な基準線

最初は単純かつ保守的にします。

1. 正常に進展している間はJev実行を継続。
2. 最初のblock / no-progress停止でRecovery LLMを1回呼ぶ。
3. Recovery LLMが返すのは以下のみ。
   - diagnosis
   - revised subgoal
   - 必要なら限定的な `avoid` 指示
4. 同じbrowser targetでJev loopを再開。
5. 同等のblockが2回目に到達したら `HANDOFF_REQUIRED` としてJev自動実行を停止。
6. 異なるblockを繰り返して無限Recoveryにならないよう、小さな総Recovery上限も設ける。

軽量実装では既存stateだけからblock signatureを作ります。例:

- current page fingerprint
- current URL/title
- block reason
- 直近数件のnon-wait action label/kindとpage_changed

既存情報で不足すると実証されるまでは、新しいsemantic transition historyは追加しません。

## MCP公開面

最初は少数toolに限定します。

- `jev_browser_start` — URL、goal、text modeでrun開始。
- `jev_browser_step` — Jevの1 bounded step。
- `jev_browser_run` — done / blocked / text待ち / recovery / handoffまで進める。
- `jev_browser_resume_text` — freshness-bound token付きで呼び出し元生成文字を渡す。
- `jev_browser_inspect` — page/action/decision/historyとhandoff情報を見る。
- `jev_browser_close` — 所有browser target/sessionを閉じる。

このbackendからraw CDP primitiveは公開しません。低レベル直接操作は別Browser MCPの責務です。

## マイルストーン

### M0 — Baseline / fork準備

成果物:

- reviewed upstream baselineを固定。
- 統合用development branchを作成。
- 実装前にupstream commit/dependency versionを記録。
- repository toolingが利用可能になった時点でuser forkを作成し、fork側に `feature/lomway-mcp-recovery` を作成。

完了条件:

- functional code変更なし。
- upstream baseline testがgreen。

### M1 — 最小MCP backend

成果物:

- 現行Agentを包むstdioまたはloopback MCP entry point。
- session ownershipとbounded cleanup。
- start / step / run / inspect / close。
- Lomway本体へdomain logicを追加しない。

完了条件:

- 現行Jev Ultrafast testがgreen。
- click-only fixtureでMCP smoke成功。
- mutating operationを自動retryしない。

### M2 — 2つのtext mode

成果物:

- `caller` / `internal`。
- `need_text` response。
- freshness-bound `resume_token`。
- browser input直前の `resume_text` 再検証。

完了条件:

- page stateが古くなった後のcaller textは入力不可。
- internal modeは現行動作を維持。
- caller mode時にbackendが入力値を勝手に生成しない。

### M3 — 軽量Recovery LLM

成果物:

- 現行blocked/no-progress条件からrecovery trigger。
- 既存 `page + history + decisions` のみからrecovery packet生成。
- Recovery LLMはdiagnosis/subgoal/avoidのみ返す。
- recovery後、通常Jevを1回再開。

完了条件:

- Recovery LLMからbrowserを直接変更できない。
- 同一失敗が無制限にloopしない。
- 新しいpersistent logger / full page historyを追加しない。

### M4 — 明確な打切りとbrowser handoff

成果物:

- block signature。
- 同等blockのデフォルト上限2回。
- total recovery budget。
- `HANDOFF_REQUIRED`。
- Browser Harness connection identity + `target_id` を含むhandoff packet。

完了条件:

- Browser Harness MCPから返却 `target_id` をlist/switchできる。
- 同一URLの複数tabでも曖昧にならない。
- raw WebSocket credentialをデフォルトで公開しない。

### M5 — Lomway統合

成果物:

- Jev Ultrafast MCPを独立backendとして起動。
- `jev_browser_*` 等のnamespace設定。
- lifecycleはSwibo等の既存supervisorが所有。
- Lomwayは集約とschema/result維持のみ。

完了条件:

- backend healthyでLomway起動。
- backend障害時も既存のmixed healthy/unhealthy policyを維持。
- `tools/list` に意図したJev browser toolsだけが出る。
- mutating callへのLomway側retry/cache/failoverなし。

### M6 — End-to-end Recovery / fallback検証

シナリオ:

- Jevだけで正常完了。
- caller text handshake。
- 1回目block -> Recovery LLM -> 成功。
- 1回目block -> recovery -> 同等2回目block -> handoff。
- fallback Browser Harness MCPが同じtargetを直接操作。
- stale handoff/freshnessはfail closed。
- close後にowned targetを残さない。

完了条件:

- 各段階でbrowser mutationのauthorityが一意に決まる。

## 後回しにするもの

初期実装には入れません。

- 各stepの完全DOM archive。
- 新しいtransition log database。
- screenshotを常時使うRecovery。
- 新履歴構造が必要な高度なA/B/A/B semantic cycle検出。
- 複数fallback browser MCPの自動選択。
- JevによるLomway tool routing。
- Jev backendからの任意raw CDP公開。

軽量Recoveryで本当に不足すると分かったものだけ後から追加します。

## repository責務

```text
signal-forge-lab/lomway
  - aggregation config
  - integration docs
  - namespace / transport smoke tests

将来の signal-forge-lab/jev-ultrafast fork
  - MCP backend
  - Jev Agent integration
  - text modes
  - recovery policy
  - handoff packet
  - browser ownership / cleanup

browser-use/browser-harness
  - 基礎CDP/browser helpers
  - 別の直接操作Browser MCP fallback
```

この分離により、backend domain logicをLomway本体へ移さないという既存方針を維持します。
