//! Operator-defined actions and pipelines (Workbridge `run_workspace_action`
//! and a minimal `orchestration` equivalent).
//!
//! Definitions live in one TOML file outside the repository. The file is
//! re-read whenever its size or modification time changes, so new actions
//! appear without any client tool reload: the MCP tool schemas stay fixed
//! (`action` + `parameters`) and validation happens at call time, exactly
//! like Workbridge.
//!
//! Safety rules:
//! * commands run as an argv vector, never through a shell string; a
//!   placeholder renders as exactly one argument (or, for `{name...}`, one
//!   argument per array item);
//! * definitions that would pass parameters to a shell or batch file are
//!   refused at load time;
//! * the working directory must resolve inside `actions.allowed_roots`, and a
//!   caller-supplied `working_directory` must stay inside the action's cwd;
//! * each action declares Workbridge effect names; pipelines declare an
//!   allow/deny effect policy that every step must satisfy (`EFFECT_DENIED`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::paths::{self, SafeRelativePath};
use super::schema::{ObjectSchema, is_identifier, scalar_text};

const DEFINITIONS_SCHEMA_VERSION: u32 = 1;
const DEFAULT_TIMEOUT_SECONDS: u64 = 300;
const MAX_TIMEOUT_SECONDS: u64 = 4 * 60 * 60;
const MAX_ARGS: usize = 256;
const MAX_PIPELINE_STEPS: usize = 32;

/// Workbridge action policy vocabulary (informational).
pub(crate) const POLICIES: [&str; 5] = [
    "read_only",
    "workspace_modify",
    "git_modify",
    "external_effect",
    "long_running",
];

/// Workbridge orchestration effect vocabulary.
pub(crate) const EFFECTS: [&str; 12] = [
    "workspace.read",
    "workspace.write",
    "process.execute",
    "git.read",
    "git.local_write",
    "git.commit",
    "git.push",
    "network.read",
    "network.write",
    "provider.invoke",
    "credential.use",
    "artifact.publish",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionFile {
    schema_version: u32,
    #[serde(default)]
    actions: Vec<ActionDef>,
    #[serde(default)]
    pipelines: Vec<PipelineDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionDef {
    name: String,
    description: String,
    #[serde(default)]
    policy: Vec<String>,
    #[serde(default)]
    effects: Vec<String>,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: String,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    allow_working_directory: bool,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    inherit_env: Vec<String>,
    #[serde(default)]
    parameters: ObjectSchema,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PipelineDef {
    name: String,
    description: String,
    effects: EffectPolicy,
    #[serde(default)]
    parameters: ObjectSchema,
    steps: Vec<StepDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectPolicy {
    #[serde(default)]
    pub(crate) allow: Vec<String>,
    #[serde(default)]
    pub(crate) deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepDef {
    action: String,
    #[serde(default)]
    parameters: Map<String, Value>,
    #[serde(default)]
    continue_on_error: bool,
}

/// One validated action.
#[derive(Debug, Clone)]
pub(crate) struct Action {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) policy: Vec<String>,
    pub(crate) effects: Vec<String>,
    pub(crate) program: String,
    args: Vec<ArgTemplate>,
    pub(crate) cwd: PathBuf,
    pub(crate) timeout_seconds: u64,
    pub(crate) allow_working_directory: bool,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) inherit_env: Vec<String>,
    pub(crate) parameters: ObjectSchema,
}

#[derive(Debug, Clone)]
pub(crate) struct Pipeline {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) effects: EffectPolicy,
    pub(crate) parameters: ObjectSchema,
    steps: Vec<StepDef>,
}

#[derive(Debug, Clone)]
enum ArgTemplate {
    /// Literal and `{name}` segments joined into exactly one argument.
    Joined(Vec<Segment>),
    /// `{name...}`: one argument per array item.
    Spread(String),
}

#[derive(Debug, Clone)]
enum Segment {
    Literal(String),
    Param(String),
}

/// A fully resolved command, ready to spawn.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ResolvedCommand {
    pub(crate) action: String,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) policy: Vec<String>,
    pub(crate) effects: Vec<String>,
    #[serde(skip)]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(skip)]
    pub(crate) inherit_env: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ResolvedStep {
    pub(crate) index: usize,
    pub(crate) continue_on_error: bool,
    pub(crate) command: ResolvedCommand,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct PipelinePlan {
    pub(crate) pipeline: String,
    pub(crate) effects: EffectPolicy,
    pub(crate) steps: Vec<ResolvedStep>,
    pub(crate) digest: String,
}

/// The parsed definition file.
#[derive(Debug, Clone, Default)]
pub(crate) struct Definitions {
    pub(crate) actions: BTreeMap<String, Action>,
    pub(crate) pipelines: BTreeMap<String, Pipeline>,
}

impl Definitions {
    pub(crate) fn parse(text: &str, allowed_roots: &[PathBuf]) -> Result<Self> {
        let file: DefinitionFile =
            toml::from_str(text).context("invalid action definition file")?;
        ensure!(
            file.schema_version == DEFINITIONS_SCHEMA_VERSION,
            "unsupported action definition schema_version {}",
            file.schema_version
        );
        let mut actions = BTreeMap::new();
        for def in file.actions {
            let name = def.name.clone();
            let action =
                Action::from_def(def, allowed_roots).with_context(|| format!("action '{name}'"))?;
            ensure!(
                actions.insert(name.clone(), action).is_none(),
                "duplicate action '{name}'"
            );
        }
        let mut pipelines = BTreeMap::new();
        for def in file.pipelines {
            let name = def.name.clone();
            let pipeline =
                Pipeline::from_def(def, &actions).with_context(|| format!("pipeline '{name}'"))?;
            ensure!(
                !actions.contains_key(&name) && pipelines.insert(name.clone(), pipeline).is_none(),
                "duplicate pipeline or action name '{name}'"
            );
        }
        Ok(Self { actions, pipelines })
    }

    pub(crate) fn catalog(&self) -> Value {
        json!({
            "actions": self.actions.values().map(Action::summary).collect::<Vec<_>>(),
            "pipelines": self.pipelines.values().map(Pipeline::summary).collect::<Vec<_>>(),
        })
    }
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|ch| ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || matches!(ch, '_' | '.' | '-'))
            && name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric()),
        "name must be 1-64 characters of [a-z0-9_.-] starting with a letter or digit"
    );
    Ok(())
}

