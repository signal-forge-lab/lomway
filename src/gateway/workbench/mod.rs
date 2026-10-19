//! Workbridge-parity features owned by the gateway itself:
//!
//! * **skills** — SKILL.md discovery advertised in `initialize` instructions
//!   and in capability search (`skill:<name>`);
//! * **actions** — `lomway_list_actions`, `lomway_run_action`,
//!   `lomway_action_status` over an operator-maintained definition file;
//! * **pipelines** — `lomway_run_pipeline`: schema-defined multi-step runs
//!   with dry-run, plan digest, and an effect guard;
//! * **file transfer** — `lomway_receive_file` (ChatGPT `openai/fileParams`)
//!   and `lomway_send_file` (embedded resource).
//!
//! Every feature is disabled unless configured. A misconfigured feature
//! never stops the gateway from starting (the gateway may be the only remote
//! access path to the machine): the problem is logged and reported by the
//! feature's tools instead.

pub(crate) mod actions;
pub(crate) mod paths;
pub(crate) mod runner;
pub(crate) mod schema;
pub(crate) mod skills;
pub(crate) mod transfer;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use tower_mcp::protocol::{Content, ResourceContent};
use tower_mcp::schemars::JsonSchema;
use tower_mcp::{CallToolResult, Tool, ToolBuilder};

use crate::config::model::GatewayConfig;
use crate::gateway::capability::{IndexEntry, Invocation, Metadata, Status, ToolSource};
use crate::gateway::schema_compat::portable_input_schema;

use self::actions::DefinitionStore;
use self::runner::{AuditLog, RunRegistry};
use self::skills::SkillRegistry;
use self::transfer::{FileReference, Inbox, TrustedHosts};

const DEFAULT_WAIT_MS: u64 = 20_000;
const MAX_WAIT_MS: u64 = 25_000;

pub(crate) const RUN_ACTION_TOOL: &str = "lomway_run_action";
pub(crate) const RUN_PIPELINE_TOOL: &str = "lomway_run_pipeline";

#[derive(Debug, Clone)]
struct ActionsRuntime {
    store: DefinitionStore,
    runs: RunRegistry,
}

/// Runtime state for all optional gateway-owned features.
#[derive(Debug, Clone, Default)]
pub(crate) struct Workbench {
    skills: Option<SkillRegistry>,
    skills_instructions_limit: usize,
    actions: Option<ActionsRuntime>,
    actions_error: Option<String>,
    transfer: Option<Inbox>,
    transfer_error: Option<String>,
}

impl Workbench {
    pub(crate) fn from_config(config: &GatewayConfig) -> Self {
        let mut workbench = Self {
            skills_instructions_limit: config.skills.instructions_limit,
            ..Self::default()
        };
        if config.skills.enabled {
            let registry = SkillRegistry::new(&config.skills);
            for diagnostic in registry.plan_diagnostics() {
                tracing::warn!(target: "lomway::skills", %diagnostic, "skill configuration");
            }
            let scanned = registry.current();
            tracing::info!(
                target: "lomway::skills",
                skills = scanned.skills.len(),
                diagnostics = scanned.diagnostics.len(),
                "skills discovered"
            );
            workbench.skills = Some(registry);
        }
        if config.actions.enabled {
            match Self::actions_runtime(config) {
                Ok(runtime) => {
                    if let (_, Some(error)) = runtime.store.current() {
                        tracing::warn!(target: "lomway::actions", %error, "action definitions not loaded");
                    }
                    workbench.actions = Some(runtime);
                }
                Err(error) => {
                    tracing::warn!(target: "lomway::actions", error = %format!("{error:#}"), "actions disabled");
                    workbench.actions_error = Some(format!("{error:#}"));
                }
            }
        }
        if config.file_transfer.enabled {
            match Self::inbox(config) {
                Ok(inbox) => workbench.transfer = Some(inbox),
                Err(error) => {
                    tracing::warn!(target: "lomway::transfer", error = %format!("{error:#}"), "file transfer disabled");
                    workbench.transfer_error = Some(format!("{error:#}"));
                }
            }
        }
        workbench
    }

