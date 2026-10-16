//! Capability declarations and the cross-surface tool search index.
//!
//! `lomway_search_tools` searches three surfaces with one ranking:
//!
//! * **direct** tools that the gateway lists in `tools/list`;
//! * **deferred** tools, read from a cached manifest (never by starting or
//!   reconnecting a backend), or a per-backend `unverified` placeholder when
//!   no manifest has ever been observed;
//! * **internal** capabilities that have no MCP tool of their own and are
//!   reached through an existing tool (for example a CLI behind Workbridge).
//!
//! Declarations live in `capabilities/catalog.toml` and carry upstream OSS
//! names plus English/Japanese aliases per tool or per action. They contain
//! names only; machine paths, ports, and URLs are rejected by validation.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::gateway::auth::pat_tool_allowed;

const CATALOG_SOURCE: &str = include_str!("../../capabilities/catalog.toml");
const CATALOG_SCHEMA_VERSION: u32 = 1;

/// Weights follow the documented ranking order: OSS names, aliases, action
/// names/aliases, tool name, then description text.
const W_OSS: f64 = 100.0;
const W_ALIAS: f64 = 80.0;
const W_ACTION: f64 = 60.0;
const W_NAME: f64 = 40.0;
const W_DESC: f64 = 15.0;
/// Minimum score for a result. Description-only incidental mentions stay
/// below it so unrelated tools are not returned.
pub(super) const SCORE_THRESHOLD: f64 = 25.0;
/// An action is reported only when its own match is meaningful.
const ACTION_REPORT_MIN: f64 = W_ACTION * 0.5;
const DESCRIPTION_PREVIEW_CHARS: usize = 400;

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    #[serde(default)]
    capability: Vec<Declaration>,
    #[serde(default)]
    internal: Vec<InternalDeclaration>,
}

/// Capability metadata for one tool (`tool`) or every tool of one backend
/// (`backend`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Declaration {
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    provider_oss: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    actions: Vec<ActionDeclaration>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionDeclaration {
    pub(super) action: String,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
}

/// A capability with no MCP tool of its own, reached through `via`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InternalDeclaration {
    pub(super) name: String,
    pub(super) via: String,
    #[serde(default)]
    pub(super) domain: Option<String>,
    #[serde(default)]
    pub(super) provider_oss: Option<String>,
    #[serde(default)]
    pub(super) summary: Option<String>,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CapabilityCatalog {
    declarations: Vec<Declaration>,
    internal: Vec<InternalDeclaration>,
}

impl CapabilityCatalog {
    /// The repository's built-in declarations, validated once.
    pub(super) fn builtin() -> &'static Self {
        static CATALOG: OnceLock<CapabilityCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            Self::parse(CATALOG_SOURCE).expect("built-in capability catalog must be valid")
        })
    }

    pub(super) fn parse(text: &str) -> Result<Self> {
        let file: CatalogFile = toml::from_str(text)?;
        ensure!(
            file.schema_version == CATALOG_SCHEMA_VERSION,
            "unsupported capability catalog schema_version {}",
            file.schema_version
        );
        for declaration in &file.capability {
            ensure!(
                declaration.tool.is_some() != declaration.backend.is_some(),
                "each [[capability]] must set exactly one of `tool` or `backend`"
            );
            let mut texts: Vec<&str> = declaration.aliases.iter().map(String::as_str).collect();
            texts.extend(declaration.tool.as_deref());
            texts.extend(declaration.backend.as_deref());
            texts.extend(declaration.provider_oss.as_deref());
            texts.extend(declaration.domain.as_deref());
            for action in &declaration.actions {
                ensure!(!action.action.trim().is_empty(), "empty action name");
                texts.push(&action.action);
                texts.extend(action.aliases.iter().map(String::as_str));
            }
            for text in texts {
                reject_machine_specific(text)?;
            }
        }
        for internal in &file.internal {
            ensure!(
                !internal.name.trim().is_empty() && !internal.via.trim().is_empty(),
                "each [[internal]] needs `name` and `via`"
            );
            let mut texts: Vec<&str> = internal.aliases.iter().map(String::as_str).collect();
            texts.push(&internal.name);
            texts.push(&internal.via);
            texts.extend(internal.summary.as_deref());
            texts.extend(internal.provider_oss.as_deref());
            texts.extend(internal.domain.as_deref());
            for text in texts {
                reject_machine_specific(text)?;
            }
        }
        Ok(Self {
            declarations: file.capability,
            internal: file.internal,
        })
    }

    pub(super) fn internal(&self) -> &[InternalDeclaration] {
        &self.internal
    }

    pub(super) fn internal_named(&self, name: &str) -> Option<&InternalDeclaration> {
        self.internal.iter().find(|internal| internal.name == name)
    }

    /// Merge every declaration that applies to `tool` of `backend`. Tool-level
    /// declarations win for provider and domain; aliases and actions merge.
    pub(super) fn metadata_for(&self, tool: Option<&str>, backend: &str) -> Metadata {
        let mut metadata = Metadata::default();
        let applicable = self.declarations.iter().filter(|declaration| {
            declaration.backend.as_deref() == Some(backend)
                || (tool.is_some() && declaration.tool.as_deref() == tool)
        });
        // Backend-level first so tool-level values override them.
        let (tool_level, backend_level): (Vec<_>, Vec<_>) =
            applicable.partition(|declaration| declaration.tool.is_some());
        metadata.tool_declared = !tool_level.is_empty();
        for declaration in backend_level.into_iter().chain(tool_level) {
            if declaration.provider_oss.is_some() {
                metadata.provider_oss = declaration.provider_oss.clone();
            }
            if declaration.domain.is_some() {
                metadata.domain = declaration.domain.clone();
            }
            for alias in &declaration.aliases {
                if !metadata.aliases.contains(alias) {
                    metadata.aliases.push(alias.clone());
                }
            }
            for action in &declaration.actions {
                match metadata
                    .actions
                    .iter_mut()
                    .find(|known| known.action == action.action)
                {
                    Some(known) => known.aliases.extend(action.aliases.iter().cloned()),
                    None => metadata.actions.push(action.clone()),
                }
            }
        }
        metadata
    }
}

