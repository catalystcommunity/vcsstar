//! GitHub as a mirror target: REST calls for repository metadata and the
//! classification of failures into transient (park and retry) and terminal
//! (blocked action with the exact error). PRs, issues, wiki, and projects
//! are never touched (plan/06).

use std::time::Duration;

use serde_json::{json, Value as Json};

use crate::{Code, Error};

/// The three outcomes the mirror worker distinguishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// GitHub or the network is unhealthy; retry with backoff.
    Transient(String),
    /// The credential cannot do this, or GitHub refuses the change; park
    /// the action as blocked with the exact error for an admin.
    Terminal(String),
}

impl Failure {
    pub fn message(&self) -> &str {
        match self {
            Failure::Transient(m) | Failure::Terminal(m) => m,
        }
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
}

/// Minimal view of a GitHub repository.
#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub full_name: String,
    pub default_branch: Option<String>,
    pub private: bool,
    pub archived: bool,
    pub description: Option<String>,
    pub has_issues: bool,
    pub has_wiki: bool,
    pub owner_type: String,
}

impl Client {
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(30))
                .build(),
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    fn req(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{}{path}", self.base))
            .set("Accept", "application/vnd.github+json")
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("User-Agent", "vcsstar-mirror/0.1")
            .set("X-GitHub-Api-Version", "2022-11-28")
    }

    fn call(&self, method: &str, path: &str, body: Option<&Json>) -> Result<(u16, Json), Failure> {
        let r = self.req(method, path);
        let result = match body {
            Some(b) => r
                .set("Content-Type", "application/json")
                .send_string(&b.to_string()),
            None => r.call(),
        };
        match result {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.into_string().unwrap_or_default();
                let json = serde_json::from_str(&text).unwrap_or(Json::Null);
                Ok((status, json))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let retry_after = resp.header("Retry-After").map(|s| s.to_string());
                let text = resp.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<Json>(&text)
                    .ok()
                    .and_then(|j| j.get("message").and_then(Json::as_str).map(str::to_string))
                    .unwrap_or_else(|| text.chars().take(300).collect());
                let detail = format!("GitHub {method} {path}: HTTP {code}: {msg}");
                if code >= 500 || code == 429 || (code == 403 && retry_after.is_some()) {
                    Err(Failure::Transient(detail))
                } else if code == 404 {
                    Ok((404, Json::Null))
                } else {
                    Err(Failure::Terminal(detail))
                }
            }
            Err(e) => Err(Failure::Transient(format!("GitHub {method} {path}: {e}"))),
        }
    }

    pub fn get_repo(&self, full_name: &str) -> Result<Option<RepoInfo>, Failure> {
        let (status, j) = self.call("GET", &format!("/repos/{full_name}"), None)?;
        if status == 404 {
            return Ok(None);
        }
        Ok(Some(parse_repo(&j)))
    }

    /// Create the repository under an org (or the token's user). Never sets
    /// has_issues/wiki/projects: those stay the owner's choice.
    pub fn create_repo(
        &self,
        owner: &str,
        name: &str,
        description: Option<&str>,
        private: bool,
    ) -> Result<RepoInfo, Failure> {
        let body = json!({
            "name": name,
            "description": description.unwrap_or(""),
            "private": private,
            "auto_init": false,
        });
        // Try the org endpoint first; a user owner answers 404 there.
        match self.call("POST", &format!("/orgs/{owner}/repos"), Some(&body)) {
            Ok((404, _)) => {
                let (_, j) = self.call("POST", "/user/repos", Some(&body))?;
                Ok(parse_repo(&j))
            }
            Ok((_, j)) => Ok(parse_repo(&j)),
            Err(e) => Err(e),
        }
    }

    pub fn patch_repo(&self, full_name: &str, patch: &Json) -> Result<RepoInfo, Failure> {
        let (status, j) = self.call("PATCH", &format!("/repos/{full_name}"), Some(patch))?;
        if status == 404 {
            return Err(Failure::Terminal(format!(
                "GitHub repository {full_name} not found"
            )));
        }
        Ok(parse_repo(&j))
    }

    /// Verify the token works at all (used by set-credential).
    pub fn whoami(&self) -> Result<String, Failure> {
        let (_, j) = self.call("GET", "/user", None)?;
        Ok(j.get("login")
            .and_then(Json::as_str)
            .unwrap_or("unknown")
            .to_string())
    }
}

