//! Agent skill discovery compatible with Workbridge (pi-coding-agent rules).
//!
//! Scan order matches Workbridge's `effectiveSkillPaths`: `~/.agents/skills`,
//! each project's `.agents/skills`, the devspace skills directory
//! (`DEVSPACE_CONFIG_DIR`/skills or `~/.devspace/skills`), `<agent_dir>/skills`,
//! then configured `paths`. Discovery rules per directory:
//!
//! * a directory containing `SKILL.md` is a skill root; recursion stops;
//! * `*.md` files directly in a scanned root are skills too;
//! * entries starting with `.` and `node_modules` are skipped;
//! * symlinks and junctions are followed; the same file reached twice is
//!   loaded once (canonical path), and the first skill of a name wins.
//!
//! Frontmatter: `name` (defaults to the parent directory name), a required
//! `description`, and `disable-model-invocation`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::paths;
use crate::config::model::SkillsConfig;

const MAX_DEPTH: usize = 8;
const MAX_SKILL_FILE_BYTES: u64 = 512 * 1024;
const MAX_SKILLS: usize = 500;
const MAX_DESCRIPTION_CHARS: usize = 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct Skill {
    pub(crate) name: String,
    pub(crate) description: String,
    /// Absolute path of the skill file as discovered (may go through a link).
    pub(crate) path: String,
    /// Canonical path of the skill file.
    pub(crate) canonical_path: String,
    /// `user`, `project`, or `path`.
    pub(crate) source: String,
    pub(crate) disable_model_invocation: bool,
    /// Path relative to the configured read root, when the canonical file
    /// lies inside it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) read_root_relative: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct ScanResult {
    pub(crate) skills: Vec<Skill>,
    pub(crate) diagnostics: Vec<String>,
}

/// Resolved scan roots for a configuration.
#[derive(Debug, Clone)]
pub(crate) struct ScanPlan {
    roots: Vec<(PathBuf, &'static str)>,
    read_root: Option<PathBuf>,
}

impl ScanPlan {
    pub(crate) fn from_config(config: &SkillsConfig) -> (Self, Vec<String>) {
        let mut diagnostics = Vec::new();
        let mut roots: Vec<(PathBuf, &'static str)> = Vec::new();
        let mut push =
            |raw: &str, source: &'static str, diagnostics: &mut Vec<String>| match paths::expand(
                raw,
            ) {
                Ok(path) => roots.push((path, source)),
                Err(error) => diagnostics.push(format!("skill path skipped: {error}")),
            };
        if config.default_paths {
            push("~/.agents/skills", "user", &mut diagnostics);
        }
        for project in &config.project_dirs {
            match paths::expand(project) {
                Ok(path) => push(
                    &path.join(".agents").join("skills").to_string_lossy(),
                    "project",
                    &mut diagnostics,
                ),
                Err(error) => diagnostics.push(format!("project dir skipped: {error}")),
            }
        }
        if config.default_paths {
            match std::env::var("DEVSPACE_CONFIG_DIR") {
                Ok(dir) if !dir.trim().is_empty() => {
                    push(&format!("{dir}/skills"), "user", &mut diagnostics)
                }
                _ => push("~/.devspace/skills", "user", &mut diagnostics),
            }
            push(
                &format!("{}/skills", config.agent_dir),
                "user",
                &mut diagnostics,
            );
        }
        for extra in &config.paths {
            push(extra, "path", &mut diagnostics);
        }
        let read_root = config.read_root.as_deref().and_then(|raw| {
            match paths::expand(raw)
                .and_then(|path| paths::canonical_dir(&path, "skills.read_root"))
            {
                Ok(path) => Some(path),
                Err(error) => {
                    diagnostics.push(format!("skills.read_root ignored: {error}"));
                    None
                }
            }
        });
        (Self { roots, read_root }, diagnostics)
    }