    fn actions_runtime(config: &GatewayConfig) -> anyhow::Result<ActionsRuntime> {
        let settings = &config.actions;
        let definitions = paths::expand_absolute(
            settings.definitions.as_deref().unwrap_or_default(),
            "actions.definitions",
        )?;
        let mut roots = Vec::new();
        for root in &settings.allowed_roots {
            let path = paths::expand_absolute(root, "actions.allowed_roots")?;
            roots.push(paths::canonical_dir(&path, "actions.allowed_roots entry")?);
        }
        let audit = match settings.audit_log.as_deref() {
            Some(path) => Some(paths::expand_absolute(path, "actions.audit_log")?),
            None => None,
        };
        Ok(ActionsRuntime {
            store: DefinitionStore::new(definitions, roots),
            runs: RunRegistry::new(
                settings.max_concurrent,
                settings.max_output_bytes,
                AuditLog::new(audit),
            ),
        })
    }

    fn inbox(config: &GatewayConfig) -> anyhow::Result<Inbox> {
        let settings = &config.file_transfer;
        let root = paths::expand_absolute(
            settings.inbox_dir.as_deref().unwrap_or_default(),
            "file_transfer.inbox_dir",
        )?;
        if !root.exists() {
            std::fs::create_dir_all(&root)?;
        }
        let metadata = std::fs::symlink_metadata(&root)?;
        anyhow::ensure!(
            !paths::is_link_or_reparse(&metadata),
            "file_transfer.inbox_dir must not be a link or reparse point"
        );
        let root = paths::canonical_dir(&root, "file_transfer.inbox_dir")?;
        let mut outboxes: Vec<PathBuf> = Vec::new();
        for dir in &settings.outbox_dirs {
            let path = paths::expand_absolute(dir, "file_transfer.outbox_dirs")?;
            outboxes.push(paths::canonical_dir(
                &path,
                "file_transfer.outbox_dirs entry",
            )?);
        }
        Ok(Inbox {
            root,
            outboxes,
            max_receive_bytes: settings.max_receive_bytes,
            max_send_bytes: settings.max_send_bytes,
            trusted: TrustedHosts::OpenAi,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.skills.is_none()
            && self.actions.is_none()
            && self.actions_error.is_none()
            && self.transfer.is_none()
            && self.transfer_error.is_none()
    }

    /// Text appended to the deployment's `initialize` instructions.
    pub(crate) fn instructions_suffix(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.actions.is_some() {
            parts.push("Gateway actions: lomway_list_actions lists operator-registered actions and pipelines; run one with lomway_run_action (dry_run=true shows the exact command first) and poll long runs with lomway_action_status. Multi-step pipelines run with lomway_run_pipeline (dry_run returns a plan digest; pass it as expected_digest).".to_string());
        }
        if self.transfer.is_some() {
            parts.push("Files: when the user attaches or generates a file that must reach this machine, call lomway_receive_file with the native file value (and optionally a relative path); it is saved in the gateway inbox and the absolute path is returned. Return a small file to the user with lomway_send_file. Never paste base64 or signed URLs into shell commands.".to_string());
        }
        if let Some(skills) = &self.skills
            && let Some(block) =
                skills::instructions_block(&skills.current(), self.skills_instructions_limit)
        {
            parts.push(block);
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    /// Dynamic capability-index entries (skills, actions, pipelines).
    pub(crate) fn index_entries(&self) -> Vec<IndexEntry> {
        let mut entries = Vec::new();
        if let Some(skills) = &self.skills {
            for skill in &skills.current().skills {
                entries.push(entry(
                    format!("skill:{}", skill.name),
                    "skills",
                    skill.description.clone(),
                    None,
                    true,
                    vec![
                        skill.name.clone(),
                        "skill".to_string(),
                        "スキル".to_string(),
                    ],
                ));
            }
        }
        if let Some(runtime) = &self.actions {
            let (definitions, _) = runtime.store.current();
            for action in definitions.actions.values() {
                entries.push(entry(
                    format!("action:{}", action.name),
                    "actions",
                    action.description.clone(),
                    Some(RUN_ACTION_TOOL),
                    false,
                    vec![
                        action.name.clone(),
                        "action".to_string(),
                        "アクション".to_string(),
                    ],
                ));
            }
            for pipeline in definitions.pipelines.values() {
                entries.push(entry(
                    format!("pipeline:{}", pipeline.name),
                    "pipelines",
                    pipeline.description.clone(),
                    Some(RUN_PIPELINE_TOOL),
                    false,
                    vec![
                        pipeline.name.clone(),
                        "pipeline".to_string(),
                        "パイプライン".to_string(),
                    ],
                ));
            }
        }
        entries
    }

    /// `lomway_describe_tool` for `skill:`, `action:`, and `pipeline:` names.
    /// `None` means the name is not one of ours.
    pub(crate) fn describe(&self, name: &str, can_call: impl Fn(&str) -> bool) -> Option<Value> {
        if let Some(skill_name) = name.strip_prefix("skill:") {
            let skill = self.skills.as_ref()?.find(skill_name)?;
            return Some(json!({
                "name": name,
                "backend": "skills",
                "invocation": Invocation::Internal,
                "status": Status::Declared,
                "description": skill.description,
                "path": skill.path,
                "canonical_path": skill.canonical_path,
                "read_root_relative": skill.read_root_relative,
                "source": skill.source,
                "how_to_invoke": skills::read_hint(&skill),
            }));
        }
        if let Some(action_name) = name.strip_prefix("action:") {
            if !can_call(RUN_ACTION_TOOL) {
                return None;
            }
            let (definitions, _) = self.actions.as_ref()?.store.current();
            let action = definitions.actions.get(action_name)?;
            let mut body = action.summary();
            body["name"] = json!(name);
            body["how_to_invoke"] = json!(format!(
                "Call lomway_run_action with action=\"{action_name}\" and parameters matching the schema; dry_run=true returns the resolved command without running it."
            ));
            return Some(body);
        }
        if let Some(pipeline_name) = name.strip_prefix("pipeline:") {
            if !can_call(RUN_PIPELINE_TOOL) {
                return None;
            }
            let (definitions, _) = self.actions.as_ref()?.store.current();
            let pipeline = definitions.pipelines.get(pipeline_name)?;
            let mut body = pipeline.summary();
            body["name"] = json!(name);
            body["how_to_invoke"] = json!(format!(
                "Call lomway_run_pipeline with pipeline=\"{pipeline_name}\" and dry_run=true to get the plan and digest, then call it again with expected_digest to run exactly that plan."
            ));
            return Some(body);
        }
        None
    }

    /// The gateway-owned tools to publish (names without the `lomway_`
    /// namespace prefix, which the channel backend adds).
    pub(crate) fn tools(self: &Arc<Self>) -> Vec<Tool> {
        let mut tools = Vec::new();
        if self.actions.is_some() || self.actions_error.is_some() {
            tools.extend(self.action_tools());
        }
        if self.transfer.is_some() || self.transfer_error.is_some() {
            tools.extend(self.transfer_tools());
        }
        tools
    }

    fn actions_or_error(&self) -> Result<&ActionsRuntime, tower_mcp::Error> {
        self.actions.as_ref().ok_or_else(|| {
            tower_mcp::Error::tool(format!(
                "gateway actions are unavailable: {}",
                self.actions_error.as_deref().unwrap_or("not configured")
            ))
        })
    }

    fn inbox_or_error(&self) -> Result<&Inbox, tower_mcp::Error> {
        self.transfer.as_ref().ok_or_else(|| {
            tower_mcp::Error::tool(format!(
                "file transfer is unavailable: {}",
                self.transfer_error.as_deref().unwrap_or("not configured")
            ))
        })
    }

    fn action_tools(self: &Arc<Self>) -> Vec<Tool> {
        let list_state = self.clone();
        let list = ToolBuilder::new("list_actions")
            .description("List the operator-registered gateway actions and pipelines with their parameter schemas, policy, and effects. New definitions appear here without reconnecting.")
            .input_schema(portable_input_schema::<EmptyInput>())
            .read_only()
            .handler(move |_: EmptyInput| {
                let state = list_state.clone();
                async move {
                    let runtime = state.actions_or_error()?;
                    let (definitions, error) = runtime.store.current();
                    let mut body = definitions.catalog();
                    if let Some(error) = error {
                        body["definition_error"] = json!(error);
                    }
                    Ok(CallToolResult::json(body))
                }
            })
            .build();

        let run_state = self.clone();
        let run = ToolBuilder::new("run_action")
            .description("Resolve, validate, and run one registered gateway action. The gateway owns the concrete command (argv, no shell), working directory, timeout, and environment; parameters are validated against the action's schema. Use dry_run=true to see the resolved command without running it. Returns the result, or a run_id to poll with lomway_action_status when the action is still running. Unknown actions return the current catalog.")
            .input_schema(portable_input_schema::<RunActionInput>())
            .handler(move |input: RunActionInput| {
                let state = run_state.clone();
                async move { state.run_action(input).await }
            })
            .build();

        let status_state = self.clone();
        let status = ToolBuilder::new("action_status")
            .description("Wait briefly for a gateway action or pipeline run and return its state, exit codes, and bounded stdout/stderr.")
            .input_schema(portable_input_schema::<StatusInput>())
            .read_only()
            .handler(move |input: StatusInput| {
                let state = status_state.clone();
                async move {
                    let runtime = state.actions_or_error()?;
                    let wait = Duration::from_millis(input.wait_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS));
                    match runtime.runs.wait(&input.run_id, wait).await {
                        Some(snapshot) => Ok(CallToolResult::json(serde_json::to_value(snapshot)?)),
                        None => Err(tower_mcp::Error::tool(format!(
                            "unknown or expired run_id '{}'",
                            input.run_id
                        ))),
                    }
                }
            })
            .build();

        let pipeline_state = self.clone();
        let pipeline = ToolBuilder::new("run_pipeline")
            .description("Plan or run a registered multi-step gateway pipeline. Every step is a registered action; the pipeline's effect policy is enforced (EFFECT_DENIED). dry_run=true returns the resolved plan and its digest; pass that digest as expected_digest to run exactly that plan (a changed plan fails with PLAN_INVALIDATED). Steps run in order and stop at the first failure unless marked continue_on_error.")
            .input_schema(portable_input_schema::<RunPipelineInput>())
            .handler(move |input: RunPipelineInput| {
                let state = pipeline_state.clone();
                async move { state.run_pipeline(input).await }
            })
            .build();
        vec![list, run, status, pipeline]
    }

    async fn run_action(&self, input: RunActionInput) -> Result<CallToolResult, tower_mcp::Error> {
        let runtime = self.actions_or_error()?;
        let (definitions, definition_error) = runtime.store.current();
        let dry_run = input.dry_run.unwrap_or(false);
        let audit = runtime.runs.audit();
        let Some(action) = definitions.actions.get(&input.action) else {
            audit.record_request(
                "action",
                &input.action,
                dry_run,
                false,
                "unknown action",
                None,
            );
            let mut message = format!(
                "Unknown gateway action '{}'. Available actions and pipelines:\n{}",
                input.action,
                serde_json::to_string_pretty(&definitions.catalog())?
            );
            if let Some(error) = definition_error {
                message.push_str(&format!("\nDefinition file error: {error}"));
            }
            return Err(tower_mcp::Error::tool(message));
        };
        let resolved = match action.resolve(
            input.parameters.as_ref(),
            input.working_directory.as_deref(),
        ) {
            Ok(resolved) => resolved,
            Err(error) => {
                let detail = format!("{error:#}");
                audit.record_request("action", &input.action, dry_run, false, &detail, None);
                return Err(tower_mcp::Error::tool(format!(
                    "invalid action request: {detail}"
                )));
            }
        };
        if dry_run {
            audit.record_request("action", &input.action, true, true, "dry run", None);
            return Ok(CallToolResult::json(json!({
                "dry_run": true,
                "resolved": resolved,
            })));
        }
        let run_id = runtime.runs.start_action(resolved).map_err(|error| {
            audit.record_request("action", &input.action, false, false, &error, None);
            tower_mcp::Error::tool(error)
        })?;
        audit.record_request(
            "action",
            &input.action,
            false,
            true,
            "started",
            Some(&run_id),
        );
        let wait = Duration::from_millis(input.wait_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS));
        let snapshot = runtime
            .runs
            .wait(&run_id, wait)
            .await
            .ok_or_else(|| tower_mcp::Error::tool("run disappeared"))?;
        Ok(CallToolResult::json(serde_json::to_value(snapshot)?))
    }

    async fn run_pipeline(
        &self,
        input: RunPipelineInput,
    ) -> Result<CallToolResult, tower_mcp::Error> {
        let runtime = self.actions_or_error()?;
        let (definitions, _) = runtime.store.current();
        let dry_run = input.dry_run.unwrap_or(false);
        let audit = runtime.runs.audit();
        let Some(pipeline) = definitions.pipelines.get(&input.pipeline) else {
            audit.record_request(
                "pipeline",
                &input.pipeline,
                dry_run,
                false,
                "unknown pipeline",
                None,
            );
            return Err(tower_mcp::Error::tool(format!(
                "Unknown gateway pipeline '{}'. Available actions and pipelines:\n{}",
                input.pipeline,
                serde_json::to_string_pretty(&definitions.catalog())?
            )));
        };
        let plan = match pipeline.plan(&definitions.actions, input.parameters.as_ref()) {
            Ok(plan) => plan,
            Err(error) => {
                let detail = format!("{error:#}");
                audit.record_request("pipeline", &input.pipeline, dry_run, false, &detail, None);
                return Err(tower_mcp::Error::tool(detail));
            }
        };
        if dry_run {
            audit.record_request("pipeline", &input.pipeline, true, true, &plan.digest, None);
            return Ok(CallToolResult::json(json!({"dry_run": true, "plan": plan})));
        }
        if let Some(expected) = input.expected_digest.as_deref()
            && expected != plan.digest
        {
            audit.record_request(
                "pipeline",
                &input.pipeline,
                false,
                false,
                "PLAN_INVALIDATED",
                None,
            );
            return Err(tower_mcp::Error::tool(format!(
                "PLAN_INVALIDATED: the current plan digest is {} (expected {expected}); re-run with dry_run=true and review the plan",
                plan.digest
            )));
        }
        let run_id = runtime.runs.start_pipeline(plan).map_err(|error| {
            audit.record_request("pipeline", &input.pipeline, false, false, &error, None);
            tower_mcp::Error::tool(error)
        })?;
        audit.record_request(
            "pipeline",
            &input.pipeline,
            false,
            true,
            "started",
            Some(&run_id),
        );
        let wait = Duration::from_millis(input.wait_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS));
        let snapshot = runtime
            .runs
            .wait(&run_id, wait)
            .await
            .ok_or_else(|| tower_mcp::Error::tool("run disappeared"))?;
        Ok(CallToolResult::json(serde_json::to_value(snapshot)?))
    }

    fn transfer_tools(self: &Arc<Self>) -> Vec<Tool> {
        let mut tools = Vec::new();
        let receive_state = self.clone();
        let receive = ToolBuilder::new("receive_file")
            .description("Save one file attached to or generated in the chat onto this machine. Pass the host-supplied native file value as `file` and optionally a relative destination `path`; the file is written below the gateway inbox only, never overwriting an existing file. Returns the inbox-relative path, the absolute path for use with other tools, the size, and SHA-256. Arbitrary URLs, absolute paths, traversal, and links are rejected.")
            .input_schema(portable_input_schema::<ReceiveInput>())
            .handler(move |input: ReceiveInput| {
                let state = receive_state.clone();
                async move {
                    let inbox = state.inbox_or_error()?;
                    let started = std::time::Instant::now();
                    match inbox.receive(&input.file, input.path.as_deref()).await {
                        Ok(received) => {
                            tracing::info!(
                                target: "lomway::transfer",
                                path = %received.path,
                                size = received.size,
                                sha256 = %received.sha256,
                                duration_ms = started.elapsed().as_millis() as u64,
                                "file received"
                            );
                            Ok(CallToolResult::json(serde_json::to_value(received)?))
                        }
                        Err(error) => {
                            tracing::warn!(target: "lomway::transfer", error = %format!("{error:#}"), "file receive refused");
                            Err(tower_mcp::Error::tool(format!("{error:#}")))
                        }
                    }
                }
            })
            .build();
        match receive.with_meta(json!({"openai/fileParams": ["file"]})) {
            Ok(tool) => tools.push(tool),
            Err(error) => {
                tracing::warn!(target: "lomway::transfer", %error, "receive_file metadata rejected")
            }
        }

        let send_state = self.clone();
        let send = ToolBuilder::new("send_file")
            .description("Return one existing small file from the gateway inbox (or a configured outbox, by 1-based index) to the client as an embedded resource. Links, traversal, and files over the configured size limit are rejected.")
            .input_schema(portable_input_schema::<SendInput>())
            .read_only()
            .handler(move |input: SendInput| {
                let state = send_state.clone();
                async move {
                    let inbox = state.inbox_or_error()?;
                    let (name, bytes, mime) = inbox
                        .read_for_send(&input.path, input.outbox)
                        .map_err(|error| tower_mcp::Error::tool(format!("{error:#}")))?;
                    let digest = transfer::hex(&<sha2::Sha256 as sha2::Digest>::digest(&bytes));
                    tracing::info!(target: "lomway::transfer", path = %input.path, size = bytes.len(), "file sent");
                    let summary = json!({"name": name, "size": bytes.len(), "sha256": format!("sha256:{digest}"), "mime_type": mime});
                    Ok(CallToolResult {
                        content: vec![
                            Content::Text {
                                text: summary.to_string(),
                                annotations: None,
                                meta: None,
                            },
                            Content::Resource {
                                resource: ResourceContent {
                                    uri: format!("lomway://file/{digest}/{}", percent_encode(&name)),
                                    mime_type: Some(mime),
                                    text: None,
                                    blob: Some(transfer::base64_encode(&bytes)),
                                    meta: None,
                                },
                                annotations: None,
                                meta: None,
                            },
                        ],
                        is_error: false,
                        structured_content: Some(summary),
                        meta: None,
                    })
                }
            })
            .build();
        tools.push(send);
        tools
    }
}

fn entry(
    name: String,
    backend: &str,
    description: String,
    via: Option<&str>,
    public: bool,
    aliases: Vec<String>,
) -> IndexEntry {
    IndexEntry {
        source: ToolSource {
            name,
            backend: backend.to_string(),
            invocation: Invocation::Internal,
            status: Status::Declared,
            manifest_seen_at: None,
            description: Some(description),
            placeholder_prefix: None,
            via: via.map(str::to_string),
            public,
        },
        metadata: Metadata {
            provider_oss: None,
            domain: Some(backend.to_string()),
            aliases,
            actions: Vec::new(),
            tool_declared: true,
        },
    }
}

fn percent_encode(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunActionInput {
    /// Registered action name (see lomway_list_actions).
    action: String,
    /// Structured parameters validated against the action's schema.
    #[serde(default)]
    parameters: Option<Map<String, Value>>,
    /// Resolve and return the concrete command without running it.
    #[serde(default)]
    dry_run: Option<bool>,
    /// Directory relative to the action's working directory, only for
    /// actions that allow it.
    #[serde(default)]
    working_directory: Option<String>,
    /// Milliseconds to wait for completion before returning a run_id
    /// (default 20000, maximum 25000).
    #[serde(default)]
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StatusInput {
    /// run_id returned by lomway_run_action or lomway_run_pipeline.
    run_id: String,
    /// Milliseconds to wait for completion (default 20000, maximum 25000).
    #[serde(default)]
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunPipelineInput {
    /// Registered pipeline name (see lomway_list_actions).
    pipeline: String,
    /// Structured parameters validated against the pipeline's schema.
    #[serde(default)]
    parameters: Option<Map<String, Value>>,
    /// Return the resolved plan and digest without running it.
    #[serde(default)]
    dry_run: Option<bool>,
    /// Digest from a dry run; the run is refused if the plan changed.
    #[serde(default)]
    expected_digest: Option<String>,
    /// Milliseconds to wait for completion before returning a run_id
    /// (default 20000, maximum 25000).
    #[serde(default)]
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReceiveInput {
    /// Native file value supplied by the MCP host.
    file: FileReference,
    /// Optional destination relative to the gateway inbox. Defaults to the
    /// host file name. Existing files are never overwritten.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SendInput {
    /// File path relative to the inbox (or to the chosen outbox).
    path: String,
    /// 1-based index of a configured outbox directory; omit for the inbox.
    #[serde(default)]
    outbox: Option<usize>,
}
