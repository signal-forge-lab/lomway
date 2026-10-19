//! Chat-host file transfer (Workbridge `download_artifact` and
//! `publish_artifact` equivalents).
//!
//! Receiving follows Workbridge's OpenAI file adapter: the `file` argument
//! is the host's file reference object (`download_url`, `file_id`, ...),
//! declared through the tool's `openai/fileParams` metadata. Only HTTPS on
//! port 443 to the OpenAI file host or OpenAI's regional blob accounts is
//! fetched, redirects are followed manually (at most three, each re-checked),
//! and the body is streamed with a hard size limit into an exclusive partial
//! file that is published without overwriting anything.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::paths::{self, SafeRelativePath};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 3;
const PARTIAL_PREFIX: &str = ".lomway-receive-";
const PARTIAL_SUFFIX: &str = ".partial";
const MAX_FILE_ID_CHARS: usize = 512;

/// The host's file reference (ChatGPT `openai/fileParams` shape).
#[derive(Debug, Clone, Deserialize, Serialize, tower_mcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileReference {
    /// Short-lived download URL supplied by the MCP host.
    pub(crate) download_url: String,
    /// Host file identifier.
    pub(crate) file_id: String,
    #[serde(default)]
    pub(crate) mime_type: Option<String>,
    #[serde(default)]
    pub(crate) file_name: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Received {
    /// Path relative to the inbox (forward slashes).
    pub(crate) path: String,
    /// Absolute path on the gateway host.
    pub(crate) absolute_path: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mime_type: Option<String>,
}

/// Which download URLs are acceptable. Production accepts only the OpenAI
/// file hosts; tests inject a loopback host.
#[derive(Debug, Clone)]
pub(crate) enum TrustedHosts {
    OpenAi,
    #[cfg(test)]
    LoopbackForTests,
}

impl TrustedHosts {
    fn check(&self, url: &Url) -> Result<()> {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let ok = match self {
            Self::OpenAi => {
                url.scheme() == "https"
                    && url.port().is_none_or(|port| port == 443)
                    && (host == "files.oaiusercontent.com" || is_openai_regional_blob(&host))
            }
            #[cfg(test)]
            Self::LoopbackForTests => url.scheme() == "http" && host == "127.0.0.1",
        };
        ensure!(
            ok && url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
            "the file download URL is outside the trusted file host"
        );
        Ok(())
    }
}

/// `oaisdmntpr<alnum>.blob.core.windows.net`, never arbitrary Azure blobs.
fn is_openai_regional_blob(host: &str) -> bool {
    host.strip_suffix(".blob.core.windows.net")
        .and_then(|account| account.strip_prefix("oaisdmntpr"))
        .is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        })
}

#[derive(Debug, Clone)]
pub(crate) struct Inbox {
    pub(crate) root: PathBuf,
    pub(crate) outboxes: Vec<PathBuf>,
    pub(crate) max_receive_bytes: u64,
    pub(crate) max_send_bytes: u64,
    pub(crate) trusted: TrustedHosts,
}