fn is_shell_like(program: &str) -> bool {
    let base = Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if base.ends_with(".bat") || base.ends_with(".cmd") || base.ends_with(".ps1") {
        return true;
    }
    let stem = base.strip_suffix(".exe").unwrap_or(&base);
    matches!(
        stem,
        "cmd"
            | "powershell"
            | "pwsh"
            | "bash"
            | "sh"
            | "zsh"
            | "dash"
            | "fish"
            | "wsl"
            | "env"
            | "start"
            | "conhost"
    )
}

impl Action {
    fn from_def(def: ActionDef, allowed_roots: &[PathBuf]) -> Result<Self> {
        validate_name(&def.name)?;
        ensure!(
            !def.description.trim().is_empty(),
            "description is required"
        );
        for policy in &def.policy {
            ensure!(
                POLICIES.contains(&policy.as_str()),
                "unknown policy '{policy}'"
            );
        }
        ensure!(
            !def.effects.is_empty(),
            "effects must list at least one effect"
        );
        for effect in &def.effects {
            ensure!(
                EFFECTS.contains(&effect.as_str()),
                "unknown effect '{effect}'"
            );
        }
        ensure!(
            !def.program.trim().is_empty() && !def.program.chars().any(char::is_control),
            "program is required"
        );
        ensure!(def.args.len() <= MAX_ARGS, "too many args");
        ensure!(
            (1..=MAX_TIMEOUT_SECONDS).contains(&def.timeout_seconds),
            "timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        );
        def.parameters.check_definition("parameters")?;
        let mut args = Vec::new();
        let mut uses_params = false;
        for raw in &def.args {
            let template = parse_arg(raw)?;
            for name in template_params(&template) {
                uses_params = true;
                let property =
                    def.parameters.properties.get(name).with_context(|| {
                        format!("argument references undefined parameter '{name}'")
                    })?;
                if matches!(template, ArgTemplate::Spread(_)) {
                    ensure!(
                        property.kind == "array",
                        "`{{{name}...}}` requires an array parameter"
                    );
                } else {
                    ensure!(
                        property.kind != "array",
                        "array parameter '{name}' must be used as `{{{name}...}}`"
                    );
                }
            }
            args.push(template);
        }
        ensure!(
            !(uses_params && is_shell_like(&def.program)),
            "parameters cannot be passed to a shell, script, or batch program; call the target executable directly"
        );
        for key in def.env.keys().chain(def.inherit_env.iter()) {
            ensure!(
                !key.is_empty()
                    && key
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '(' | ')')),
                "invalid environment variable name '{key}'"
            );
            ensure!(
                !key.to_ascii_uppercase().starts_with("LOMWAY_"),
                "LOMWAY_* variables are never passed to actions"
            );
        }
        let cwd = paths::expand_absolute(&def.cwd, "cwd")?;
        let cwd = paths::canonical_dir(&cwd, "cwd")?;
        ensure!(
            allowed_roots
                .iter()
                .any(|root| paths::is_inside(&cwd, root)),
            "cwd is outside actions.allowed_roots"
        );
        Ok(Self {
            name: def.name,
            description: def.description,
            policy: def.policy,
            effects: def.effects,
            program: def.program,
            args,
            cwd,
            timeout_seconds: def.timeout_seconds,
            allow_working_directory: def.allow_working_directory,
            env: def.env,
            inherit_env: def.inherit_env,
            parameters: def.parameters,
        })
    }

    pub(crate) fn summary(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "policy": self.policy,
            "effects": self.effects,
            "timeout_seconds": self.timeout_seconds,
            "allow_working_directory": self.allow_working_directory,
            "parameters": self.parameters.to_json(),
        })
    }

    /// Validate parameters and render the command.
    pub(crate) fn resolve(
        &self,
        parameters: Option<&Map<String, Value>>,
        working_directory: Option<&str>,
    ) -> Result<ResolvedCommand> {
        let values = self.parameters.validate(parameters)?;
        let cwd = match working_directory
            .map(str::trim)
            .filter(|wd| !wd.is_empty() && *wd != ".")
        {
            None => self.cwd.clone(),
            Some(relative) => {
                ensure!(
                    self.allow_working_directory,
                    "this action does not accept working_directory"
                );
                let rel = SafeRelativePath::parse(relative)
                    .map_err(|error| anyhow!("working_directory: {error}"))?;
                let joined = rel.join_to(&self.cwd);
                let canonical = paths::canonical_dir(&joined, "working_directory")?;
                ensure!(
                    paths::is_inside(&canonical, &self.cwd),
                    "working_directory escapes the action directory"
                );
                canonical
            }
        };
        for (name, property) in &self.parameters.properties {
            let texts: Vec<&str> = match values.get(name) {
                Some(Value::String(text)) => vec![text.as_str()],
                Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            let is_path = property.format.as_deref() == Some("relative-path")
                || property
                    .items
                    .as_ref()
                    .is_some_and(|items| items.format.as_deref() == Some("relative-path"));
            if !is_path {
                continue;
            }
            for text in texts {
                let joined = SafeRelativePath::parse(text)?.join_to(&cwd);
                if let Ok(canonical) = std::fs::canonicalize(&joined) {
                    ensure!(
                        paths::is_inside(&paths::strip_verbatim(canonical), &cwd),
                        "parameter '{name}' resolves outside the working directory"
                    );
                }
            }
        }
        let mut args = Vec::new();
        for template in &self.args {
            match template {
                ArgTemplate::Spread(name) => {
                    if let Some(Value::Array(items)) = values.get(name) {
                        args.extend(items.iter().filter_map(scalar_text));
                    }
                }
                ArgTemplate::Joined(segments) => {
                    let mut text = String::new();
                    let mut missing = false;
                    for segment in segments {
                        match segment {
                            Segment::Literal(literal) => text.push_str(literal),
                            Segment::Param(name) => match values.get(name).and_then(scalar_text) {
                                Some(value) => text.push_str(&value),
                                None => missing = true,
                            },
                        }
                    }
                    if !missing {
                        args.push(text);
                    }
                }
            }
        }
        Ok(ResolvedCommand {
            action: self.name.clone(),
            program: self.program.clone(),
            args,
            cwd: cwd.to_string_lossy().to_string(),
            timeout_seconds: self.timeout_seconds,
            policy: self.policy.clone(),
            effects: self.effects.clone(),
            env: self.env.clone(),
            inherit_env: self.inherit_env.clone(),
        })
    }
}