/// Repository declarations are public: refuse anything that looks like a
/// URL, a loopback/private/link-local/CGNAT address, a `host:port` binding,
/// an absolute or home-relative machine path (POSIX, Windows, UNC), or an
/// email address. Ordinary slashes between words (`read/write`, `CLI/daemon`,
/// a lone `/`) are not paths and stay allowed.
fn reject_machine_specific(text: &str) -> Result<()> {
    if let Some(reason) = machine_specific_reason(text) {
        bail!(
            "capability declarations must not contain machine-specific locations ({reason}): {text:?}"
        );
    }
    Ok(())
}

fn machine_specific_reason(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("://") {
        return Some("URL");
    }
    if lower.contains("localhost") {
        return Some("loopback host");
    }
    if lower.contains('\\') {
        return Some("Windows or UNC path");
    }
    if lower.contains(":/") {
        return Some("drive or scheme path");
    }
    if lower.contains("~/") {
        return Some("home-relative path");
    }
    for token in lower
        .split(|ch: char| ch.is_whitespace() || is_token_delimiter(ch))
        .filter(|token| !token.is_empty())
    {
        if looks_like_absolute_posix_path(token) {
            return Some("absolute path");
        }
        if looks_like_email(token) {
            return Some("email address");
        }
        if looks_like_host_port(token) {
            return Some("host:port");
        }
    }
    if contains_non_public_ip(&lower) {
        return Some("loopback, private, link-local, or CGNAT address");
    }
    None
}

fn is_token_delimiter(ch: char) -> bool {
    matches!(
        ch,
        ',' | ';' | '"' | '\'' | '`' | '(' | ')' | '{' | '}' | '<' | '>' | '|' | '='
    ) || (!ch.is_ascii() && !ch.is_alphanumeric())
}

/// `/etc/secrets`, `/mcp`, `/.config` — but not a lone `/` or `//`.
fn looks_like_absolute_posix_path(token: &str) -> bool {
    let token = token.trim_start_matches(['[', '*']);
    let mut chars = token.chars();
    chars.next() == Some('/')
        && chars.next().is_some_and(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '~' | '$')
        })
}

fn looks_like_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    let domain = domain.trim_end_matches(['.', ']', ':']);
    !local.is_empty()
        && local
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '+' | '-' | '%'))
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        })
}

/// `host:8080`, `10.0.0.1:8080`, `[::1]:8080`. A bare ratio or clock time
/// such as `16:9` or `10:30` has no host-like part and is not matched.
fn looks_like_host_port(token: &str) -> bool {
    let token = token.trim_end_matches(['.', ']']);
    let Some((host, port)) = token.rsplit_once(':') else {
        return false;
    };
    if port.is_empty()
        || port.len() > 5
        || !port.chars().all(|ch| ch.is_ascii_digit())
        || !port.parse::<u32>().is_ok_and(|port| port <= 65_535)
    {
        return false;
    }
    if let Some(inner) = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
    {
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    }
    !host.is_empty()
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'))
        && (host.contains('.') || host.chars().any(|ch| ch.is_ascii_alphabetic()))
}

