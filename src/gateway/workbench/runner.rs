//! Process execution, background run tracking, and the action audit log.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::sync::{Semaphore, watch};

use super::actions::{PipelinePlan, ResolvedCommand};

const RETAIN_COMPLETED: usize = 32;
const RETAIN_FOR: Duration = Duration::from_secs(60 * 60);

/// Environment variables passed to every action (case-insensitive on
/// Windows). Everything else, notably `LOMWAY_*` backend credentials, is
/// dropped unless the definition names it in `inherit_env`.
const BASE_ENV: [&str; 34] = [
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "TEMP",
    "TMP",
    "TMPDIR",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "COMMONPROGRAMFILES",
    "COMMONPROGRAMFILES(X86)",
    "USERNAME",
    "USER",
    "LOGNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "LANG",
    "LC_ALL",
    "TERM",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "SHELL",
];

/// Build the child environment from an explicit allowlist.
pub(crate) fn child_env(command: &ResolvedCommand) -> BTreeMap<String, String> {
    child_env_from(command, std::env::vars())
}

pub(crate) fn child_env_from(
    command: &ResolvedCommand,
    parent: impl Iterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    let wanted: Vec<String> = BASE_ENV
        .iter()
        .map(|name| name.to_string())
        .chain(command.inherit_env.iter().cloned())
        .map(|name| name.to_ascii_uppercase())
        .collect();
    let mut env = BTreeMap::new();
    for (key, value) in parent {
        let upper = key.to_ascii_uppercase();
        if upper.starts_with("LOMWAY_") {
            continue;
        }
        if wanted.contains(&upper) {
            env.insert(key, value);
        }
    }
    for (key, value) in &command.env {
        env.insert(key.clone(), value.clone());
    }
    env
}

/// Head + tail capture with a fixed memory bound.
#[derive(Debug, Default)]
struct Capture {
    limit_half: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
}

impl Capture {
    fn new(limit: usize) -> Self {
        Self {
            limit_half: (limit / 2).max(512),
            ..Self::default()
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        let room = self.limit_half.saturating_sub(self.head.len());
        let (head, rest) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(head);
        for byte in rest {
            if self.tail.len() == self.limit_half {
                self.tail.pop_front();
            }
            self.tail.push_back(*byte);
        }
    }

    fn render(&self) -> (String, bool) {
        let kept = self.head.len() + self.tail.len();
        let truncated = self.total > kept as u64;
        let mut text = String::from_utf8_lossy(&self.head).to_string();
        if truncated {
            text.push_str(&format!(
                "\n… [{} bytes omitted] …\n",
                self.total - kept as u64
            ));
        }
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        text.push_str(&String::from_utf8_lossy(&tail));
        (text, truncated)
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProcessOutcome {
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) duration_ms: u128,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) spawn_error: Option<String>,
}

impl ProcessOutcome {
    pub(crate) fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out && self.spawn_error.is_none()
    }
}

/// Spawn `command` without a shell, capture bounded output, and enforce
/// its timeout (killing the whole process tree on Windows).
pub(crate) async fn run_process(command: &ResolvedCommand, max_output: usize) -> ProcessOutcome {
    let started = Instant::now();
    let mut process = tokio::process::Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .env_clear()
        .envs(child_env(command))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ProcessOutcome {
                exit_code: None,
                timed_out: false,
                duration_ms: started.elapsed().as_millis(),
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                spawn_error: Some(format!("could not start '{}': {error}", command.program)),
            };
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read = |stream: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| async move {
        let mut capture = Capture::new(max_output);
        if let Some(mut stream) = stream {
            let mut buffer = vec![0u8; 16 * 1024];
            while let Ok(read) = stream.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                capture.push(&buffer[..read]);
            }
        }
        capture
    };
    let out_task = tokio::spawn(read(stdout.map(|s| Box::new(s) as _)));
    let err_task = tokio::spawn(read(stderr.map(|s| Box::new(s) as _)));
    let pid = child.id();
    let waited =
        tokio::time::timeout(Duration::from_secs(command.timeout_seconds), child.wait()).await;
    let (exit_code, timed_out) = match waited {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(_)) => (None, false),
        Err(_) => {
            kill_tree(pid, &mut child).await;
            (None, true)
        }
    };
    // Grandchildren may keep pipes open; do not wait forever for EOF.
    let collect = |task: tokio::task::JoinHandle<Capture>| async move {
        match tokio::time::timeout(Duration::from_secs(5), task).await {
            Ok(Ok(capture)) => capture.render(),
            _ => (String::from("[output unavailable]"), true),
        }
    };
    let (stdout, stdout_truncated) = collect(out_task).await;
    let (stderr, stderr_truncated) = collect(err_task).await;
    ProcessOutcome {
        exit_code,
        timed_out,
        duration_ms: started.elapsed().as_millis(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        spawn_error: None,
    }
}