fn parse_arg(raw: &str) -> Result<ArgTemplate> {
    ensure!(!raw.contains('\0'), "arguments must not contain NUL");
    if let Some(inner) = raw
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix("...}"))
    {
        ensure!(is_identifier(inner), "invalid placeholder '{raw}'");
        return Ok(ArgTemplate::Spread(inner.to_string()));
    }
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => name.push(c),
                        None => bail!("unterminated placeholder in '{raw}'"),
                    }
                }
                ensure!(
                    is_identifier(&name),
                    "invalid placeholder '{{{name}}}' in '{raw}'"
                );
                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }
                segments.push(Segment::Param(name));
            }
            '}' => bail!("unmatched '}}' in '{raw}' (use '}}}}' for a literal brace)"),
            other => literal.push(other),
        }
    }
    if !literal.is_empty() || segments.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    Ok(ArgTemplate::Joined(segments))
}

fn template_params(template: &ArgTemplate) -> Vec<&str> {
    match template {
        ArgTemplate::Spread(name) => vec![name.as_str()],
        ArgTemplate::Joined(segments) => segments
            .iter()
            .filter_map(|segment| match segment {
                Segment::Param(name) => Some(name.as_str()),
                Segment::Literal(_) => None,
            })
            .collect(),
    }
}

