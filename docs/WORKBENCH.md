# Gateway workbench: skills, actions, pipelines, and file transfer

Lomway can provide four Workbridge-style features itself, so a deployment
that uses a generic coding backend (for example Coding Tools MCP) does not
need Workbridge for them. Every feature is **disabled by default** and is
enabled per section in the public configuration.

| Feature | Workbridge equivalent | Lomway surface |
|---|---|---|
| Skills | `open_workspace` skills + `read` of SKILL.md | `initialize` instructions, `lomway_search_tools` / `lomway_describe_tool` (`skill:<name>`) |
| Actions | `run_workspace_action` | `lomway_list_actions`, `lomway_run_action`, `lomway_action_status` |
| Pipelines | `orchestration_prepare` / `orchestration_run` (minimal) | `lomway_run_pipeline` |
| Receive a chat file | `download_artifact` | `lomway_receive_file` (`openai/fileParams`) |
| Return a file | `run_workspace_action action=publish_artifact` | `lomway_send_file` |

A misconfigured feature never prevents the gateway from starting: the error
is logged and returned by that feature's tools.

## Skills

```toml
[skills]
enabled = true
# default_paths = true        # ~/.agents/skills, devspace skills dir, <agent_dir>/skills
# agent_dir = "~/.codex"
# paths = []                  # extra directories or single .md files
# project_dirs = []           # each <dir>/.agents/skills
# read_root = "${USERPROFILE}/workspace"   # file backend workspace root
# instructions_limit = 40
# rescan_seconds = 60
```

Scan order and discovery rules match Workbridge (pi-coding-agent): a
directory with `SKILL.md` is a skill root and is not searched further; `*.md`
files directly inside a scanned root are skills; names starting with `.` and
`node_modules` are skipped; links and junctions are followed; the same file is
loaded once; the first skill of a name wins. Frontmatter `name` (default: the
parent directory name) and `description` (required) are read;
`disable-model-invocation: true` keeps a skill out of the instructions list.
`.gitignore` files inside skill directories are not evaluated.

* The `initialize` instructions gain a short list (`- name: description`),
  capped at `instructions_limit`. Restricted callers keep receiving the
  generic restricted instructions.
* Every skill is indexed as `skill:<name>` for `lomway_search_tools`. Skills
  are instructions, not tools, so they are shown to every caller regardless of
  the caller's tool allowlist.
* `lomway_describe_tool name="skill:<name>"` returns the path and how to read
  it: with `coding_tools_read_file` and a workspace-relative path when the
  canonical file lies inside `read_root`, otherwise with
  `coding_tools_exec_command` (`Get-Content -Raw -LiteralPath ...`).
* The scan is repeated at most every `rescan_seconds` when the index is read,
  so added or removed skills appear in search without a restart. The
  instructions list is a startup snapshot (MCP delivers instructions only at
  `initialize`).

## Actions

```toml
[actions]
enabled = true
definitions = "${LOCALAPPDATA}/Lomway/actions.toml"   # outside the repository
allowed_roots = ["${USERPROFILE}/src"]
# audit_log = "${LOCALAPPDATA}/Lomway/actions-audit.jsonl"
# max_output_bytes = 262144
# max_concurrent = 4
```

The definition file is re-read whenever its size or modification time
changes. Tool schemas never change, so new actions need no client reload. A
broken edit keeps the last good definitions and reports the error in
`lomway_list_actions`. See `examples/actions.example.toml`.

```toml
schema_version = 1

[[actions]]
name = "git_status"                       # [a-z0-9_.-]
description = "Show short git status for a repository."
policy = ["read_only"]                    # Workbridge policy names
effects = ["process.execute", "git.read"] # Workbridge effect names
program = "git"                           # executable; no shell
args = ["-C", "{repo}", "status", "--short"]
cwd = "${USERPROFILE}/src"                # must be inside allowed_roots
timeout_seconds = 120                     # 1 .. 14400
allow_working_directory = false
env = {}                                  # fixed, non-secret values
inherit_env = []                          # extra variables to pass through
[actions.parameters]                      # JSON Schema subset
required = ["repo"]
[actions.parameters.properties.repo]
type = "string"
format = "relative-path"
```

Rules:

* **No shell.** The command runs as an argv vector. `{name}` inside an
  argument is replaced and the argument stays one argument; `{name...}`
  expands an array parameter to one argument per item; `{{`/`}}` are literal
  braces. An argument that references an omitted optional parameter is
  dropped. Definitions that pass parameters to `cmd`, `powershell`, `pwsh`,
  `bash`, `sh`, `wsl`, `env`, or a `.bat`/`.cmd`/`.ps1` file are refused.
* **Parameters** support `string`, `integer`, `number`, `boolean`, and arrays of
  strings, with `enum`, `minLength`/`maxLength`, `minimum`/`maximum`,
  `maxItems`, `default`, and `format = "relative-path"`. Unknown keywords and
  unknown parameters are rejected. Strings may not contain control
  characters, and may not start with `-` unless `allow_leading_dash = true`
  (prevents option injection).