fn contains_non_public_ip(lower: &str) -> bool {
    let v4 = lower
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .map(|candidate| candidate.trim_matches('.'))
        .filter_map(|candidate| candidate.parse::<std::net::Ipv4Addr>().ok())
        .any(non_public_v4);
    if v4 {
        return true;
    }
    lower
        .split(|ch: char| !(ch.is_ascii_hexdigit() || matches!(ch, ':' | '.' | '%')))
        .filter(|candidate| candidate.contains(':'))
        .map(|candidate| candidate.split('%').next().unwrap_or_default())
        .filter_map(|candidate| candidate.parse::<std::net::Ipv6Addr>().ok())
        .any(non_public_v6)
}

fn non_public_v4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        // CGNAT shared address space 100.64.0.0/10.
        || (a == 100 && (64..=127).contains(&b))
}

fn non_public_v6(ip: std::net::Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return non_public_v4(v4);
    }
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        // Unique local fc00::/7 and link-local fe80::/10.
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
}

#[derive(Debug, Clone, Default)]
pub(super) struct Metadata {
    pub(super) provider_oss: Option<String>,
    pub(super) domain: Option<String>,
    pub(super) aliases: Vec<String>,
    pub(super) actions: Vec<ActionDeclaration>,
    /// A tool-level declaration exists (ranks ahead of backend-wide ties).
    pub(super) tool_declared: bool,
}

// ---------------------------------------------------------------------------
// Caller access
// ---------------------------------------------------------------------------

/// What the calling principal may see. `None` means the request was not
/// restricted by a PAT/OAuth allowlist (unrestricted owner or no northbound
/// auth). Decisions reuse the PAT/OAuth allowlist predicate from `auth`.
#[derive(Debug, Clone, Default)]
pub(super) struct CallerAccess {
    allowed: Option<Vec<String>>,
}

impl CallerAccess {
    pub(super) fn new(allowed: Option<Vec<String>>) -> Self {
        Self { allowed }
    }

    pub(super) fn tool(&self, name: &str) -> bool {
        match &self.allowed {
            Some(allowed) => pat_tool_allowed(allowed, name),
            None => pat_tool_allowed(&["*".to_string()], name),
        }
    }