/// Workbridge `assertBackendEffectsAllowed`: every required effect must be
/// allowed and none denied. Returns the rejected effects.
pub(crate) fn check_effects(required: &[String], policy: &EffectPolicy) -> Vec<String> {
    required
        .iter()
        .filter(|effect| policy.deny.contains(effect) || !policy.allow.contains(effect))
        .cloned()
        .collect()
}

impl Pipeline {
    fn from_def(def: PipelineDef, actions: &BTreeMap<String, Action>) -> Result<Self> {
        validate_name(&def.name)?;
        ensure!(
            !def.description.trim().is_empty(),
            "description is required"
        );
        for effect in def.effects.allow.iter().chain(&def.effects.deny) {
            ensure!(
                EFFECTS.contains(&effect.as_str()),
                "unknown effect '{effect}'"
            );
        }
        ensure!(
            !def.steps.is_empty() && def.steps.len() <= MAX_PIPELINE_STEPS,
            "a pipeline needs 1 to {MAX_PIPELINE_STEPS} steps"
        );
        def.parameters.check_definition("parameters")?;
        for (index, step) in def.steps.iter().enumerate() {
            let action = actions.get(&step.action).with_context(|| {
                format!("step {index} references unknown action '{}'", step.action)
            })?;
            let rejected = check_effects(&action.effects, &def.effects);
            if !rejected.is_empty() {
                bail!(
                    "EFFECT_DENIED: step {index} action '{}' requires effects outside the pipeline policy: {}",
                    step.action,
                    rejected.join(", ")
                );
            }
            for key in step.parameters.keys() {
                ensure!(
                    action.parameters.properties.contains_key(key),
                    "step {index} passes unknown parameter '{key}' to '{}'",
                    step.action
                );
            }
            for value in step.parameters.values() {
                for name in referenced_params(value) {
                    ensure!(
                        def.parameters.properties.contains_key(&name),
                        "step {index} references undefined pipeline parameter '{name}'"
                    );
                }
            }
        }
        Ok(Self {
            name: def.name,
            description: def.description,
            effects: def.effects,
            parameters: def.parameters,
            steps: def.steps,
        })
    }

