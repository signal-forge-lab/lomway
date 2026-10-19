//! Path helpers shared by skills, actions, and file transfer.
//!
//! * `~` / `${VAR}` expansion for operator-configured paths;
//! * a portable "safe relative path" parser modeled on Workbridge's
//!   `parseSafeWindowsArtifactRelativePath` (applied on every platform so a
//!   definition behaves the same everywhere);
//! * reparse-point / symlink detection and root confinement checks.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

/// Expand a leading `~` (home directory) and `${VAR}` references.
/// An unset variable is an error rather than an empty string.
pub(crate) fn expand(input: &str) -> Result<PathBuf> {
    expand_with(input, |name| std::env::var(name).ok())
}

pub(crate) fn expand_with(input: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<PathBuf> {
    let trimmed = input.trim();
    ensure!(!trimmed.is_empty(), "empty path");
    let mut out = String::new();
    let mut rest = trimmed;
    if rest == "~" || rest.starts_with("~/") || rest.starts_with("~\\") {
        let home = lookup("USERPROFILE")
            .filter(|value| !value.is_empty())
            .or_else(|| lookup("HOME").filter(|value| !value.is_empty()))
            .context("cannot expand `~`: neither USERPROFILE nor HOME is set")?;
        out.push_str(&home);
        rest = &rest[1..];
    }
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').context("unterminated `${` in path")?;
        let name = &after[..end];
        ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "invalid environment variable reference in path"
        );
        let value =
            lookup(name).with_context(|| format!("environment variable {name} is not set"))?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(PathBuf::from(out))
}

/// Expand and require an absolute path.
pub(crate) fn expand_absolute(input: &str, what: &str) -> Result<PathBuf> {
    let path = expand(input).with_context(|| format!("{what}: cannot expand path"))?;
    ensure!(path.is_absolute(), "{what} must be an absolute path");
    Ok(path)
}

/// Canonicalize an existing directory, requiring it to be a directory.
pub(crate) fn canonical_dir(path: &Path, what: &str) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("{what} does not exist or is not accessible"))?;
    ensure!(canonical.is_dir(), "{what} is not a directory");
    Ok(strip_verbatim(canonical))
}

/// `std::fs::canonicalize` returns `\\?\C:\...` on Windows. Keep the
/// familiar form for display and prefix comparisons when it is a plain
/// drive path.
pub(crate) fn strip_verbatim(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\")
            && rest.len() > 2
            && rest.as_bytes()[1] == b':'
        {
            return PathBuf::from(rest.to_string());
        }
    }
    path
}

/// Case-insensitive on Windows, exact elsewhere. Both paths must already be
/// canonical.
pub(crate) fn is_inside(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path = path.to_string_lossy().to_lowercase().replace('/', "\\");
        let root = root.to_string_lossy().to_lowercase().replace('/', "\\");
        let root = root.trim_end_matches('\\');
        path == root || path.starts_with(&format!("{root}\\"))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

/// A validated relative path: no absolute/drive/UNC prefix, no `..`, no
/// `:` (alternate data streams), no control characters, no Windows device
/// names, and no component ending in a dot or space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeRelativePath {
    parts: Vec<String>,
}

impl SafeRelativePath {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        ensure!(!value.is_empty(), "path must not be empty");
        ensure!(value.len() <= 1024, "path is too long");
        ensure!(
            !value.chars().any(char::is_control),
            "path must not contain control characters"
        );
        ensure!(
            !value.starts_with('/') && !value.starts_with('\\'),
            "path must be relative"
        );
        ensure!(!value.contains(':'), "path must not contain ':'");
        ensure!(
            !value.ends_with('/') && !value.ends_with('\\'),
            "path must name a file or directory, not end with a separator"
        );
        let raw: Vec<&str> = value.split(['/', '\\']).collect();
        ensure!(
            !raw.contains(&".."),
            "path must not contain '..' components"
        );
        let parts: Vec<String> = raw
            .into_iter()
            .filter(|part| !part.is_empty() && *part != ".")
            .map(str::to_string)
            .collect();
        ensure!(!parts.is_empty(), "path must not be empty");
        for part in &parts {
            if let Some(reason) = unsafe_component(part) {
                bail!("path component is not allowed ({reason})");
            }
        }
        Ok(Self { parts })
    }

    pub(crate) fn parts(&self) -> &[String] {
        &self.parts
    }

    pub(crate) fn file_name(&self) -> &str {
        self.parts.last().map(String::as_str).unwrap_or_default()
    }

    pub(crate) fn display(&self) -> String {
        self.parts.join("/")
    }

    pub(crate) fn join_to(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        for part in &self.parts {
            path.push(part);
        }
        path
    }
}