    /// Whether any tool of a backend could be visible. Used for unverified
    /// placeholders whose concrete tool names are not known yet.
    pub(super) fn any_with_prefix(&self, prefix: &str) -> bool {
        match &self.allowed {
            Some(allowed) => allowed
                .iter()
                .any(|tool| tool == "*" || tool.starts_with(prefix)),
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Invocation {
    Direct,
    Deferred,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Status {
    /// Present in the gateway's current `tools/list` catalog. This says
    /// only that the tool is listed: it does not claim the backend is
    /// healthy or that a call will succeed.
    Listed,
    /// Deferred definition from the last observed manifest.
    Cached,
    /// Deferred backend whose tools have never been observed.
    Unverified,
    /// Declared internal capability reached through another tool.
    Declared,
}

/// One searchable tool definition, before capability metadata is merged.
#[derive(Debug, Clone)]
pub(super) struct ToolSource {
    pub(super) name: String,
    pub(super) backend: String,
    pub(super) invocation: Invocation,
    pub(super) status: Status,
    pub(super) manifest_seen_at: Option<String>,
    pub(super) description: Option<String>,
    /// Deferred placeholder access is decided by backend prefix.
    pub(super) placeholder_prefix: Option<String>,
    /// Internal capability: the tool that actually reaches it.
    pub(super) via: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct IndexEntry {
    pub(super) source: ToolSource,
    pub(super) metadata: Metadata,
}

pub(super) fn build_index(
    catalog: &CapabilityCatalog,
    sources: Vec<ToolSource>,
) -> Vec<IndexEntry> {
    let mut entries: Vec<IndexEntry> = sources
        .into_iter()
        .map(|source| {
            let tool = source
                .placeholder_prefix
                .is_none()
                .then_some(source.name.as_str());
            let metadata = catalog.metadata_for(tool, &source.backend);
            IndexEntry { source, metadata }
        })
        .collect();
    for internal in catalog.internal() {
        entries.push(IndexEntry {
            source: ToolSource {
                name: internal.name.clone(),
                backend: "internal".to_string(),
                invocation: Invocation::Internal,
                status: Status::Declared,
                manifest_seen_at: None,
                description: internal.summary.clone(),
                placeholder_prefix: None,
                via: Some(internal.via.clone()),
            },
            metadata: Metadata {
                provider_oss: internal.provider_oss.clone(),
                domain: internal.domain.clone(),
                aliases: internal.aliases.clone(),
                actions: Vec::new(),
                tool_declared: true,
            },
        });
    }
    entries
}

impl IndexEntry {
    pub(super) fn visible_to(&self, access: &CallerAccess) -> bool {
        if let Some(via) = &self.source.via {
            return access.tool(via);
        }
        if let Some(prefix) = &self.source.placeholder_prefix {
            return access.any_with_prefix(prefix);
        }
        access.tool(&self.source.name)
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SearchResult {
    pub(super) name: String,
    pub(super) backend: String,
    pub(super) invocation: Invocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) action: Option<String>,
    pub(super) provider_oss: Option<String>,
    pub(super) matched_aliases: Vec<String>,
    pub(super) status: Status,
    pub(super) manifest_seen_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) via: Option<String>,
    pub(super) score: f64,
    pub(super) description: Option<String>,
    pub(super) how_to_invoke: String,
    #[serde(skip)]
    tool_declared: bool,
}

/// Rank visible entries for `query`. Entries the caller may not use are
/// removed before scoring so their existence is never observable.
pub(super) fn search(
    entries: &[IndexEntry],
    query: &str,
    backend: Option<&str>,
    access: &CallerAccess,
    limit: usize,
) -> Vec<SearchResult> {
    let query = PreparedQuery::new(query);
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<SearchResult> = entries
        .iter()
        .filter(|entry| entry.visible_to(access))
        .filter(|entry| backend.is_none_or(|wanted| entry.source.backend == wanted))
        .filter_map(|entry| score_entry(entry, &query))
        .filter(|result| result.score >= SCORE_THRESHOLD)
        .collect();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.tool_declared.cmp(&left.tool_declared))
            .then_with(|| invocation_rank(left.invocation).cmp(&invocation_rank(right.invocation)))
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked.truncate(limit);
    ranked
}

fn invocation_rank(invocation: Invocation) -> u8 {
    match invocation {
        Invocation::Direct => 0,
        Invocation::Deferred => 1,
        Invocation::Internal => 2,
    }
}

fn score_entry(entry: &IndexEntry, query: &PreparedQuery) -> Option<SearchResult> {
    let metadata = &entry.metadata;
    let mut matched = Vec::new();

    let oss = metadata
        .provider_oss
        .as_deref()
        .map(|oss| {
            let score = query.match_score(oss);
            if score >= 0.7 {
                matched.push(oss.to_string());
            }
            W_OSS * score
        })
        .unwrap_or(0.0);

    let mut alias = 0.0_f64;
    for candidate in &metadata.aliases {
        let score = query.match_score(candidate);
        if score >= 0.7 && !matched.contains(candidate) {
            matched.push(candidate.clone());
        }
        alias = alias.max(W_ALIAS * score);
    }

    let name = W_NAME
        * query
            .match_score(&entry.source.name)
            .max(query.match_score(&entry.source.backend));

    let mut best_action: Option<(f64, &str, Vec<String>)> = None;
    for action in &metadata.actions {
        let mut action_matched = Vec::new();
        let mut score = query.match_score(&action.action);
        for candidate in &action.aliases {
            let alias_score = query.match_score(candidate);
            if alias_score >= 0.7 {
                action_matched.push(candidate.clone());
            }
            score = score.max(alias_score);
        }
        let score = W_ACTION * score;
        if best_action
            .as_ref()
            .is_none_or(|(best, _, _)| score > *best)
        {
            best_action = Some((score, action.action.as_str(), action_matched));
        }
    }
    let action_score = best_action.as_ref().map_or(0.0, |(score, _, _)| *score);

    let description = entry
        .source
        .description
        .as_deref()
        .map_or(0.0, |text| W_DESC * query.match_score(text));

    let primary = oss.max(alias).max(name);
    let total = primary.max(action_score) + 0.25 * primary.min(action_score) + description;
    if total <= 0.0 {
        return None;
    }

    let action = best_action.and_then(|(score, action, action_matched)| {
        (score >= ACTION_REPORT_MIN).then(|| {
            for alias in action_matched {
                if !matched.contains(&alias) {
                    matched.push(alias);
                }
            }
            action.to_string()
        })
    });

    Some(SearchResult {
        name: entry.source.name.clone(),
        backend: entry.source.backend.clone(),
        invocation: entry.source.invocation,
        how_to_invoke: invocation_guidance(entry, action.as_deref()),
        action,
        provider_oss: metadata.provider_oss.clone(),
        matched_aliases: matched,
        status: entry.source.status,
        manifest_seen_at: entry.source.manifest_seen_at.clone(),
        domain: metadata.domain.clone(),
        via: entry.source.via.clone(),
        score: (total * 10.0).round() / 10.0,
        description: entry.source.description.as_deref().map(preview),
        tool_declared: metadata.tool_declared,
    })
}

pub(super) fn invocation_guidance(entry: &IndexEntry, action: Option<&str>) -> String {
    let source = &entry.source;
    match (source.invocation, source.status) {
        (Invocation::Direct, _) => {
            let mut text = format!(
                "Direct tool: it is listed in tools/list, so call `{}` directly. Do not use lomway_call_tool.",
                source.name
            );
            if let Some(action) = action {
                text.push_str(&format!(" Pass action=\"{action}\"."));
            }
            text
        }
        (Invocation::Deferred, Status::Unverified) => format!(
            "Deferred backend with no cached tool manifest yet. Call lomway_describe_tool with name \"{}\" to connect the backend and list its tools, then lomway_call_tool.",
            source.name
        ),
        (Invocation::Deferred, _) => format!(
            "Deferred tool from the cached manifest (manifest_seen_at). Use lomway_describe_tool with name \"{}\" for the exact current schema, then lomway_call_tool.",
            source.name
        ),
        (Invocation::Internal, _) => format!(
            "Internal capability without its own MCP tool: reach it through `{}`.",
            source.via.as_deref().unwrap_or("the listed tool")
        ),
    }
}

fn preview(text: &str) -> String {
    if text.chars().count() <= DESCRIPTION_PREVIEW_CHARS {
        return text.to_string();
    }
    let mut short: String = text.chars().take(DESCRIPTION_PREVIEW_CHARS).collect();
    short.push_str("… (use lomway_describe_tool for the full description)");
    short
}

// ---------------------------------------------------------------------------
// Text matching (ASCII words + character bigrams for Japanese)
// ---------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "to", "of", "for", "in", "on", "and", "or", "with", "by", "is", "it", "tool",
    "tools", "use", "using", "me", "my", "i", "want", "please", "how", "do", "can",
];

/// Lowercase, fold full-width ASCII, and turn identifier punctuation into
/// spaces so `find_referencing_symbols` matches "find referencing symbols".
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let ch = match ch as u32 {
            0xFF01..=0xFF5E => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            0x3000 => ' ',
            _ => ch,
        };
        let ch = ch.to_lowercase().next().unwrap_or(ch);
        if ch.is_ascii_alphanumeric() || !ch.is_ascii() && !is_cjk_punctuation(ch) {
            out.push(ch);
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_cjk_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '、' | '。' | '「' | '」' | '・' | '？' | '！' | '（' | '）' | '：' | '…'
    ) || ch.is_whitespace()
}

fn ascii_words(normalized: &str) -> BTreeSet<String> {
    normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word))
        .map(str::to_string)
        .collect()
}