async fn kill_tree(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(not(windows))]
    let _ = pid;
    let _ = child.kill().await;
}

// ---------------------------------------------------------------------------
// Background runs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct StepRecord {
    pub(crate) index: usize,
    pub(crate) action: String,
    pub(crate) state: RunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) outcome: Option<ProcessOutcome>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunSnapshot {
    pub(crate) run_id: String,
    pub(crate) kind: &'static str,
    pub(crate) name: String,
    pub(crate) state: RunState,
    pub(crate) started_at_unix: u64,
    pub(crate) elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_digest: Option<String>,
    pub(crate) steps: Vec<StepRecord>,
}

#[derive(Debug)]
struct RunEntry {
    snapshot: Mutex<RunSnapshot>,
    started: Instant,
    finished_at: Mutex<Option<Instant>>,
    done: watch::Sender<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct RunRegistry {
    runs: Arc<Mutex<HashMap<String, Arc<RunEntry>>>>,
    slots: Arc<Semaphore>,
    max_output: usize,
    audit: AuditLog,
}

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seed = format!(
        "{}:{}:{}",
        std::process::id(),
        nanos,
        RUN_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let digest = Sha256::digest(seed.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("run_{hex}")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl RunRegistry {
    pub(crate) fn new(max_concurrent: usize, max_output: usize, audit: AuditLog) -> Self {
        Self {
            runs: Arc::new(Mutex::new(HashMap::new())),
            slots: Arc::new(Semaphore::new(max_concurrent)),
            max_output,
            audit,
        }
    }

    pub(crate) fn audit(&self) -> &AuditLog {
        &self.audit
    }

    fn prune(&self) {
        let mut runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        runs.retain(|_, entry| {
            entry
                .finished_at
                .lock()
                .map(|f| f.is_none_or(|at| at.elapsed() < RETAIN_FOR))
                .unwrap_or(true)
        });
        let mut finished: Vec<(String, Instant)> = runs
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .finished_at
                    .lock()
                    .ok()
                    .and_then(|f| *f)
                    .map(|at| (id.clone(), at))
            })
            .collect();
        if finished.len() > RETAIN_COMPLETED {
            finished.sort_by_key(|(_, at)| *at);
            for (id, _) in finished.iter().take(finished.len() - RETAIN_COMPLETED) {
                runs.remove(id);
            }
        }
    }

    fn register(
        &self,
        kind: &'static str,
        name: &str,
        plan_digest: Option<String>,
        steps: Vec<StepRecord>,
    ) -> Arc<RunEntry> {
        self.prune();
        let (done, _) = watch::channel(false);
        let entry = Arc::new(RunEntry {
            snapshot: Mutex::new(RunSnapshot {
                run_id: new_run_id(),
                kind,
                name: name.to_string(),
                state: RunState::Running,
                started_at_unix: unix_now(),
                elapsed_ms: 0,
                plan_digest,
                steps,
            }),
            started: Instant::now(),
            finished_at: Mutex::new(None),
            done,
        });
        let id = entry.snapshot.lock().unwrap().run_id.clone();
        self.runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, entry.clone());
        entry
    }