impl Inbox {
    /// Choose the destination: the caller's relative path, else the host
    /// file name, else `<file_id><ext>`.
    fn destination(
        &self,
        reference: &FileReference,
        path: Option<&str>,
    ) -> Result<SafeRelativePath> {
        if let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) {
            return SafeRelativePath::parse(path).map_err(|error| anyhow!("path: {error}"));
        }
        let supplied = reference
            .file_name
            .as_deref()
            .or(reference.name.as_deref())
            .map(|name| name.replace('\\', "/"))
            .and_then(|name| name.rsplit('/').next().map(str::trim).map(str::to_string))
            .filter(|name| !name.is_empty() && !name.starts_with('.'));
        if let Some(name) = supplied
            && let Ok(rel) = SafeRelativePath::parse(&name)
        {
            return Ok(rel);
        }
        let id_ok = reference.file_id.len() <= 256
            && reference
                .file_id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            && (reference.file_id.starts_with("file-") || reference.file_id.starts_with("file_"));
        let base = if id_ok {
            reference.file_id.as_str()
        } else {
            "chatgpt-file"
        };
        let extension = extension_for(reference.mime_type.as_deref()).unwrap_or(".bin");
        SafeRelativePath::parse(&format!("{base}{extension}"))
    }

    /// Download one host file into the inbox.
    pub(crate) async fn receive(
        &self,
        reference: &FileReference,
        path: Option<&str>,
    ) -> Result<Received> {
        ensure!(
            !reference.file_id.is_empty()
                && reference.file_id.chars().count() <= MAX_FILE_ID_CHARS
                && !reference.file_id.chars().any(char::is_control),
            "invalid file_id"
        );
        if let Some(size) = reference.size {
            ensure!(
                size <= self.max_receive_bytes,
                "the file exceeds the configured receive limit"
            );
        }
        let destination = self.destination(reference, path)?;
        let mut url = Url::parse(&reference.download_url).context("invalid download URL")?;
        self.trusted.check(&url)?;
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DOWNLOAD_TIMEOUT);
        #[cfg(test)]
        let builder = if matches!(self.trusted, TrustedHosts::LoopbackForTests) {
            builder.no_proxy()
        } else {
            builder
        };
        let client = builder.build().context("could not build HTTP client")?;
        let mut response = None;
        for hop in 0..=MAX_REDIRECTS {
            let reply = client
                .get(url.clone())
                .send()
                .await
                .map_err(|_| anyhow!("the file could not be downloaded"))?;
            if reply.status().is_redirection() {
                ensure!(hop < MAX_REDIRECTS, "too many redirects");
                let location = reply
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .context("redirect without a location")?;
                url = url.join(location).context("invalid redirect location")?;
                self.trusted.check(&url)?;
                continue;
            }
            response = Some(reply);
            break;
        }
        let mut response = response.context("the file could not be downloaded")?;
        ensure!(
            response.status().is_success(),
            "the file download failed with HTTP {}",
            response.status().as_u16()
        );
        let content_length = response.content_length();
        if let Some(length) = content_length {
            ensure!(
                length <= self.max_receive_bytes,
                "the file exceeds the configured receive limit"
            );
            if let Some(size) = reference.size {
                ensure!(size == length, "file metadata did not match the download");
            }
        }
        let mime_type = reference.mime_type.clone().or_else(|| {
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        });

        let parent = paths::prepare_parent_dirs(&self.root, &destination)?;
        let final_path = parent.join(destination.file_name());
        ensure!(
            std::fs::symlink_metadata(&final_path).is_err(),
            "the destination already exists"
        );
        let partial = parent.join(format!(
            "{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}",
            unique_suffix()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .context("could not create the partial file")?;
        let cleanup = PartialGuard(Some(partial.clone()));
        let mut hasher = Sha256::new();
        let mut size: u64 = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("the file download was interrupted"))?
        {
            size += chunk.len() as u64;
            ensure!(
                size <= self.max_receive_bytes,
                "the file exceeds the configured receive limit"
            );
            file.write_all(&chunk).context("could not write the file")?;
            hasher.update(&chunk);
        }
        if let Some(expected) = reference.size.or(content_length) {
            ensure!(expected == size, "file metadata did not match the download");
        }
        file.sync_all().context("could not flush the file")?;
        drop(file);
        // Re-verify the parent chain right before publishing.
        let parent_again = paths::prepare_parent_dirs(&self.root, &destination)?;
        ensure!(
            parent_again == parent,
            "destination changed during download"
        );
        // `hard_link` fails if the destination exists: no overwrite, ever.
        std::fs::hard_link(&partial, &final_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                anyhow!("the destination already exists")
            } else {
                anyhow!("could not publish the file: {error}")
            }
        })?;
        drop(cleanup);
        let canonical = paths::strip_verbatim(
            std::fs::canonicalize(&final_path).context("could not verify the saved file")?,
        );
        if !paths::is_inside(&canonical, &self.root) {
            let _ = std::fs::remove_file(&final_path);
            bail!("the saved file resolved outside the inbox");
        }
        Ok(Received {
            path: destination.display(),
            absolute_path: canonical.to_string_lossy().to_string(),
            size,
            sha256: format!("sha256:{}", hex(&hasher.finalize())),
            mime_type,
        })
    }

    /// Read one existing file from the inbox or an outbox for embedding.
    pub(crate) fn read_for_send(
        &self,
        path: &str,
        root: Option<usize>,
    ) -> Result<(String, Vec<u8>, String)> {
        let rel = SafeRelativePath::parse(path).map_err(|error| anyhow!("path: {error}"))?;
        let base = match root {
            None | Some(0) => &self.root,
            Some(index) => self.outboxes.get(index - 1).context("unknown root index")?,
        };
        let canonical = paths::resolve_existing_no_links(base, &rel)?;
        let metadata = std::fs::metadata(&canonical)?;
        ensure!(metadata.is_file(), "path is not a regular file");
        ensure!(
            metadata.len() <= self.max_send_bytes,
            "the file exceeds the configured send limit"
        );
        let bytes = std::fs::read(&canonical)?;
        ensure!(
            bytes.len() as u64 <= self.max_send_bytes,
            "the file exceeds the configured send limit"
        );
        let mime = mime_for_name(rel.file_name()).to_string();
        Ok((rel.file_name().to_string(), bytes, mime))
    }
}