fn cjk_bigrams(normalized: &str) -> BTreeSet<String> {
    let mut grams = BTreeSet::new();
    for run in normalized.split(|ch: char| ch.is_ascii() || ch.is_whitespace()) {
        let chars: Vec<char> = run.chars().collect();
        match chars.len() {
            0 => {}
            1 => {
                grams.insert(chars[0].to_string());
            }
            _ => {
                for pair in chars.windows(2) {
                    grams.insert(pair.iter().collect());
                }
            }
        }
    }
    grams
}

/// Whole-word containment for ASCII needles, substring for others.
fn contains_term(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if !needle.is_ascii() {
        return haystack.contains(needle);
    }
    let padded_haystack = format!(" {haystack} ");
    padded_haystack.contains(&format!(" {needle} "))
}

pub(super) struct PreparedQuery {
    normalized: String,
    words: BTreeSet<String>,
    bigrams: BTreeSet<String>,
}

impl PreparedQuery {
    pub(super) fn new(query: &str) -> Self {
        let normalized = normalize(query);
        Self {
            words: ascii_words(&normalized),
            bigrams: cjk_bigrams(&normalized),
            normalized,
        }
    }

    fn is_empty(&self) -> bool {
        self.words.is_empty() && self.bigrams.is_empty()
    }