    pub(crate) fn summary(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "effects": self.effects,
            "steps": self.steps.iter().map(|step| step.action.clone()).collect::<Vec<_>>(),
            "parameters": self.parameters.to_json(),
        })
    }

    /// Validate pipeline parameters, render every step, re-check the effect
    /// guard, and compute the plan digest.
    pub(crate) fn plan(
        &self,
        actions: &BTreeMap<String, Action>,
        parameters: Option<&Map<String, Value>>,
    ) -> Result<PipelinePlan> {
        let values = self.parameters.validate(parameters)?;
        let mut steps = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            let action = actions.get(&step.action).with_context(|| {
                format!(
                    "PLAN_INVALIDATED: action '{}' no longer exists",
                    step.action
                )
            })?;
            let rejected = check_effects(&action.effects, &self.effects);
            if !rejected.is_empty() {
                bail!(
                    "EFFECT_DENIED: step {index} action '{}' requires {}",
                    step.action,
                    rejected.join(", ")
                );
            }
            let mut rendered = Map::new();
            for (key, value) in &step.parameters {
                if let Some(value) = substitute(value, &values) {
                    rendered.insert(key.clone(), value);
                }
            }
            let command = action
                .resolve(Some(&rendered), None)
                .with_context(|| format!("step {index} ('{}')", step.action))?;
            steps.push(ResolvedStep {
                index,
                continue_on_error: step.continue_on_error,
                command,
            });
        }
        let digest_input = json!({
            "schema": "lomway.pipeline.plan/v1",
            "pipeline": self.name,
            "effects": self.effects,
            "steps": steps,
        });
        Ok(PipelinePlan {
            pipeline: self.name.clone(),
            effects: self.effects.clone(),
            steps,
            digest: format!("sha256:{}", canonical_sha256(&digest_input)),
        })
    }
}

fn referenced_params(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => match parse_arg(text) {
            Ok(template) => template_params(&template)
                .into_iter()
                .map(str::to_string)
                .collect(),
            Err(_) => Vec::new(),
        },
        Value::Array(items) => items.iter().flat_map(referenced_params).collect(),
        _ => Vec::new(),
    }
}

/// `"{name}"` alone substitutes the raw value (any type); placeholders inside
/// a longer string interpolate scalar text. A value that references a
/// missing optional parameter is dropped.
fn substitute(value: &Value, values: &Map<String, Value>) -> Option<Value> {
    match value {
        Value::String(text) => {
            let Ok(template) = parse_arg(text) else {
                return Some(value.clone());
            };
            match template {
                ArgTemplate::Spread(name) => values.get(&name).cloned(),
                ArgTemplate::Joined(segments) => {
                    if let [Segment::Param(name)] = segments.as_slice() {
                        return values.get(name).cloned();
                    }
                    let mut out = String::new();
                    for segment in segments {
                        match segment {
                            Segment::Literal(literal) => out.push_str(&literal),
                            Segment::Param(name) => out.push_str(&scalar_text(values.get(&name)?)?),
                        }
                    }
                    Some(Value::String(out))
                }
            }
        }
        Value::Array(items) => Some(Value::Array(
            items
                .iter()
                .filter_map(|item| substitute(item, values))
                .collect(),
        )),
        other => Some(other.clone()),
    }
}

