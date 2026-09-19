# Jev Ultrafast MCP Integration and Recovery Plan

Status: design / implementation milestones  
Integration branch: `feature/jev-ultrafast-mcp-recovery`  
Upstream: `browser-use/jev-ultrafast`

## Purpose

Expose Jev Ultrafast as an independently managed MCP backend that Lomway can aggregate without moving browser-agent domain logic into Lomway.

The first implementation should stay intentionally small:

- keep Jev as the normal fast operation/target selector;
- support two text modes: caller-generated text and the existing internal helper model;
- use only information Jev Ultrafast already retains for the first recovery implementation;
- escalate to a reasoning LLM only after a clear no-progress boundary;
- if recovery crosses a fixed budget, stop Jev automation and return enough handoff metadata for another browser MCP to take over the exact browser target.

Lomway remains a thin aggregation boundary. It does not own the Jev loop, browser lifecycle, recovery policy, or browser execution.

## Target architecture

```text
Caller / orchestration LLM
        |
        | MCP
        v
Lomway
        |
        v
Jev Ultrafast MCP backend
        |
        +--> Jev: operation + target
        |
        +--> text_mode=caller
        |       |
        |       +--> NEED_TEXT -> caller generates value -> resume_text
        |
        +--> text_mode=internal
        |       |
        |       +--> existing small text helper LLM
        |
        v
deterministic freshness / visibility / occlusion checks
        |
        v
Browser Harness / CDP
        |
        +--> normal progress -> Jev loop
        |
        +--> BLOCKED / repeated no-progress
                |
                v
          Recovery LLM
                |
                +--> revised subgoal / avoid hints
                |
                v
             Jev loop
                |
                +--> equivalent block reaches limit
                        |
                        v
                HANDOFF_REQUIRED
                        |
                        v
                direct browser MCP
```

The recovery LLM must not emit selectors, coordinates, arbitrary JavaScript, or direct browser mutations. It may only return a revised subgoal, diagnosis, and bounded guidance for the Jev loop.

## Existing state available for lightweight recovery

The current Jev Ultrafast implementation already retains enough information for a first recovery pass:

- current page snapshot: URL, title, visible text, scroll state, action candidates, values/states, fingerprint and guards;
- action history: step, action label, kind, choice, probability, confidence, typed text, selected operation/target, whether the page changed, URL, latency and usage;
- decision history: Jev operation and target decisions, probabilities, confidence, raw answers, model, request body, latency and usage;
- optional screenshots/recording when explicitly enabled.

Milestone 1 recovery must use these existing structures. Do not add a new persistent logging subsystem or full DOM history.

## Browser handoff contract

A direct-browser fallback must be able to identify the exact target without guessing by URL.

Jev Ultrafast already stores the Browser Harness CDP `targetId` internally as `Browser.target`. Browser Harness also exposes `browser_list_tabs` and `browser_switch_tab(targetId)`.

Therefore the MCP backend must expose a handoff packet when recovery is exhausted:

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

Rules:

- `target_id` is the primary tab identity.
- Do not use URL alone because duplicate URLs/tabs are possible.
- Do not expose a raw CDP WebSocket URL by default.
- Include the Browser Harness connection name/mode so a fallback MCP can verify it is attached to the same browser instance.
- If both MCP backends are already configured to the same Browser Harness/CDP endpoint, a port does not need to be returned.
- If they are not guaranteed to share the same endpoint, startup/configuration must establish that relationship. A target ID from one Chrome instance is meaningless in another.
- A connection-scoped CDP `sessionId` is not a portable handoff identifier and should not be treated as one.

## Text modes

### `text_mode="caller"` — preferred MCP mode

When Jev selects `TYPE_TEXT`, the MCP backend returns a bounded request rather than calling a second LLM:

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

The caller supplies the value through a resume tool. The backend must re-check freshness immediately before typing. A stale resume token must not mutate the page.

### `text_mode="internal"`

Preserve the current Jev Ultrafast text helper path for autonomous runs, CLI use, and callers that cannot participate in a mid-run text handshake.

## Initial recovery boundary

Keep the first implementation explicit and conservative.

Default policy:

1. Normal Jev execution continues while progress is observed.
2. First block/no-progress stop -> one Recovery LLM attempt.
3. Recovery returns only:
   - diagnosis,
   - revised subgoal,
   - optional bounded `avoid` hints.
4. Resume the same browser target through Jev.
5. If an equivalent block occurs a second time, return `HANDOFF_REQUIRED` and stop autonomous Jev execution.
6. Also enforce a small global recovery budget so a series of different blockers cannot recurse indefinitely.

For the lightweight implementation, derive a block signature from existing state only, for example:

- current page fingerprint;
- current URL/title;
- block reason;
- the last few non-wait action labels/kinds and page-change flags.

Do not add semantic transition history until evidence shows that the existing state is insufficient.

## MCP surface

Start with a small public tool surface:

- `jev_browser_start` — create a run with URL, goal, and text mode;
- `jev_browser_step` — perform one bounded Jev step;
- `jev_browser_run` — run until done, blocked, text input is required, recovery is required, or handoff is required;
- `jev_browser_resume_text` — supply caller-generated text using a freshness-bound resume token;
- `jev_browser_inspect` — return current page/action/decision/history diagnostics plus handoff metadata;
- `jev_browser_close` — close the owned browser target/session.

Do not publish raw CDP primitives from this backend. Direct low-level browser control belongs to a separate browser MCP.

## Milestones

### M0 — Baseline and fork preparation

Deliverables:

- preserve the reviewed upstream baseline;
- create the integration development branch;
- document upstream commit and dependency versions before implementation;
- create the user fork when repository tooling is available, then create the fork-side branch `feature/lomway-mcp-recovery`.

Gate:

- no functional code changes;
- upstream baseline tests pass.

### M1 — Minimal MCP backend

Deliverables:

- stdio or loopback MCP entry point around the existing Agent;
- session ownership and bounded cleanup;
- start/step/run/inspect/close tools;
- no Lomway domain logic changes.

Gate:

- existing Jev Ultrafast tests remain green;
- MCP smoke test completes a simple click-only fixture;
- mutating operations are never automatically retried.

### M2 — Dual text mode

Deliverables:

- `caller` and `internal` text modes;
- `need_text` response;
- freshness-bound `resume_token`;
- `resume_text` revalidation before browser input.

Gate:

- caller text cannot be applied after page state becomes stale;
- internal mode preserves current behavior;
- no text value is invented when caller mode is selected.

### M3 — Lightweight Recovery LLM

Deliverables:

- recovery trigger on current blocked/no-progress conditions;
- recovery packet built only from existing `page + history + decisions`;
- Recovery LLM returns diagnosis/subgoal/avoid only;
- one normal Jev retry after recovery.

Gate:

- Recovery LLM has no direct browser mutation capability;
- repeated identical failures cannot cause an unbounded loop;
- no new persistent logger or full-page-history subsystem.

### M4 — Clear escalation and browser handoff

Deliverables:

- block signature;
- default equivalent-block limit of 2;
- bounded total recovery budget;
- `HANDOFF_REQUIRED` state;
- handoff packet containing Browser Harness connection identity and `target_id`.

Gate:

- Browser Harness MCP can list and switch to the exact returned `target_id`;
- duplicate URLs do not cause ambiguous takeover;
- no raw WebSocket credential is exposed by default.

### M5 — Lomway integration

Deliverables:

- run the Jev Ultrafast MCP as an independent backend;
- configure a namespace such as `jev_browser_*`;
- Swibo or another existing supervisor owns backend lifecycle;
- Lomway only aggregates and preserves tool schemas/results.

Gate:

- Lomway starts with the backend healthy;
- Lomway still starts according to its existing mixed healthy/unhealthy backend policy;
- `tools/list` contains only the intended Jev browser tools;
- no Lomway retry/caching/failover is enabled for mutating calls.

### M6 — End-to-end recovery and fallback test

Scenarios:

- normal goal completes with Jev only;
- caller text handshake;
- first block -> Recovery LLM -> success;
- first block -> recovery -> equivalent second block -> handoff;
- fallback Browser Harness MCP takes over the exact target;
- stale handoff/freshness cases fail closed;
- close/cleanup leaves no orphaned owned target.

Gate:

- all paths are deterministic about who owns browser mutation at each stage.

## Deferred work

Do not include these in the first implementation:

- full per-step DOM archival;
- a new transition-log database;
- screenshot-driven recovery by default;
- semantic A/B/A/B cycle detection requiring new history structures;
- automatic selection among multiple fallback browser MCPs;
- Jev-based Lomway tool routing;
- direct arbitrary CDP exposure from the Jev backend.

These can be added only if the lightweight recovery evidence shows a real need.

## Repository responsibility split

```text
signal-forge-lab/lomway
  - aggregation configuration
  - integration documentation
  - namespace / transport smoke tests

future signal-forge-lab/jev-ultrafast fork
  - MCP backend
  - Jev Agent integration
  - text modes
  - recovery policy
  - handoff packet
  - browser ownership / cleanup

browser-use/browser-harness
  - underlying CDP/browser helpers
  - separate direct-browser MCP fallback
```

This split preserves Lomway's rule that backend domain logic remains outside the gateway.