    #[cfg(test)]
    pub(crate) fn for_roots(
        roots: Vec<(PathBuf, &'static str)>,
        read_root: Option<PathBuf>,
    ) -> Self {
        Self { roots, read_root }
    }

    pub(crate) fn scan(&self) -> ScanResult {
        let mut state = ScanState::default();
        for (root, source) in &self.roots {
            if !root.exists() {
                continue;
            }
            if root.is_file() {
                if root.extension().is_some_and(|ext| ext == "md") {
                    state.load(root, source);
                } else {
                    state.diagnostics.push(format!(
                        "skill path is not a markdown file: {}",
                        root.display()
                    ));
                }
                continue;
            }
            state.walk(root, source, true, 0);
        }
        let mut skills = state.skills;
        if let Some(read_root) = &self.read_root {
            for skill in &mut skills {
                let canonical = Path::new(&skill.canonical_path);
                if paths::is_inside(canonical, read_root)
                    && let Some(relative) = relative_to(canonical, read_root)
                {
                    skill.read_root_relative = Some(relative);
                }
            }
        }
        ScanResult {
            skills,
            diagnostics: state.diagnostics,
        }
    }
}

/// `path` relative to `root` with `/` separators (case-insensitive prefix on
/// Windows, matching [`paths::is_inside`]).
fn relative_to(path: &Path, root: &Path) -> Option<String> {
    let path_text = path.to_string_lossy();
    let root_text = root.to_string_lossy();
    let root_len = root_text.trim_end_matches(['/', '\\']).len();
    let rest = path_text.get(root_len..)?;
    let rest = rest.trim_start_matches(['/', '\\']);
    (!rest.is_empty()).then(|| rest.replace('\\', "/"))
}

#[derive(Default)]
struct ScanState {
    skills: Vec<Skill>,
    diagnostics: Vec<String>,
    seen_files: HashSet<String>,
    seen_names: HashSet<String>,
}

impl ScanState {
    fn walk(&mut self, dir: &Path, source: &'static str, include_root_files: bool, depth: usize) {
        if depth > MAX_DEPTH || self.skills.len() >= MAX_SKILLS {
            return;
        }
        let skill_file = dir.join("SKILL.md");
        if skill_file.is_file() {
            self.load(&skill_file, source);
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            // `metadata` follows links and junctions, like Workbridge.
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                self.walk(&path, source, false, depth + 1);
            } else if metadata.is_file() && include_root_files && name.ends_with(".md") {
                self.load(&path, source);
            }
        }
    }

    fn load(&mut self, file: &Path, source: &'static str) {
        let canonical = match std::fs::canonicalize(file) {
            Ok(path) => paths::strip_verbatim(path),
            Err(_) => return,
        };
        let canonical_text = canonical.to_string_lossy().to_string();
        if !self.seen_files.insert(canonical_text.clone()) {
            return;
        }
        match std::fs::metadata(&canonical) {
            Ok(metadata) if metadata.len() <= MAX_SKILL_FILE_BYTES => {}
            _ => {
                self.diagnostics.push(format!(
                    "skill file too large or unreadable: {}",
                    file.display()
                ));
                return;
            }
        }
        let Ok(text) = std::fs::read_to_string(&canonical) else {
            self.diagnostics
                .push(format!("skill file is not UTF-8: {}", file.display()));
            return;
        };
        let front = parse_frontmatter(&text);
        let Some(description) = front.description.filter(|d| !d.trim().is_empty()) else {
            self.diagnostics
                .push(format!("skill has no description: {}", file.display()));
            return;
        };
        let name = front
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                file.parent()
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        if name.is_empty() {
            return;
        }
        if !self.seen_names.insert(name.clone()) {
            self.diagnostics.push(format!(
                "skill name \"{name}\" collision: {}",
                file.display()
            ));
            return;
        }
        let description: String = description
            .trim()
            .chars()
            .take(MAX_DESCRIPTION_CHARS)
            .collect();
        self.skills.push(Skill {
            name,
            description,
            path: paths::strip_verbatim(file.to_path_buf())
                .to_string_lossy()
                .to_string(),
            canonical_path: canonical_text,
            source: source.to_string(),
            disable_model_invocation: front.disable_model_invocation,
            read_root_relative: None,
        });
    }
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Frontmatter {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) disable_model_invocation: bool,
}

/// Minimal YAML frontmatter reader for the keys skills use: plain, single-
/// or double-quoted scalars and `>`/`|` block scalars.
pub(crate) fn parse_frontmatter(text: &str) -> Frontmatter {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Frontmatter::default();
    }
    let body: Vec<&str> = lines.take_while(|line| line.trim_end() != "---").collect();
    let mut front = Frontmatter::default();
    let mut index = 0;
    while index < body.len() {
        let line = body[index];
        index += 1;
        if line.starts_with([' ', '\t']) || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let parsed = if value.starts_with('>') || value.starts_with('|') {
            let folded = value.starts_with('>');
            let mut block = Vec::new();
            while index < body.len()
                && (body[index].starts_with([' ', '\t']) || body[index].trim().is_empty())
            {
                block.push(body[index].trim());
                index += 1;
            }
            while block.last().is_some_and(|line| line.is_empty()) {
                block.pop();
            }
            if folded {
                block.join(" ")
            } else {
                block.join("\n")
            }
        } else {
            let mut scalar = unquote(value);
            // Plain multi-line scalars continue on indented lines.
            while index < body.len()
                && body[index].starts_with([' ', '\t'])
                && !value.starts_with(['"', '\''])
            {
                scalar.push(' ');
                scalar.push_str(body[index].trim());
                index += 1;
            }
            scalar
        };
        match key {
            "name" => front.name = Some(parsed),
            "description" => front.description = Some(parsed),
            "disable-model-invocation" => {
                front.disable_model_invocation = parsed.eq_ignore_ascii_case("true");
            }
            _ => {}
        }
    }
    front
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\\\", "\\");
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    // Strip a trailing comment on plain scalars.
    match value.find(" #") {
        Some(position) => value[..position].trim_end().to_string(),
        None => value.to_string(),
    }
}