fn unsafe_component(part: &str) -> Option<&'static str> {
    if part.ends_with('.') || part.ends_with(' ') {
        return Some("trailing dot or space");
    }
    if part
        .chars()
        .any(|ch| matches!(ch, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return Some("reserved character");
    }
    let base = part.split('.').next().unwrap_or(part).trim_end();
    let upper = base.to_ascii_uppercase();
    let device = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.len() == 4
        && upper[3..].chars().all(|ch| ('1'..='9').contains(&ch)));
    if device {
        return Some("device name");
    }
    None
}

/// True when the entry itself (not its target) is a symlink, junction, or
/// any other reparse point.
pub(crate) fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Create (one level at a time) and verify every directory of `rel`'s
/// parent chain below `root`, refusing symlinks and reparse points.
/// Returns the final parent directory.
pub(crate) fn prepare_parent_dirs(root: &Path, rel: &SafeRelativePath) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    let parts = rel.parts();
    for part in &parts[..parts.len().saturating_sub(1)] {
        current.push(part);
        match std::fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).context("could not create destination directory");
            }
        }
        let metadata = std::fs::symlink_metadata(&current)
            .context("could not inspect destination directory")?;
        ensure!(
            !is_link_or_reparse(&metadata) && metadata.is_dir(),
            "destination parent is a link, reparse point, or not a directory"
        );
    }
    Ok(current)
}

/// Resolve an existing entry below `root` without following links in any
/// component, returning its canonical path.
pub(crate) fn resolve_existing_no_links(root: &Path, rel: &SafeRelativePath) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    for part in rel.parts() {
        current.push(part);
        let metadata = std::fs::symlink_metadata(&current).context("path does not exist")?;
        ensure!(
            !is_link_or_reparse(&metadata),
            "path goes through a link or reparse point"
        );
    }
    let canonical = strip_verbatim(std::fs::canonicalize(&current).context("path does not exist")?);
    ensure!(is_inside(&canonical, root), "path escapes its root");
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_handles_home_and_env_and_fails_on_unset() {
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some("/srv/profile".to_string()),
            "X" => Some("val".to_string()),
            _ => None,
        };
        assert_eq!(
            expand_with("~/a", lookup).unwrap(),
            PathBuf::from("/srv/profile/a")
        );
        assert_eq!(
            expand_with("${X}/b", lookup).unwrap(),
            PathBuf::from("val/b")
        );
        assert!(expand_with("${MISSING}/b", lookup).is_err());
        assert!(expand_with("${bad-name}", lookup).is_err());
        assert!(expand_with("  ", lookup).is_err());
    }

    #[test]
    fn safe_relative_paths_reject_escapes_and_windows_hazards() {
        for good in ["a.txt", "dir/a.bin", "dir\\sub\\a", "./x/y", "日本語.pdf"] {
            assert!(SafeRelativePath::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "/abs",
            "\\abs",
            "C:\\x",
            "c:x",
            "\\\\server\\share\\x",
            "../x",
            "a/../../x",
            "a/..",
            "x:stream",
            "con",
            "aux.txt",
            "dir/COM1",
            "lpt9.log",
            "a. ",
            "dot.",
            "q?",
            "a|b",
            "tab\there",
            "dir/",
        ] {
            assert!(SafeRelativePath::parse(bad).is_err(), "{bad:?}");
        }
        let rel = SafeRelativePath::parse("a//b/./c.txt").unwrap();
        assert_eq!(rel.display(), "a/b/c.txt");
        assert_eq!(rel.file_name(), "c.txt");
    }

    #[test]
    fn parent_preparation_refuses_symlinked_directories() {
        let root = tempfile::tempdir().unwrap();
        let root_path = canonical_dir(root.path(), "root").unwrap();
        let rel = SafeRelativePath::parse("x/y/z.txt").unwrap();
        let parent = prepare_parent_dirs(&root_path, &rel).unwrap();
        assert!(parent.ends_with(Path::new("x").join("y")));
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), root_path.join("link")).unwrap();
            let rel = SafeRelativePath::parse("link/z.txt").unwrap();
            assert!(prepare_parent_dirs(&root_path, &rel).is_err());
            std::fs::write(outside.path().join("f"), b"x").unwrap();
            let rel = SafeRelativePath::parse("link/f").unwrap();
            assert!(resolve_existing_no_links(&root_path, &rel).is_err());
        }
        #[cfg(windows)]
        {
            // A directory junction needs no privilege on Windows.
            let outside = tempfile::tempdir().unwrap();
            let link = root_path.join("junction");
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(status.status.success());
            let rel = SafeRelativePath::parse("junction/z.txt").unwrap();
            assert!(prepare_parent_dirs(&root_path, &rel).is_err());
            std::fs::write(outside.path().join("f"), b"x").unwrap();
            let rel = SafeRelativePath::parse("junction/f").unwrap();
            assert!(resolve_existing_no_links(&root_path, &rel).is_err());
        }
    }

    #[test]
    fn inside_checks_are_component_aware() {
        assert!(is_inside(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(is_inside(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!is_inside(Path::new("/a/bc"), Path::new("/a/b")));
    }
}