    /// Start one action in the background. Fails fast when every slot is
    /// busy instead of queueing unbounded work.
    pub(crate) fn start_action(&self, command: ResolvedCommand) -> Result<String, String> {
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            "too many actions are running; try again after one finishes".to_string()
        })?;
        let entry = self.register(
            "action",
            &command.action,
            None,
            vec![StepRecord {
                index: 0,
                action: command.action.clone(),
                state: RunState::Running,
                outcome: None,
            }],
        );
        let run_id = entry.snapshot.lock().unwrap().run_id.clone();
        let max_output = self.max_output;
        let audit = self.audit.clone();
        let id = run_id.clone();
        tokio::spawn(async move {
            let outcome = run_process(&command, max_output).await;
            drop(permit);
            audit.record_outcome(&id, &command, &outcome);
            let state = if outcome.success() {
                RunState::Succeeded
            } else {
                RunState::Failed
            };
            finish(&entry, state, |snapshot| {
                snapshot.steps[0].state = state;
                snapshot.steps[0].outcome = Some(outcome);
            });
        });
        Ok(run_id)
    }

    /// Start a pipeline: steps run sequentially; a failing step stops the
    /// run unless it is marked `continue_on_error`.
    pub(crate) fn start_pipeline(&self, plan: PipelinePlan) -> Result<String, String> {
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            "too many actions are running; try again after one finishes".to_string()
        })?;
        let steps = plan
            .steps
            .iter()
            .map(|step| StepRecord {
                index: step.index,
                action: step.command.action.clone(),
                state: RunState::Running,
                outcome: None,
            })
            .collect();
        let entry = self.register("pipeline", &plan.pipeline, Some(plan.digest.clone()), steps);
        let run_id = entry.snapshot.lock().unwrap().run_id.clone();
        let max_output = self.max_output;
        let audit = self.audit.clone();
        let id = run_id.clone();
        tokio::spawn(async move {
            let mut overall = RunState::Succeeded;
            let mut stopped = false;
            for step in &plan.steps {
                if stopped {
                    break;
                }
                let outcome = run_process(&step.command, max_output).await;
                audit.record_outcome(&format!("{id}#{}", step.index), &step.command, &outcome);
                let state = if outcome.success() {
                    RunState::Succeeded
                } else {
                    RunState::Failed
                };
                if state == RunState::Failed {
                    overall = RunState::Failed;
                    stopped = !step.continue_on_error;
                }
                if let Ok(mut snapshot) = entry.snapshot.lock() {
                    snapshot.steps[step.index].state = state;
                    snapshot.steps[step.index].outcome = Some(outcome);
                }
            }
            drop(permit);
            finish(&entry, overall, |snapshot| {
                for step in &mut snapshot.steps {
                    if step.state == RunState::Running {
                        // Never started because an earlier step failed.
                        step.state = RunState::Failed;
                    }
                }
            });
        });
        Ok(run_id)
    }

    /// Wait up to `wait` for completion, then return a snapshot.
    pub(crate) async fn wait(&self, run_id: &str, wait: Duration) -> Option<RunSnapshot> {
        let entry = self
            .runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(run_id)
            .cloned()?;
        let mut done = entry.done.subscribe();
        if !*done.borrow() {
            let _ = tokio::time::timeout(wait, done.wait_for(|finished| *finished)).await;
        }
        let mut snapshot = entry.snapshot.lock().ok()?.clone();
        if snapshot.state == RunState::Running {
            snapshot.elapsed_ms = entry.started.elapsed().as_millis();
        }
        Some(snapshot)
    }
}