/// A scan cache that re-scans at most once per `interval` when read.
#[derive(Debug, Clone)]
pub(crate) struct SkillRegistry {
    plan: Arc<ScanPlan>,
    interval: Duration,
    state: Arc<Mutex<(Instant, Arc<ScanResult>)>>,
    plan_diagnostics: Arc<Vec<String>>,
}

impl SkillRegistry {
    pub(crate) fn new(config: &SkillsConfig) -> Self {
        let (plan, plan_diagnostics) = ScanPlan::from_config(config);
        Self::with_plan(
            plan,
            plan_diagnostics,
            Duration::from_secs(config.rescan_seconds),
        )
    }

    pub(crate) fn with_plan(
        plan: ScanPlan,
        plan_diagnostics: Vec<String>,
        interval: Duration,
    ) -> Self {
        let result = plan.scan();
        Self {
            plan: Arc::new(plan),
            interval,
            state: Arc::new(Mutex::new((Instant::now(), Arc::new(result)))),
            plan_diagnostics: Arc::new(plan_diagnostics),
        }
    }

    /// Current skills, re-scanning when the cached scan is older than the
    /// configured interval.
    pub(crate) fn current(&self) -> Arc<ScanResult> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if guard.0.elapsed() >= self.interval {
            *guard = (Instant::now(), Arc::new(self.plan.scan()));
        }
        guard.1.clone()
    }

    pub(crate) fn plan_diagnostics(&self) -> &[String] {
        &self.plan_diagnostics
    }

    pub(crate) fn find(&self, name: &str) -> Option<Skill> {
        self.current()
            .skills
            .iter()
            .find(|skill| skill.name == name)
            .cloned()
    }
}

/// How to read a skill file through the file/exec backend.
pub(crate) fn read_hint(skill: &Skill) -> String {
    match &skill.read_root_relative {
        Some(relative) => format!(
            "Read it with coding_tools_read_file path=\"{relative}\" (relative to the Coding Tools workspace). Resolve relative references in the skill against its directory."
        ),
        None => format!(
            "The file is outside the Coding Tools workspace: read it with coding_tools_exec_command, for example `powershell -NoProfile -Command \"Get-Content -Raw -LiteralPath '{}'\"`. Resolve relative references in the skill against its directory.",
            skill.path.replace('\'', "''")
        ),
    }
}