struct PartialGuard(Option<PathBuf>);

impl Drop for PartialGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let digest = Sha256::digest(format!("{}:{nanos}", std::process::id()).as_bytes());
    hex(&digest[..8])
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn extension_for(mime: Option<&str>) -> Option<&'static str> {
    Some(match mime?.split(';').next()?.trim() {
        "application/pdf" => ".pdf",
        "application/zip" => ".zip",
        "application/json" => ".json",
        "text/plain" => ".txt",
        "text/markdown" => ".md",
        "text/csv" => ".csv",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        _ => return None,
    })
}

pub(crate) fn mime_for_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit('.').next().unwrap_or_default();
    match extension {
        "zip" => "application/zip",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
}

/// Standard base64 (RFC 4648) without a new dependency.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::get;

    use std::path::Path;

    use super::*;

    async fn serve() -> SocketAddr {
        let app = Router::new()
            .route("/ok", get(|| async { vec![7u8; 1000] }))
            .route(
                "/redirect-out",
                get(|| async {
                    let mut headers = HeaderMap::new();
                    headers.insert(header::LOCATION, "https://evil.example/x".parse().unwrap());
                    (StatusCode::FOUND, headers).into_response()
                }),
            )
            .route(
                "/redirect-in",
                get(|| async {
                    let mut headers = HeaderMap::new();
                    headers.insert(header::LOCATION, "/ok".parse().unwrap());
                    (StatusCode::FOUND, headers).into_response()
                }),
            )
            .route("/big", get(|| async { vec![1u8; 5000] }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr
    }

    fn inbox(dir: &Path, limit: u64) -> Inbox {
        Inbox {
            root: paths::canonical_dir(dir, "inbox").unwrap(),
            outboxes: Vec::new(),
            max_receive_bytes: limit,
            max_send_bytes: 2000,
            trusted: TrustedHosts::LoopbackForTests,
        }
    }

    fn reference(addr: SocketAddr, route: &str, name: Option<&str>) -> FileReference {
        FileReference {
            download_url: format!("http://{addr}{route}"),
            file_id: "file-abc123".to_string(),
            mime_type: None,
            file_name: name.map(str::to_string),
            name: None,
            size: None,
        }
    }

    #[tokio::test]
    async fn receives_into_inbox_without_overwriting() {
        let addr = serve().await;
        let dir = tempfile::tempdir().unwrap();
        let inbox = inbox(dir.path(), 4096);
        let got = inbox
            .receive(
                &reference(addr, "/ok", Some("report.bin")),
                Some("in/sub/a.bin"),
            )
            .await
            .unwrap();
        assert_eq!(got.path, "in/sub/a.bin");
        assert_eq!(got.size, 1000);
        assert!(got.sha256.starts_with("sha256:"));
        assert_eq!(
            std::fs::read(dir.path().join("in/sub/a.bin")).unwrap(),
            vec![7u8; 1000]
        );
        let again = inbox
            .receive(&reference(addr, "/ok", None), Some("in/sub/a.bin"))
            .await;
        assert!(again.unwrap_err().to_string().contains("already exists"));
        // Default name from the host's file name, then from the file id.
        let named = inbox
            .receive(&reference(addr, "/ok", Some("../x/evil.txt")), None)
            .await
            .unwrap();
        assert_eq!(named.path, "evil.txt");
        let by_id = inbox
            .receive(&reference(addr, "/ok", None), None)
            .await
            .unwrap();
        assert_eq!(by_id.path, "file-abc123.bin");
        // Redirect within the trusted host is followed.
        let redirected = inbox
            .receive(&reference(addr, "/redirect-in", None), Some("r.bin"))
            .await
            .unwrap();
        assert_eq!(redirected.size, 1000);
        // No partial files are left behind.
        let leftovers: Vec<_> = walk(dir.path())
            .into_iter()
            .filter(|p| p.to_string_lossy().contains(PARTIAL_PREFIX))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[tokio::test]
    async fn rejects_untrusted_hosts_limits_and_unsafe_paths() {
        let addr = serve().await;
        let dir = tempfile::tempdir().unwrap();
        let inbox = inbox(dir.path(), 4096);
        assert!(
            inbox
                .receive(&reference(addr, "/redirect-out", None), Some("x"))
                .await
                .is_err()
        );
        assert!(
            inbox
                .receive(&reference(addr, "/big", None), Some("big"))
                .await
                .is_err()
        );
        let small = self::inbox(dir.path(), 100);
        let error = small
            .receive(&reference(addr, "/ok", None), Some("toolarge"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("limit"));
        assert!(!dir.path().join("toolarge").exists());
        for bad in ["../x", "/abs", "C:\\x", "con", "a:b"] {
            assert!(
                inbox
                    .receive(&reference(addr, "/ok", None), Some(bad))
                    .await
                    .is_err(),
                "{bad}"
            );
        }
        let mut wrong_size = reference(addr, "/ok", None);
        wrong_size.size = Some(999);
        assert!(inbox.receive(&wrong_size, Some("ws")).await.is_err());
        let production = Inbox {
            trusted: TrustedHosts::OpenAi,
            ..inbox.clone()
        };
        assert!(
            production
                .receive(&reference(addr, "/ok", None), Some("p"))
                .await
                .is_err()
        );
    }

    #[test]
    fn openai_host_rules_match_workbridge() {
        let check = |url: &str| {
            TrustedHosts::OpenAi
                .check(&Url::parse(url).unwrap())
                .is_ok()
        };
        assert!(check("https://files.oaiusercontent.com/file-1?sig=x"));
        assert!(check("https://oaisdmntprabc123.blob.core.windows.net/c/x"));
        assert!(!check("http://files.oaiusercontent.com/x"));
        assert!(!check("https://files.oaiusercontent.com:8443/x"));
        assert!(!check("https://evil.blob.core.windows.net/x"));
        assert!(!check("https://oaisdmntpr.blob.core.windows.net/x"));
        // URL user info (built at runtime so the source has no email-shaped text).
        assert!(!check(&format!(
            "https://user:pw{}files.oaiusercontent.com/x",
            '@'
        )));
        assert!(!check("https://files.oaiusercontent.com.evil.example/x"));
    }

    #[test]
    fn send_reads_only_confined_regular_files_within_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.zip"), b"PK").unwrap();
        std::fs::write(dir.path().join("big.bin"), vec![0u8; 3000]).unwrap();
        let inbox = inbox(dir.path(), 4096);
        let (name, bytes, mime) = inbox.read_for_send("a.zip", None).unwrap();
        assert_eq!(
            (name.as_str(), bytes.as_slice(), mime.as_str()),
            ("a.zip", &b"PK"[..], "application/zip")
        );
        assert!(inbox.read_for_send("big.bin", None).is_err());
        assert!(inbox.read_for_send("../a.zip", None).is_err());
        assert!(inbox.read_for_send("missing", None).is_err());
        assert!(inbox.read_for_send("a.zip", Some(1)).is_err());
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(input.as_bytes()), expected);
        }
    }
}