fn finish(entry: &RunEntry, state: RunState, update: impl FnOnce(&mut RunSnapshot)) {
    if let Ok(mut snapshot) = entry.snapshot.lock() {
        update(&mut snapshot);
        snapshot.state = state;
        snapshot.elapsed_ms = entry.started.elapsed().as_millis();
    }
    if let Ok(mut finished) = entry.finished_at.lock() {
        *finished = Some(Instant::now());
    }
    let _ = entry.done.send(true);
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

/// JSON Lines audit trail. Parameter values and output are never written;
/// arguments are recorded as SHA-256 digests only.
#[derive(Debug, Clone, Default)]
pub(crate) struct AuditLog {
    path: Option<Arc<PathBuf>>,
    lock: Arc<Mutex<()>>,
}

impl AuditLog {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.map(Arc::new),
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn digest(text: &str) -> String {
        let digest = Sha256::digest(text.as_bytes());
        digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
    }

    pub(crate) fn record(&self, mut event: Value) {
        if let Value::Object(map) = &mut event {
            map.insert("ts_unix".to_string(), json!(unix_now()));
        }
        tracing::info!(target: "lomway::actions", event = %event, "action audit");
        let Some(path) = &self.path else {
            return;
        };
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())
        {
            let _ = writeln!(file, "{event}");
        }
    }

    pub(crate) fn record_request(
        &self,
        kind: &str,
        name: &str,
        dry_run: bool,
        accepted: bool,
        detail: &str,
        run_id: Option<&str>,
    ) {
        self.record(json!({
            "event": "request",
            "kind": kind,
            "name": name,
            "dry_run": dry_run,
            "accepted": accepted,
            "detail": detail,
            "run_id": run_id,
        }));
    }

    pub(crate) fn record_outcome(
        &self,
        run_id: &str,
        command: &ResolvedCommand,
        outcome: &ProcessOutcome,
    ) {
        self.record(json!({
            "event": "finished",
            "run_id": run_id,
            "action": command.action,
            "program": command.program,
            "arg_sha256": command.args.iter().map(|arg| Self::digest(arg)).collect::<Vec<_>>(),
            "cwd": command.cwd,
            "exit_code": outcome.exit_code,
            "timed_out": outcome.timed_out,
            "duration_ms": outcome.duration_ms,
            "spawn_error": outcome.spawn_error.is_some(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str, args: &[&str], timeout: u64) -> ResolvedCommand {
        ResolvedCommand {
            action: "t".to_string(),
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: std::env::temp_dir().to_string_lossy().to_string(),
            timeout_seconds: timeout,
            policy: Vec::new(),
            effects: Vec::new(),
            env: BTreeMap::new(),
            inherit_env: Vec::new(),
        }
    }

    #[test]
    fn child_env_is_allowlisted_and_never_carries_lomway_secrets() {
        let mut cmd = command("x", &[], 1);
        cmd.inherit_env = vec!["GH_TOKEN".to_string()];
        cmd.env.insert("FIXED".to_string(), "1".to_string());
        let parent = vec![
            ("Path".to_string(), "p".to_string()),
            (
                "LOMWAY_BACKEND_CODING_TOOLS_TOKEN".to_string(),
                "secret".to_string(),
            ),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "secret".to_string()),
            ("GH_TOKEN".to_string(), "t".to_string()),
        ];
        let env = child_env_from(&cmd, parent.into_iter());
        assert_eq!(env.get("Path").map(String::as_str), Some("p"));
        assert_eq!(env.get("GH_TOKEN").map(String::as_str), Some("t"));
        assert_eq!(env.get("FIXED").map(String::as_str), Some("1"));
        assert!(!env.contains_key("LOMWAY_BACKEND_CODING_TOOLS_TOKEN"));
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn capture_keeps_head_and_tail_within_bound() {
        let mut capture = Capture::new(1024);
        for _ in 0..100 {
            capture.push(&[b'a'; 100]);
        }
        capture.push(b"END");
        let (text, truncated) = capture.render();
        assert!(truncated);
        assert!(text.ends_with("END"));
        assert!(text.len() < 1200);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn processes_run_without_a_shell_and_time_out() {
        let echo = run_process(&command("echo", &["a; echo b", "$(id)"], 10), 4096).await;
        assert!(echo.success());
        assert_eq!(echo.stdout.trim(), "a; echo b $(id)");
        let slow = run_process(&command("sleep", &["5"], 1), 4096).await;
        assert!(slow.timed_out && !slow.success());
        let missing = run_process(&command("definitely-not-a-program-xyz", &[], 1), 4096).await;
        assert!(missing.spawn_error.is_some());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn processes_run_without_a_shell_and_time_out() {
        let where_exe = run_process(&command("where.exe", &["where.exe"], 20), 4096).await;
        assert!(where_exe.success(), "{where_exe:?}");
        let slow = run_process(&command("ping.exe", &["-n", "30", "127.0.0.1"], 1), 4096).await;
        assert!(slow.timed_out && !slow.success());
        let missing = run_process(&command("definitely-not-a-program-xyz", &[], 1), 4096).await;
        assert!(missing.spawn_error.is_some());
    }

    #[tokio::test]
    async fn registry_tracks_runs_and_limits_concurrency() {
        let dir = tempfile::tempdir().unwrap();
        let audit_path = dir.path().join("audit.jsonl");
        let registry = RunRegistry::new(1, 4096, AuditLog::new(Some(audit_path.clone())));
        #[cfg(unix)]
        let (slow, fast) = (command("sleep", &["2"], 10), command("true", &[], 10));
        #[cfg(windows)]
        let (slow, fast) = (
            command("ping.exe", &["-n", "3", "127.0.0.1"], 20),
            command("where.exe", &["where.exe"], 20),
        );
        let first = registry.start_action(slow).unwrap();
        assert!(registry.start_action(fast.clone()).is_err());
        let snapshot = registry
            .wait(&first, Duration::from_millis(10))
            .await
            .unwrap();
        assert_eq!(snapshot.state, RunState::Running);
        let snapshot = registry
            .wait(&first, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded);
        let second = registry.start_action(fast).unwrap();
        let snapshot = registry
            .wait(&second, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded);
        assert!(
            registry
                .wait("run_missing", Duration::from_millis(1))
                .await
                .is_none()
        );
        let audit = std::fs::read_to_string(audit_path).unwrap();
        assert_eq!(audit.lines().count(), 2);
        assert!(audit.contains("arg_sha256"));
    }
}