    /// Similarity in `[0, 1]`: 1.0 exact, 0.8 candidate inside the query,
    /// 0.7 query inside the candidate, otherwise up to 0.6 by word overlap
    /// and Japanese character-bigram overlap.
    pub(super) fn match_score(&self, candidate: &str) -> f64 {
        let candidate = normalize(candidate);
        if candidate.is_empty() || self.normalized.is_empty() {
            return 0.0;
        }
        if candidate == self.normalized {
            return 1.0;
        }
        if candidate.chars().count() >= 2 && contains_term(&self.normalized, &candidate) {
            return 0.8;
        }
        if self.normalized.chars().count() >= 3 && contains_term(&candidate, &self.normalized) {
            return 0.7;
        }
        let units = self.words.len() + self.bigrams.len();
        if units == 0 {
            return 0.0;
        }
        let candidate_words = ascii_words(&candidate);
        let shared_words = self.words.intersection(&candidate_words).count();
        let candidate_bigrams = cjk_bigrams(&candidate);
        let shared_bigrams = self.bigrams.intersection(&candidate_bigrams).count();
        // A single shared bigram (for example 検索) is too weak to count.
        let shared_bigrams = if shared_bigrams >= 2 {
            shared_bigrams
        } else {
            0
        };
        0.6 * (shared_words + shared_bigrams) as f64 / units as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct FixtureTool {
        name: String,
        description: Option<String>,
    }

    /// Direct tool names and descriptions as served by a real deployment
    /// (input schemas removed), plus unverified deferred placeholders.
    fn fixture_entries() -> Vec<IndexEntry> {
        let tools: Vec<FixtureTool> = serde_json::from_str(include_str!(
            "../../test-fixtures/capability-search/direct-tools.json"
        ))
        .expect("fixture parses");
        let mut sources: Vec<ToolSource> = tools
            .into_iter()
            .map(|tool| ToolSource {
                backend: tool.name.split('_').next().unwrap_or_default().to_string(),
                name: tool.name,
                invocation: Invocation::Direct,
                status: Status::Listed,
                manifest_seen_at: None,
                description: tool.description,
                placeholder_prefix: None,
                via: None,
            })
            .collect();
        for (backend, prefix) in [
            ("browser", "browser_"),
            ("chrome", "chrome_"),
            ("cua-windows", "cua_windows_"),
            ("ufo", "ufo_"),
        ] {
            sources.push(ToolSource {
                name: format!("{prefix}*"),
                backend: backend.to_string(),
                invocation: Invocation::Deferred,
                status: Status::Unverified,
                manifest_seen_at: None,
                description: None,
                placeholder_prefix: Some(prefix.to_string()),
                via: None,
            });
        }
        build_index(CapabilityCatalog::builtin(), sources)
    }

    fn top(query: &str, access: &CallerAccess) -> Vec<SearchResult> {
        let results = search(&fixture_entries(), query, None, access, 8);
        let summary: Vec<String> = results
            .iter()
            .take(3)
            .map(|result| {
                format!(
                    "{} [{:?}] action={:?} score={}",
                    result.name, result.invocation, result.action, result.score
                )
            })
            .collect();
        println!("ACCEPTANCE {query:?} top3: {summary:?}");
        results
    }

    #[test]
    fn builtin_catalog_parses_and_declares_three_domains() {
        let catalog = CapabilityCatalog::builtin();
        let semantic = catalog.metadata_for(Some("workbridge_run_semantic_action"), "workbridge");
        assert_eq!(semantic.provider_oss.as_deref(), Some("Serena"));
        let actions: Vec<&str> = semantic.actions.iter().map(|a| a.action.as_str()).collect();
        for expected in [
            "find_symbol",
            "find_declaration",
            "find_implementations",
            "find_referencing_symbols",
            "get_symbols_overview",
            "get_diagnostics_for_file",
            "search_for_pattern",
        ] {
            assert!(actions.contains(&expected), "{expected} missing");
        }
        assert_eq!(
            catalog
                .metadata_for(Some("workbridge_run_graft_action"), "workbridge")
                .provider_oss
                .as_deref(),
            Some("Graft")
        );
        assert_eq!(
            catalog
                .metadata_for(Some("workbridge_run_codebase_memory_action"), "workbridge")
                .provider_oss
                .as_deref(),
            Some("Codebase Memory")
        );
        for (backend, oss) in [
            ("browser", "Stealth Browser"),
            ("chrome", "chrome-devtools-mcp"),
            ("cua-windows", "Cua"),
            ("hindsight", "Hindsight"),
        ] {
            assert_eq!(
                catalog.metadata_for(None, backend).provider_oss.as_deref(),
                Some(oss)
            );
        }
        assert!(catalog.internal_named("browserskill").is_some());
    }

    #[test]
    fn catalog_rejects_machine_specific_locations() {
        for bad in [
            "schema_version = 1\n[[capability]]\ntool = \"x\"\naliases = [\"http://127.0.0.1:9/mcp\"]\n",
            "schema_version = 1\n[[capability]]\ntool = \"x\"\naliases = [\"C:\\\\Users\\\\me\"]\n",
            "schema_version = 1\n[[capability]]\ntool = \"x\"\nbackend = \"y\"\n",
            "schema_version = 2\n",
        ] {
            assert!(CapabilityCatalog::parse(bad).is_err(), "{bad}");
        }
    }

    fn capability_with_alias(alias: &str) -> String {
        format!(
            "schema_version = 1\n[[capability]]\ntool = \"x\"\naliases = [{}]\n",
            toml::Value::String(alias.to_string())
        )
    }

    #[test]
    fn catalog_rejects_posix_paths_private_ips_host_ports_and_emails() {
        for bad in [
            "/etc/secrets",
            "read /etc/secrets first",
            "/mcp",
            "~/notes",
            "C:/Users/me",
            "\\\\server\\share",
            "192.168.0.187",
            "10.0.0.1:8080",
            "10.0.0.1",
            "172.16.4.2",
            "169.254.169.254",
            "100.64.0.1",
            "127.0.0.2",
            "0.0.0.0",
            "::1",
            "fe80::1%eth0",
            "fd12:3456::1",
            "::ffff:192.168.1.1",
            "[::1]:8080",
            "devbox:17777",
            "gateway.internal:443",
            "owner@example.com",
            "localhost",
            "https://example.com",
        ] {
            assert!(
                CapabilityCatalog::parse(&capability_with_alias(bad)).is_err(),
                "{bad:?} must be rejected"
            );
        }
        for ok in [
            "read/write",
            "CLI/daemon",
            "TCP/IP",
            "A / B",
            "16:9",
            "10:30",
            "v1.2.3",
            "シンボル検索",
            "@mention",
            "8.8.8.8",
        ] {
            assert!(
                CapabilityCatalog::parse(&capability_with_alias(ok)).is_ok(),
                "{ok:?} must stay allowed"
            );
        }
    }

    #[test]
    fn catalog_validates_every_internal_field_including_domain() {
        let internal = |field: &str, value: &str| {
            let mut fields = std::collections::BTreeMap::from([
                ("name", "tool-x"),
                ("via", "workbridge_exec_command"),
                ("domain", "browser"),
                ("provider_oss", "X"),
                ("summary", "Summary"),
            ]);
            fields.insert(field, value);
            let mut text = String::from("schema_version = 1\n[[internal]]\n");
            for (key, value) in fields {
                text.push_str(&format!(
                    "{key} = {}\n",
                    toml::Value::String(value.to_string())
                ));
            }
            text.push_str("aliases = [\"x\"]\n");
            text
        };
        assert!(CapabilityCatalog::parse(&internal("domain", "browser")).is_ok());
        for field in ["name", "via", "domain", "provider_oss", "summary"] {
            for bad in ["/etc/secrets", "192.168.0.187", "10.0.0.1:8080"] {
                assert!(
                    CapabilityCatalog::parse(&internal(field, bad)).is_err(),
                    "[[internal]] {field} = {bad:?} must be rejected"
                );
            }
        }
        // Every [[capability]] field is validated too, domain included.
        for field in ["tool", "domain", "provider_oss"] {
            let text = format!(
                "schema_version = 1\n[[capability]]\n{}{field} = \"/etc/secrets\"\n",
                if field == "tool" {
                    ""
                } else {
                    "tool = \"x\"\n"
                }
            );
            assert!(CapabilityCatalog::parse(&text).is_err(), "{field}");
        }
        let action = "schema_version = 1\n[[capability]]\ntool = \"x\"\n[[capability.actions]]\naction = \"a\"\naliases = [\"192.168.0.187\"]\n";
        assert!(CapabilityCatalog::parse(action).is_err());
    }

    #[test]
    fn repository_catalog_passes_strict_validation() {
        assert!(CapabilityCatalog::parse(CATALOG_SOURCE).is_ok());
    }

    #[test]
    fn acceptance_queries_rank_serena_first_as_direct() {
        let access = CallerAccess::default();
        for query in ["Serena", "LSP", "シンボル検索", "関数の参照元を探す"] {
            let results = top(query, &access);
            let first = results
                .first()
                .unwrap_or_else(|| panic!("{query}: no results"));
            assert_eq!(first.name, "workbridge_run_semantic_action", "{query}");
            assert_eq!(first.invocation, Invocation::Direct, "{query}");
            assert_eq!(first.provider_oss.as_deref(), Some("Serena"), "{query}");
            assert!(first.how_to_invoke.contains("Do not use lomway_call_tool"));
        }
        let references = top("関数の参照元を探す", &access);
        assert_eq!(
            references[0].action.as_deref(),
            Some("find_referencing_symbols")
        );
        assert!(
            references[0]
                .matched_aliases
                .iter()
                .any(|alias| alias.contains("参照元"))
        );
        assert_eq!(
            top("シンボル検索", &access)[0].action.as_deref(),
            Some("find_symbol")
        );
    }

    #[test]
    fn caller_without_workbridge_sees_nothing_for_serena() {
        let access = CallerAccess::new(Some(vec![
            "lomway_search_tools".to_string(),
            "hindsight_recall".to_string(),
        ]));
        assert!(top("Serena", &access).is_empty());
        assert!(top("関数の参照元を探す", &access).is_empty());
        // Allowed tools remain findable for the same caller.
        assert_eq!(top("記憶を検索", &access)[0].name, "hindsight_recall");
    }

    #[test]
    fn unrelated_capability_returns_nothing() {
        let access = CallerAccess::default();
        assert!(top("量子暗号鍵配送", &access).is_empty());
        assert!(top("quantum key distribution", &access).is_empty());
    }

    #[test]
    fn deferred_without_manifest_is_unverified_placeholder() {
        let access = CallerAccess::default();
        let results = top("Stealth Browser", &access);
        let first = &results[0];
        assert_eq!(first.name, "browser_*");
        assert_eq!(first.invocation, Invocation::Deferred);
        assert_eq!(first.status, Status::Unverified);
        assert!(first.manifest_seen_at.is_none());
        assert_eq!(top("chrome devtools", &access)[0].backend, "chrome");
        assert_eq!(top("Cua", &access)[0].backend, "cua-windows");
        // A restricted caller with no browser tools does not see it.
        let limited = CallerAccess::new(Some(vec!["hindsight_recall".to_string()]));
        assert!(top("Stealth Browser", &limited).is_empty());
    }

    #[test]
    fn internal_capability_is_reachable_only_through_its_via_tool() {
        let results = top("BrowserSkill", &CallerAccess::default());
        assert_eq!(results[0].name, "browserskill");
        assert_eq!(results[0].invocation, Invocation::Internal);
        assert_eq!(results[0].via.as_deref(), Some("workbridge_exec_command"));
        let limited = CallerAccess::new(Some(vec!["workbridge_read".to_string()]));
        assert!(
            top("BrowserSkill", &limited)
                .iter()
                .all(|result| result.name != "browserskill")
        );
    }

    #[test]
    fn memory_domain_is_searchable_in_japanese_and_english() {
        let access = CallerAccess::default();
        let hindsight = top("Hindsight", &access);
        assert_eq!(hindsight[0].backend, "hindsight");
        assert!(
            ["hindsight_recall", "hindsight_retain", "hindsight_reflect"]
                .contains(&hindsight[0].name.as_str()),
            "declared core memory tools rank first: {}",
            hindsight[0].name
        );
        assert_eq!(top("長期記憶", &access)[0].backend, "hindsight");
        assert_eq!(top("recall memory", &access)[0].name, "hindsight_recall");
    }

    #[test]
    fn matching_handles_identifiers_and_full_width_text() {
        let query = PreparedQuery::new("find referencing symbols");
        assert_eq!(query.match_score("find_referencing_symbols"), 1.0);
        assert_eq!(PreparedQuery::new("ＬＳＰ").match_score("lsp"), 1.0);
        assert_eq!(PreparedQuery::new("検索").match_score("全文検索"), 0.0);
    }
}