/// SHA-256 over canonical JSON (sorted keys, no whitespace), independent of
/// serde_json's map ordering feature.
pub(crate) fn canonical_sha256(value: &Value) -> String {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                let sorted: BTreeSet<&String> = map.keys().collect();
                out.push('{');
                for (i, key) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut text = String::new();
    write(value, &mut text);
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The definition file, re-read when its size or mtime changes. A broken
/// edit keeps the last good definitions and surfaces the error.
#[derive(Debug, Clone)]
pub(crate) struct DefinitionStore {
    path: PathBuf,
    allowed_roots: Arc<Vec<PathBuf>>,
    state: Arc<Mutex<StoreState>>,
}

type FileStamp = (u64, Option<SystemTime>);

#[derive(Debug, Default)]
struct StoreState {
    stamp: Option<FileStamp>,
    current: Arc<Definitions>,
    error: Option<String>,
}

impl DefinitionStore {
    pub(crate) fn new(path: PathBuf, allowed_roots: Vec<PathBuf>) -> Self {
        let store = Self {
            path,
            allowed_roots: Arc::new(allowed_roots),
            state: Arc::new(Mutex::new(StoreState::default())),
        };
        let _ = store.current();
        store
    }

    /// Current definitions plus the last load error, if any.
    pub(crate) fn current(&self) -> (Arc<Definitions>, Option<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let stamp = std::fs::metadata(&self.path)
            .ok()
            .map(|metadata| (metadata.len(), metadata.modified().ok()));
        if stamp.is_none() {
            state.error = Some("action definition file is missing or unreadable".to_string());
            state.stamp = None;
            return (state.current.clone(), state.error.clone());
        }
        if stamp != state.stamp {
            state.stamp = stamp;
            match std::fs::read_to_string(&self.path)
                .context("cannot read action definition file")
                .and_then(|text| Definitions::parse(&text, &self.allowed_roots))
            {
                Ok(definitions) => {
                    state.current = Arc::new(definitions);
                    state.error = None;
                }
                Err(error) => state.error = Some(format!("{error:#}")),
            }
        }
        (state.current.clone(), state.error.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> Vec<PathBuf> {
        vec![paths::canonical_dir(dir, "root").unwrap()]
    }

    fn defs(dir: &Path, body: &str) -> Result<Definitions> {
        let cwd = dir.to_string_lossy().replace('\\', "/");
        let text = format!("schema_version = 1\n{}", body.replace("@CWD@", &cwd));
        Definitions::parse(&text, &roots(dir))
    }

    const ECHO: &str = r#"
[[actions]]
name = "echo"
description = "Echo arguments."
policy = ["read_only"]
effects = ["process.execute"]
program = "git"
args = ["log", "--format={fmt}", "{files...}", "{{literal}}", "--", "{rev}"]
cwd = "@CWD@"
[actions.parameters]
required = ["fmt"]
[actions.parameters.properties.fmt]
type = "string"
[actions.parameters.properties.rev]
type = "string"
[actions.parameters.properties.files]
type = "array"
items = { type = "string", format = "relative-path" }
"#;

    #[test]
    fn placeholders_render_as_single_arguments_without_shell_splitting() {
        let dir = tempfile::tempdir().unwrap();
        let defs = defs(dir.path(), ECHO).unwrap();
        let action = &defs.actions["echo"];
        let params = json!({"fmt": "%H; rm -rf / && echo $(x) | y", "files": ["a b.txt", "c"]});
        let resolved = action.resolve(params.as_object(), None).unwrap();
        assert_eq!(
            resolved.args,
            vec![
                "log",
                "--format=%H; rm -rf / && echo $(x) | y",
                "a b.txt",
                "c",
                "{literal}",
                "--"
            ]
        );
        // Optional parameter present: its argument is kept.
        let params = json!({"fmt": "x", "rev": "HEAD"});
        let resolved = action.resolve(params.as_object(), None).unwrap();
        assert_eq!(resolved.args.last().unwrap(), "HEAD");
        for bad in [
            json!({}),
            json!({"fmt": "x", "rev": "--upload-pack=evil"}),
            json!({"fmt": "x", "files": ["../../etc/passwd"]}),
            json!({"fmt": "x", "extra": 1}),
        ] {
            assert!(action.resolve(bad.as_object(), None).is_err(), "{bad}");
        }
    }

    #[test]
    fn definitions_reject_shells_unknown_effects_and_escaping_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let shell = ECHO.replace("program = \"git\"", "program = \"cmd.exe\"");
        assert!(format!("{:#}", defs(dir.path(), &shell).unwrap_err()).contains("shell"));
        let shell_static = r#"
[[actions]]
name = "static"
description = "Static script is fine."
effects = ["process.execute"]
program = "powershell"
args = ["-NoProfile", "-File", "x.ps1"]
cwd = "@CWD@"
"#;
        assert!(defs(dir.path(), shell_static).is_ok());
        let bad_effect = ECHO.replace("process.execute", "rocket.launch");
        assert!(defs(dir.path(), &bad_effect).is_err());
        let outside = tempfile::tempdir().unwrap();
        let escaping = ECHO.replace(
            "@CWD@",
            &outside.path().to_string_lossy().replace('\\', "/"),
        );
        assert!(defs(dir.path(), &escaping).is_err());
        let undefined = ECHO.replace("{rev}", "{nope}");
        assert!(defs(dir.path(), &undefined).is_err());
        let env_bad = ECHO.replace(
            "cwd = \"@CWD@\"",
            "cwd = \"@CWD@\"\ninherit_env = [\"LOMWAY_BACKEND_CODING_TOOLS_TOKEN\"]",
        );
        assert!(defs(dir.path(), &env_bad).is_err());
    }

    #[test]
    fn working_directory_is_confined_and_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub/inner")).unwrap();
        let body = ECHO.replace(
            "cwd = \"@CWD@\"",
            "cwd = \"@CWD@\"\nallow_working_directory = true",
        );
        let defs = defs(dir.path(), &body).unwrap();
        let action = &defs.actions["echo"];
        let params = json!({"fmt": "x"});
        let resolved = action
            .resolve(params.as_object(), Some("sub/inner"))
            .unwrap();
        assert!(resolved.cwd.ends_with("inner"));
        assert!(action.resolve(params.as_object(), Some("../")).is_err());
        assert!(action.resolve(params.as_object(), Some("missing")).is_err());
        let strict = self::defs(dir.path(), ECHO).unwrap();
        assert!(
            strict.actions["echo"]
                .resolve(params.as_object(), Some("sub"))
                .is_err()
        );
    }

    #[test]
    fn pipelines_enforce_effect_guard_and_digest_tracks_plan() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = format!(
            "{ECHO}\n{}",
            r#"
[[pipelines]]
name = "two"
description = "Two steps."
effects = { allow = ["process.execute"], deny = ["git.push"] }
[pipelines.parameters.properties.format]
type = "string"
default = "%H"
[[pipelines.steps]]
action = "echo"
parameters = { fmt = "{format}" }
[[pipelines.steps]]
action = "echo"
parameters = { fmt = "pre-{format}", rev = "HEAD" }
continue_on_error = true
"#
        );
        let defs = defs(dir.path(), &pipeline).unwrap();
        let plan = defs.pipelines["two"].plan(&defs.actions, None).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].command.args[1], "--format=%H");
        assert_eq!(plan.steps[1].command.args[1], "--format=pre-%H");
        let again = defs.pipelines["two"].plan(&defs.actions, None).unwrap();
        assert_eq!(plan.digest, again.digest);
        let other = defs.pipelines["two"]
            .plan(&defs.actions, json!({"format": "%h"}).as_object())
            .unwrap();
        assert_ne!(plan.digest, other.digest);

        let denied = pipeline.replace(
            "allow = [\"process.execute\"]",
            "allow = [\"workspace.read\"]",
        );
        let error = self::defs(dir.path(), &denied).unwrap_err();
        assert!(format!("{error:#}").contains("EFFECT_DENIED"), "{error:#}");
        let deny_hit = pipeline.replace("deny = [\"git.push\"]", "deny = [\"process.execute\"]");
        assert!(
            format!("{:#}", self::defs(dir.path(), &deny_hit).unwrap_err())
                .contains("EFFECT_DENIED")
        );
        let unknown_param = pipeline.replace("{format}\" }", "{nope}\" }");
        assert!(self::defs(dir.path(), &unknown_param).is_err());
    }

    #[test]
    fn store_reloads_on_change_and_keeps_last_good() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("actions.toml");
        let cwd = dir.path().to_string_lossy().replace('\\', "/");
        let text = format!("schema_version = 1\n{}", ECHO.replace("@CWD@", &cwd));
        std::fs::write(&file, &text).unwrap();
        let store = DefinitionStore::new(file.clone(), roots(dir.path()));
        assert!(store.current().0.actions.contains_key("echo"));
        std::fs::write(&file, format!("{text}\nbroken = [")).unwrap();
        let (defs, error) = store.current();
        assert!(defs.actions.contains_key("echo"));
        assert!(error.is_some());
        let second = text.replace("name = \"echo\"", "name = \"echo2\"");
        std::fs::write(&file, format!("{second}\n")).unwrap();
        let (defs, error) = store.current();
        assert!(defs.actions.contains_key("echo2") && error.is_none());
    }

    #[test]
    fn shipped_example_definitions_are_valid() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().replace('\\', "/");
        let text = include_str!("../../../examples/actions.example.toml")
            .replace("${USERPROFILE}/src", &cwd);
        let defs = Definitions::parse(&text, &roots(dir.path())).unwrap();
        assert!(defs.actions.contains_key("workspace_review.status"));
        let plan = defs.pipelines["rust_verify"]
            .plan(&defs.actions, json!({"repo": "lomway"}).as_object())
            .unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert!(
            !plan.steps[1]
                .command
                .args
                .iter()
                .any(|a| a.starts_with("--package"))
        );
    }

    #[test]
    fn canonical_digest_ignores_key_order() {
        let a = json!({"b": 1, "a": [1, {"y": 2, "x": 1}]});
        let b = json!({"a": [1, {"x": 1, "y": 2}], "b": 1});
        assert_eq!(canonical_sha256(&a), canonical_sha256(&b));
    }
}