/// Short list appended to the `initialize` instructions.
pub(crate) fn instructions_block(result: &ScanResult, limit: usize) -> Option<String> {
    let visible: Vec<&Skill> = result
        .skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .collect();
    if visible.is_empty() || limit == 0 {
        return None;
    }
    let mut text = String::from(
        "Agent skills are available on this machine. When a task matches a skill, read its SKILL.md before acting (lomway_describe_tool name=\"skill:<name>\" returns the path and how to read it). Skills:",
    );
    for skill in visible.iter().take(limit) {
        let short: String = skill.description.chars().take(110).collect();
        let ellipsis = if skill.description.chars().count() > 110 {
            "…"
        } else {
            ""
        };
        text.push_str(&format!("\n- {}: {short}{ellipsis}", skill.name));
    }
    if visible.len() > limit {
        text.push_str(&format!(
            "\n(+{} more: search with lomway_search_tools query=\"skill <topic>\")",
            visible.len() - limit
        ));
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn frontmatter_handles_quotes_blocks_and_flags() {
        let front = parse_frontmatter(
            "---\nname: my-skill\ndescription: \"Use when: \\\"x\\\"\"\ndisable-model-invocation: true\n---\nbody",
        );
        assert_eq!(front.name.as_deref(), Some("my-skill"));
        assert_eq!(front.description.as_deref(), Some("Use when: \"x\""));
        assert!(front.disable_model_invocation);
        let folded = parse_frontmatter(
            "---\ndescription: >\n  first line\n  second line\nname: 'it''s'\n---\n",
        );
        assert_eq!(
            folded.description.as_deref(),
            Some("first line second line")
        );
        assert_eq!(folded.name.as_deref(), Some("it's"));
        let plain = parse_frontmatter("---\ndescription: one\n  two # c\n---\n");
        assert_eq!(plain.description.as_deref(), Some("one two # c"));
        assert_eq!(parse_frontmatter("no frontmatter"), Frontmatter::default());
        let bom = parse_frontmatter("\u{feff}---\r\ndescription: d\r\n---\r\n");
        assert_eq!(bom.description.as_deref(), Some("d"));
    }

    #[test]
    fn discovery_follows_workbridge_rules() {
        let temp = tempfile::tempdir().unwrap();
        let user = temp.path().join("user");
        write(
            &user.join("alpha/SKILL.md"),
            "---\ndescription: Alpha skill\n---\n",
        );
        // Recursion stops at a skill root.
        write(
            &user.join("alpha/nested/SKILL.md"),
            "---\nname: hidden\ndescription: no\n---\n",
        );
        write(
            &user.join("group/beta/SKILL.md"),
            "---\nname: beta\ndescription: Beta\n---\n",
        );
        write(
            &user.join("root-file.md"),
            "---\nname: rooted\ndescription: Root md\n---\n",
        );
        write(
            &user.join("group/not-root.md"),
            "---\nname: nope\ndescription: x\n---\n",
        );
        write(
            &user.join(".hidden/SKILL.md"),
            "---\nname: dot\ndescription: x\n---\n",
        );
        write(
            &user.join("node_modules/x/SKILL.md"),
            "---\nname: nm\ndescription: x\n---\n",
        );
        write(&user.join("nodesc/SKILL.md"), "---\nname: nodesc\n---\n");
        let extra = temp.path().join("extra");
        write(
            &extra.join("alpha/SKILL.md"),
            "---\ndescription: duplicate name\n---\n",
        );
        write(
            &extra.join("gamma/SKILL.md"),
            "---\ndescription: Gamma\n---\n",
        );
        let plan = ScanPlan::for_roots(
            vec![
                (user.clone(), "user"),
                (extra, "path"),
                (temp.path().join("missing"), "path"),
            ],
            Some(paths::canonical_dir(temp.path(), "root").unwrap()),
        );
        let result = plan.scan();
        let names: Vec<&str> = result.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "rooted", "gamma"]);
        let alpha = &result.skills[0];
        assert_eq!(alpha.source, "user");
        assert_eq!(
            alpha.read_root_relative.as_deref(),
            Some("user/alpha/SKILL.md")
        );
        assert!(result.diagnostics.iter().any(|d| d.contains("collision")));
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("no description"))
        );
        assert!(read_hint(alpha).contains("coding_tools_read_file"));
    }

    #[test]
    fn linked_duplicates_load_once() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        write(&real.join("s/SKILL.md"), "---\ndescription: S\n---\n");
        let links = temp.path().join("links");
        std::fs::create_dir_all(&links).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.join("s"), links.join("s-link")).unwrap();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(links.join("s-link"))
                .arg(real.join("s"))
                .output()
                .unwrap();
            assert!(status.status.success());
        }
        let plan = ScanPlan::for_roots(vec![(links, "user"), (real, "path")], None);
        let result = plan.scan();
        assert_eq!(result.skills.len(), 1);
        assert!(read_hint(&result.skills[0]).contains("coding_tools_exec_command"));
    }

    #[test]
    fn instructions_list_is_bounded_and_skips_disabled() {
        let skill = |name: &str, disabled: bool| Skill {
            name: name.to_string(),
            description: "d".repeat(200),
            path: String::new(),
            canonical_path: String::new(),
            source: "user".to_string(),
            disable_model_invocation: disabled,
            read_root_relative: None,
        };
        let result = ScanResult {
            skills: vec![
                skill("a", false),
                skill("b", true),
                skill("c", false),
                skill("d", false),
            ],
            diagnostics: Vec::new(),
        };
        let text = instructions_block(&result, 2).unwrap();
        assert!(text.contains("- a:") && text.contains("- c:"));
        assert!(!text.contains("- b:") && !text.contains("- d:"));
        assert!(text.contains("+1 more"));
        assert!(instructions_block(&ScanResult::default(), 5).is_none());
    }
}