fn parse_repo(j: &Json) -> RepoInfo {
    RepoInfo {
        full_name: j
            .get("full_name")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string(),
        default_branch: j
            .get("default_branch")
            .and_then(Json::as_str)
            .map(str::to_string),
        private: j.get("private").and_then(Json::as_bool).unwrap_or(false),
        archived: j.get("archived").and_then(Json::as_bool).unwrap_or(false),
        description: j
            .get("description")
            .and_then(Json::as_str)
            .map(str::to_string),
        has_issues: j.get("has_issues").and_then(Json::as_bool).unwrap_or(false),
        has_wiki: j.get("has_wiki").and_then(Json::as_bool).unwrap_or(false),
        owner_type: j
            .pointer("/owner/type")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

/// Classify `git push` / `git ls-remote` / `git fetch` output.
pub fn classify_git(stderr: &str, stdout: &str) -> Failure {
    let all = format!("{stdout}\n{stderr}");
    let lower = all.to_lowercase();
    let transient = [
        "could not resolve host",
        "connection timed out",
        "connection refused",
        "connection reset",
        "operation timed out",
        "returned error: 500",
        "returned error: 502",
        "returned error: 503",
        "returned error: 504",
        "returned error: 429",
        "unexpected disconnect",
        "early eof",
        "the remote end hung up",
        "rpc failed",
        "network is unreachable",
        "temporary failure",
        "ssl",
    ];
    if transient.iter().any(|t| lower.contains(t)) {
        return Failure::Transient(trim(&all));
    }
    if lower.contains("could not read username") || lower.contains("authentication failed") {
        return Failure::Terminal(format!(
            "GitHub refused the credential (HTTP 401/403): {}",
            trim(&all)
        ));
    }
    Failure::Terminal(trim(&all))
}

fn trim(s: &str) -> String {
    let t: String = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    t.chars().take(800).collect()
}

impl From<Failure> for Error {
    fn from(f: Failure) -> Self {
        match f {
            Failure::Transient(m) => Error::new(Code::Unavailable, m),
            Failure::Terminal(m) => Error::new(Code::PreconditionFailed, m),
        }
    }
}

/// HMAC-SHA256 check of a GitHub webhook (`X-Hub-Signature-256`).
pub fn verify_webhook_signature(secret: &[u8], body: &[u8], header: &str) -> bool {
    use hmac::Mac;
    let Some(hex_sig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(given) = hex::decode(hex_sig.trim()) else {
        return false;
    };
    let mut mac = match hmac::Hmac::<sha2::Sha256>::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&given).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert!(matches!(
            classify_git(
                "fatal: unable to access: Could not resolve host: github.com",
                ""
            ),
            Failure::Transient(_)
        ));
        assert!(matches!(
            classify_git("remote: Permission to x/y.git denied to bot.\nfatal: unable to access: The requested URL returned error: 403", ""),
            Failure::Terminal(_)
        ));
        assert!(matches!(
            classify_git("", "!\trefs/heads/main:refs/heads/main\t[remote rejected] (protected branch hook declined)"),
            Failure::Terminal(_)
        ));
    }

    #[test]
    fn webhook_signature() {
        // Example from GitHub's docs.
        let ok = verify_webhook_signature(
            b"It's a Secret to Everybody",
            b"Hello, World!",
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17",
        );
        assert!(ok);
        assert!(!verify_webhook_signature(
            b"wrong",
            b"Hello, World!",
            "sha256=757107ea"
        ));
    }
}