* **Directories.** `cwd` is canonicalized and must lie inside
  `actions.allowed_roots`. A caller-supplied `working_directory` is accepted
  only when `allow_working_directory = true` and must resolve inside `cwd`.
  `relative-path` parameters that exist must resolve inside the working
  directory.
* **Environment.** Children get an allowlist (`PATH`, `SYSTEMROOT`, `TEMP`,
  user profile directories, `CARGO_HOME`, ...), plus `env` and
  `inherit_env`. `LOMWAY_*` variables (backend tokens) are never passed.
* **Timeouts.** The whole process tree is killed at `timeout_seconds`
  (`taskkill /T /F` on Windows).
* **Output** is kept as head and tail, `max_output_bytes` per stream.
* **Runs.** `lomway_run_action` waits up to `wait_ms` (default 20 s, max
  25 s) and otherwise returns a `run_id` for `lomway_action_status`. Finished
  runs are kept for one hour (at most 32). At most `max_concurrent` actions
  run at once; extra requests are refused rather than queued.
* `dry_run = true` returns the resolved program, arguments, directory,
  timeout, policy, and effects without running anything.
* Unknown actions return the current catalog, like Workbridge.
* **Audit.** Every request and finished run is logged through tracing
  (`lomway::actions`) and, when `audit_log` is set, appended as JSON Lines.
  Argument values are recorded only as SHA-256 prefixes; output is never
  logged.

The definition file is the trust anchor: whoever can edit it decides what the
gateway runs. Keep it in a per-user directory outside the repository.

## Pipelines

```toml
[[pipelines]]
name = "verify"
description = "Format check, then tests."
effects = { allow = ["process.execute", "workspace.read", "workspace.write"], deny = ["git.push", "network.write"] }
[pipelines.parameters.properties.repo]
type = "string"
format = "relative-path"
[[pipelines.steps]]
action = "cargo_fmt_check"
parameters = { repo = "{repo}" }
[[pipelines.steps]]
action = "cargo_test"
parameters = { repo = "{repo}" }
continue_on_error = false
```

* **Effect guard** (Workbridge `assertBackendEffectsAllowed`): each step's
  action effects must all be in `allow` and none in `deny`, checked when the
  file is loaded and again before every plan. A violation is `EFFECT_DENIED`.
* Step parameters may reference pipeline parameters: `"{name}"` alone passes
  the value unchanged (any type); inside a longer string it interpolates.
* `dry_run = true` returns every resolved step and a plan digest (SHA-256 of
  canonical JSON). Passing that digest as `expected_digest` runs exactly that
  plan; if the definitions or parameters changed, the run fails with
  `PLAN_INVALIDATED`.
* Steps run in order and stop at the first failure unless
  `continue_on_error = true`. Poll with `lomway_action_status`.

Not implemented (Workbridge orchestration features that need agent
backends): LLM worker/reviewer stages, routing policies, budgets,
`HUMAN_REQUIRED`, pause/resume/cancel, and completion policies.

## File transfer

```toml
[file_transfer]
enabled = true
inbox_dir = "${LOCALAPPDATA}/Lomway/inbox"
# outbox_dirs = []
# max_receive_bytes = 104857600   # 100 MiB
# max_send_bytes = 10485760       # 10 MiB
```

`lomway_receive_file(file, path?)` declares `_meta["openai/fileParams"] =
["file"]`, so ChatGPT passes the attached or generated file as an object with
`download_url`, `file_id`, and optional `mime_type`, `file_name`, `name`,
`size` (unknown keys are rejected). Rules follow Workbridge:

* only `https` on port 443 to `files.oaiusercontent.com` or
  `oaisdmntpr<alnum>.blob.core.windows.net`, without user info or fragment;
  redirects are followed manually, at most three, each re-checked; 30 s
  timeout;
* size checked against metadata, `Content-Length`, and while streaming;
* the destination is relative to `inbox_dir` (default: the host file name,
  else `<file_id><ext>`); absolute paths, drive or UNC prefixes, `..`, `:`,
  reserved characters, device names, and trailing dots or spaces are
  rejected;
* each parent directory is created one level at a time and must not be a
  symlink, junction, or other reparse point;
* bytes go to an exclusive `.lomway-receive-*.partial` file, are hashed and
  synced, and are published with a hard link, which never overwrites; the
  partial file is always removed; the final canonical path is re-checked.

The result has the inbox-relative path, the absolute path (for other tools),
size, SHA-256, and MIME type.

Limitation: Workbridge additionally pins every parent directory handle while
writing. Lomway re-verifies the parent chain before publishing and the final
canonical path afterwards, which detects but does not prevent a concurrent
local process swapping a directory during the download.

`lomway_send_file(path, outbox?)` returns one existing regular file from the
inbox (or an outbox, by 1-based index) as an MCP embedded resource (`blob`,
base64, with a MIME type guessed from the extension) after the same link and
traversal checks, up to `max_send_bytes`.
