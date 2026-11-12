//! GitHub as a mirror target: REST calls for repository metadata, pull
//! requests, reviews, comments, and commit statuses, and the classification
//! of failures into transient (park and retry) and terminal (blocked action
//! with the exact error). Issues, wiki, and projects are never touched
//! (plan/06); pull requests sync both ways through `github_sync`.

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
    /// Label the rate limit of this token is tracked under (never the value).
    label: String,
}

/// Exchange an App JWT for an installation access token
/// (`POST /app/installations/{id}/access_tokens`). Returns the token and
/// its expiry. 401 and 404 are terminal: the App key or installation id
/// is wrong and a retry cannot help.
pub fn installation_token(
    base: &str,
    jwt: &str,
    installation_id: &str,
) -> Result<(String, chrono::DateTime<chrono::Utc>), Failure> {
    let client = Client::new(base, jwt);
    let path = format!("/app/installations/{installation_id}/access_tokens");
    let (status, j) = client.call("POST", &path, Some(&json!({})))?;
    if status == 404 {
        return Err(Failure::Terminal(format!(
            "GitHub App installation {installation_id} not found (is the App installed on the owner?)"
        )));
    }
    let token = j
        .get("token")
        .and_then(Json::as_str)
        .ok_or_else(|| Failure::Terminal("GitHub returned no installation token".into()))?;
    let expires_at = j
        .get("expires_at")
        .and_then(Json::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::minutes(55));
    Ok((token.to_string(), expires_at))
}

/// The installation of the App on an owner (organization or user),
/// `GET /orgs/{owner}/installation` then `GET /users/{owner}/installation`
/// with the App JWT. `None` when the App is not installed there.
pub fn installation_for_owner(base: &str, jwt: &str, owner: &str) -> Result<Option<i64>, Failure> {
    let client = Client::new(base, jwt).with_label("app-jwt");
    for path in [
        format!("/orgs/{owner}/installation"),
        format!("/users/{owner}/installation"),
    ] {
        let (status, j) = client.call("GET", &path, None)?;
        if status != 404 {
            return Ok(j.get("id").and_then(Json::as_i64));
        }
    }
    Ok(None)
}

/// One page of the App's installations (`GET /app/installations` with the
/// App JWT), with `more` when the page was full.
pub fn list_app_installations(
    base: &str,
    jwt: &str,
    page: u32,
) -> Result<(Vec<Json>, bool), Failure> {
    let client = Client::new(base, jwt).with_label("app-jwt");
    client.page(&format!("/app/installations?per_page=100&page={page}"), 100)
}

/// The installation of the App on the owner of `full_name`
/// (`GET /repos/{owner}/{repo}/installation` with the App JWT). `None`
/// when the App is not installed there, or not for that repository.
pub fn installation_for_repo(
    base: &str,
    jwt: &str,
    full_name: &str,
) -> Result<Option<i64>, Failure> {
    let client = Client::new(base, jwt);
    let (status, j) = client.call("GET", &format!("/repos/{full_name}/installation"), None)?;
    if status == 404 {
        return Ok(None);
    }
    Ok(j.get("id").and_then(Json::as_i64))
}

/// Tokens from GitHub's OAuth token endpoint. A GitHub App with expiring
/// user tokens answers with a refresh token too; a classic grant has
/// neither expiry nor refresh token.
#[derive(Debug, Clone)]
pub struct OauthTokens {
    pub access_token: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub refresh_token: Option<String>,
    pub refresh_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Exchange an authorization code (`POST /login/oauth/access_token`).
pub fn oauth_exchange(
    web_base: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<OauthTokens, Failure> {
    oauth_token_call(
        web_base,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ],
    )
}

/// Refresh an expiring user token. GitHub rotates the refresh token.
pub fn oauth_refresh(
    web_base: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<OauthTokens, Failure> {
    oauth_token_call(
        web_base,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )
}

fn oauth_token_call(web_base: &str, form: &[(&str, &str)]) -> Result<OauthTokens, Failure> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let url = format!(
        "{}/login/oauth/access_token",
        web_base.trim_end_matches('/')
    );
    let result = agent
        .post(&url)
        .set("Accept", "application/json")
        .set("User-Agent", "vcsstar-mirror/0.1")
        .send_form(form);
    let text = match result {
        Ok(resp) => resp.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            let msg: String = text.chars().take(300).collect();
            return Err(if code >= 500 {
                Failure::Transient(format!("GitHub OAuth token endpoint: HTTP {code}: {msg}"))
            } else {
                Failure::Terminal(format!("GitHub OAuth token endpoint: HTTP {code}: {msg}"))
            });
        }
        Err(e) => {
            return Err(Failure::Transient(format!(
                "GitHub OAuth token endpoint: {e}"
            )))
        }
    };
    let j: Json = serde_json::from_str(&text).map_err(|_| {
        Failure::Terminal("GitHub OAuth token endpoint answered without JSON".into())
    })?;
    // Errors come back with HTTP 200 and an `error` field.
    if let Some(err) = j.get("error").and_then(Json::as_str) {
        let desc = j
            .get("error_description")
            .and_then(Json::as_str)
            .unwrap_or("");
        return Err(Failure::Terminal(format!("GitHub OAuth: {err}: {desc}")));
    }
    let access_token = j
        .get("access_token")
        .and_then(Json::as_str)
        .ok_or_else(|| Failure::Terminal("GitHub OAuth answered without an access token".into()))?
        .to_string();
    let now = chrono::Utc::now();
    let secs = |k: &str| j.get(k).and_then(Json::as_i64);
    Ok(OauthTokens {
        access_token,
        expires_at: secs("expires_in").map(|n| now + chrono::Duration::seconds(n)),
        refresh_token: j
            .get("refresh_token")
            .and_then(Json::as_str)
            .map(str::to_string),
        refresh_expires_at: secs("refresh_token_expires_in")
            .map(|n| now + chrono::Duration::seconds(n)),
    })
}

/// Revoke a user's authorization of the App
/// (`DELETE /applications/{client_id}/grant`, basic auth with the client
/// secret). Best effort: a token GitHub no longer knows answers 404 and
/// that is fine.
pub fn revoke_grant(
    base: &str,
    client_id: &str,
    client_secret: &str,
    access_token: &str,
) -> Result<(), Failure> {
    use base64::Engine;
    let basic =
        base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{client_secret}"));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let url = format!(
        "{}/applications/{client_id}/grant",
        base.trim_end_matches('/')
    );
    let r = agent
        .delete(&url)
        .set("Accept", "application/vnd.github+json")
        .set("Authorization", &format!("Basic {basic}"))
        .set("User-Agent", "vcsstar-mirror/0.1")
        .set("Content-Type", "application/json")
        .send_string(&json!({ "access_token": access_token }).to_string());
    match r {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(404 | 422, _)) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Err(Failure::Terminal(format!(
                "GitHub DELETE grant: HTTP {code}: {}",
                text.chars().take(200).collect::<String>()
            )))
        }
        Err(e) => Err(Failure::Transient(format!("GitHub DELETE grant: {e}"))),
    }
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
            label: crate::github_health::token_label(token),
        }
    }

    /// Name the token for rate limit tracking (`app`, `user:<login>`, a
    /// credential purpose).
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether calls with this token should wait for its rate limit window.
    pub fn rate_hold(&self) -> Option<Duration> {
        crate::github_health::rate_hold(&self.label)
    }

    fn note_headers(&self, resp: &ureq::Response) {
        let num = |h: &str| resp.header(h).and_then(|v| v.trim().parse::<u64>().ok());
        if let (Some(remaining), Some(limit), Some(reset)) = (
            num("x-ratelimit-remaining"),
            num("x-ratelimit-limit"),
            num("x-ratelimit-reset"),
        ) {
            let reset_at =
                chrono::DateTime::from_timestamp(reset as i64, 0).unwrap_or_else(chrono::Utc::now);
            crate::github_health::note_rate_limit(&self.label, remaining, limit, reset_at);
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
        use crate::github_health::{self as health, Lane};
        let r = self.req(method, path);
        let result = match body {
            Some(b) => r
                .set("Content-Type", "application/json")
                .send_string(&b.to_string()),
            None => r.call(),
        };
        match result {
            Ok(resp) => {
                self.note_headers(&resp);
                health::record_success(Lane::Api);
                let status = resp.status();
                let text = resp.into_string().unwrap_or_default();
                let json = serde_json::from_str(&text).unwrap_or(Json::Null);
                Ok((status, json))
            }
            Err(ureq::Error::Status(code, resp)) => {
                self.note_headers(&resp);
                let retry_after = resp
                    .header("Retry-After")
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let remaining = resp
                    .header("x-ratelimit-remaining")
                    .and_then(|v| v.trim().parse::<u64>().ok());
                let text = resp.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<Json>(&text)
                    .ok()
                    .map(|j| error_text(&j))
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| text.chars().take(300).collect());
                let detail = format!("GitHub {method} {path}: HTTP {code}: {msg}");
                let rate_limited = code == 429
                    || (code == 403
                        && (retry_after.is_some()
                            || remaining == Some(0)
                            || msg.to_ascii_lowercase().contains("rate limit")));
                if rate_limited {
                    // Primary limit: the window reset is in the headers
                    // (noted above). Secondary limit: Retry-After, or a
                    // minute when GitHub names none.
                    let wait = Duration::from_secs(retry_after.unwrap_or(60));
                    health::note_retry_after(&self.label, wait);
                    health::record_failure(Lane::Api, &detail);
                    Err(Failure::Transient(detail))
                } else if code >= 500 {
                    health::record_failure(Lane::Api, &detail);
                    Err(Failure::Transient(detail))
                } else if code == 404 && method == "GET" {
                    // GitHub is up: a 4xx is an answer, not a failure of
                    // the lane.
                    health::record_success(Lane::Api);
                    Ok((404, Json::Null))
                } else {
                    // A 404 on a write is GitHub hiding a repository the
                    // token cannot see, or a target that is gone: both are
                    // terminal, and the message says which.
                    health::record_success(Lane::Api);
                    Err(Failure::Terminal(detail))
                }
            }
            Err(e) => {
                let detail = format!("GitHub {method} {path}: {e}");
                health::record_failure(Lane::Api, &detail);
                Err(Failure::Transient(detail))
            }
        }
    }

    /// One page of a list endpoint; `more` when the page was full.
    fn page(&self, path: &str, per_page: usize) -> Result<(Vec<Json>, bool), Failure> {
        let (_, j) = self.call("GET", path, None)?;
        let items = j.as_array().cloned().unwrap_or_default();
        let more = items.len() >= per_page;
        Ok((items, more))
    }

    /// Repositories the App installation this token belongs to can see
    /// (`GET /installation/repositories`), one page.
    pub fn list_installation_repos(
        &self,
        page: u32,
        per_page: usize,
    ) -> Result<(Vec<Json>, bool), Failure> {
        let (_, j) = self.call(
            "GET",
            &format!("/installation/repositories?per_page={per_page}&page={page}"),
            None,
        )?;
        let items = j
            .get("repositories")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();
        let more = items.len() >= per_page;
        Ok((items, more))
    }

    /// Repositories of an owner visible to this token, one page. An
    /// organization answers on `/orgs`, a user on `/users`.
    pub fn list_owner_repos(
        &self,
        owner: &str,
        page: u32,
        per_page: usize,
    ) -> Result<(Vec<Json>, bool), Failure> {
        let path =
            format!("/orgs/{owner}/repos?type=all&sort=full_name&per_page={per_page}&page={page}");
        let (status, j) = self.call("GET", &path, None)?;
        if status != 404 {
            let items = j.as_array().cloned().unwrap_or_default();
            let more = items.len() >= per_page;
            return Ok((items, more));
        }
        self.page(
            &format!("/users/{owner}/repos?per_page={per_page}&page={page}"),
            per_page,
        )
    }

    /// Pull requests sorted by `updated_at`, newest first, one page.
    pub fn list_pulls(
        &self,
        full_name: &str,
        state: &str,
        page: u32,
        per_page: usize,
    ) -> Result<(Vec<Json>, bool), Failure> {
        self.page(
            &format!("/repos/{full_name}/pulls?state={state}&sort=updated&direction=desc&per_page={per_page}&page={page}"),
            per_page,
        )
    }

    /// Pull requests in creation order, oldest first, one page. The list
    /// only grows at its end, so a (page, index) position stays valid
    /// between passes (the history import's cursor).
    pub fn list_pulls_created(
        &self,
        full_name: &str,
        page: u32,
        per_page: usize,
    ) -> Result<(Vec<Json>, bool), Failure> {
        self.page(
            &format!("/repos/{full_name}/pulls?state=all&sort=created&direction=asc&per_page={per_page}&page={page}"),
            per_page,
        )
    }

    /// Reviews of a pull request, one page of 100.
    pub fn list_reviews(
        &self,
        full_name: &str,
        number: i64,
        page: u32,
    ) -> Result<(Vec<Json>, bool), Failure> {
        self.page(
            &format!("/repos/{full_name}/pulls/{number}/reviews?per_page=100&page={page}"),
            100,
        )
    }

    /// Conversation comments of a pull request, optionally only those
    /// updated at or after `since` (RFC 3339), one page of 100.
    pub fn list_issue_comments(
        &self,
        full_name: &str,
        number: i64,
        since: Option<&str>,
        page: u32,
    ) -> Result<(Vec<Json>, bool), Failure> {
        let mut path =
            format!("/repos/{full_name}/issues/{number}/comments?per_page=100&page={page}");
        if let Some(s) = since {
            path.push_str(&format!("&since={}", url_encode(s)));
        }
        self.page(&path, 100)
    }

    /// Inline review comments of a pull request, optionally since a time,
    /// one page of 100.
    pub fn list_review_comments(
        &self,
        full_name: &str,
        number: i64,
        since: Option<&str>,
        page: u32,
    ) -> Result<(Vec<Json>, bool), Failure> {
        let mut path =
            format!("/repos/{full_name}/pulls/{number}/comments?per_page=100&page={page}");
        if let Some(s) = since {
            path.push_str(&format!("&since={}", url_encode(s)));
        }
        self.page(&path, 100)
    }

    /// The commit statuses of a sha (`statuses` of the combined status).
    pub fn combined_status(&self, full_name: &str, sha: &str) -> Result<Vec<Json>, Failure> {
        let (status, j) = self.call(
            "GET",
            &format!("/repos/{full_name}/commits/{sha}/status?per_page=100"),
            None,
        )?;
        if status == 404 {
            return Ok(vec![]);
        }
        Ok(j.get("statuses")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Check runs of a sha (needs the App's Checks permission; an empty
    /// list otherwise).
    pub fn check_runs(&self, full_name: &str, sha: &str) -> Result<Vec<Json>, Failure> {
        match self.call(
            "GET",
            &format!("/repos/{full_name}/commits/{sha}/check-runs?per_page=100"),
            None,
        ) {
            Ok((404, _)) => Ok(vec![]),
            Ok((_, j)) => Ok(j
                .get("check_runs")
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default()),
            Err(Failure::Terminal(m)) if m.contains("HTTP 403") => Ok(vec![]),
            Err(e) => Err(e),
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

    // ----- pull requests, reviews, comments, statuses (github_sync) -----

    /// One pull request, or `None` when the number is unknown.
    pub fn get_pull(&self, full_name: &str, number: i64) -> Result<Option<Json>, Failure> {
        let (status, j) = self.call("GET", &format!("/repos/{full_name}/pulls/{number}"), None)?;
        Ok((status != 404).then_some(j))
    }

    /// Open pull requests from `head` (`owner:branch`) into `base`, to find
    /// one that an earlier attempt created before the link was stored.
    pub fn find_pulls(
        &self,
        full_name: &str,
        head: &str,
        base: &str,
    ) -> Result<Vec<Json>, Failure> {
        let path = format!(
            "/repos/{full_name}/pulls?state=open&head={}&base={}&per_page=10",
            url_encode(head),
            url_encode(base)
        );
        let (_, j) = self.call("GET", &path, None)?;
        Ok(j.as_array().cloned().unwrap_or_default())
    }

    /// Open pull requests whose base is `base`.
    pub fn pulls_by_base(&self, full_name: &str, base: &str) -> Result<Vec<Json>, Failure> {
        let path = format!(
            "/repos/{full_name}/pulls?state=open&base={}&per_page=100",
            url_encode(base)
        );
        let (_, j) = self.call("GET", &path, None)?;
        Ok(j.as_array().cloned().unwrap_or_default())
    }

    pub fn create_pull(
        &self,
        full_name: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<Json, Failure> {
        let (_, j) = self.call(
            "POST",
            &format!("/repos/{full_name}/pulls"),
            Some(&json!({ "title": title, "body": body, "head": head, "base": base })),
        )?;
        Ok(j)
    }

    pub fn patch_pull(&self, full_name: &str, number: i64, patch: &Json) -> Result<Json, Failure> {
        let (status, j) = self.call(
            "PATCH",
            &format!("/repos/{full_name}/pulls/{number}"),
            Some(patch),
        )?;
        if status == 404 {
            return Err(Failure::Terminal(format!(
                "GitHub pull request {full_name}#{number} not found"
            )));
        }
        Ok(j)
    }

    /// A conversation comment. Returns the comment id.
    pub fn create_issue_comment(
        &self,
        full_name: &str,
        number: i64,
        body: &str,
    ) -> Result<i64, Failure> {
        let (_, j) = self.call(
            "POST",
            &format!("/repos/{full_name}/issues/{number}/comments"),
            Some(&json!({ "body": body })),
        )?;
        id_of(&j, "comment")
    }

    pub fn patch_issue_comment(&self, full_name: &str, id: i64, body: &str) -> Result<(), Failure> {
        self.call(
            "PATCH",
            &format!("/repos/{full_name}/issues/comments/{id}"),
            Some(&json!({ "body": body })),
        )?;
        Ok(())
    }

    /// An inline comment on a diff line of the pull request's head commit.
    pub fn create_review_comment(
        &self,
        full_name: &str,
        number: i64,
        body: &str,
        commit_id: &str,
        path: &str,
        line: i64,
    ) -> Result<i64, Failure> {
        let (_, j) = self.call(
            "POST",
            &format!("/repos/{full_name}/pulls/{number}/comments"),
            Some(&json!({
                "body": body, "commit_id": commit_id, "path": path,
                "line": line, "side": "RIGHT",
            })),
        )?;
        id_of(&j, "review comment")
    }

    pub fn patch_review_comment(
        &self,
        full_name: &str,
        id: i64,
        body: &str,
    ) -> Result<(), Failure> {
        self.call(
            "PATCH",
            &format!("/repos/{full_name}/pulls/comments/{id}"),
            Some(&json!({ "body": body })),
        )?;
        Ok(())
    }

    /// Submit a review: `event` is APPROVE, REQUEST_CHANGES, or COMMENT.
    pub fn create_review(
        &self,
        full_name: &str,
        number: i64,
        event: &str,
        body: &str,
        commit_id: &str,
    ) -> Result<i64, Failure> {
        let (_, j) = self.call(
            "POST",
            &format!("/repos/{full_name}/pulls/{number}/reviews"),
            Some(&json!({ "event": event, "body": body, "commit_id": commit_id })),
        )?;
        id_of(&j, "review")
    }

    /// A commit status; GitHub keeps the latest per context.
    pub fn create_status(
        &self,
        full_name: &str,
        sha: &str,
        state: &str,
        context: &str,
        target_url: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), Failure> {
        let mut body = json!({ "state": state, "context": context });
        if let Some(u) = target_url {
            body["target_url"] = json!(u);
        }
        if let Some(d) = description {
            body["description"] = json!(d.chars().take(140).collect::<String>());
        }
        self.call(
            "POST",
            &format!("/repos/{full_name}/statuses/{sha}"),
            Some(&body),
        )?;
        Ok(())
    }

    /// Verify the token works at all (used by set-credential).
    pub fn whoami(&self) -> Result<String, Failure> {
        Ok(self.get_user()?.1)
    }

    /// The account behind the token: `(id, login)`.
    pub fn get_user(&self) -> Result<(i64, String), Failure> {
        let (status, j) = self.call("GET", "/user", None)?;
        if status == 404 {
            return Err(Failure::Terminal("GitHub does not know this token".into()));
        }
        let id = id_of(&j, "user")?;
        let login = j
            .get("login")
            .and_then(Json::as_str)
            .unwrap_or("unknown")
            .to_string();
        Ok((id, login))
    }
}

/// GitHub's `message` plus its `errors` list (a 422 keeps the reason
/// there: "Review Can not approve your own pull request").
fn error_text(j: &Json) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = j.get("message").and_then(Json::as_str) {
        parts.push(m.to_string());
    }
    if let Some(errors) = j.get("errors").and_then(Json::as_array) {
        for e in errors {
            match e {
                Json::String(s) => parts.push(s.clone()),
                Json::Object(o) => {
                    let text = o
                        .get("message")
                        .and_then(Json::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            [o.get("resource"), o.get("field"), o.get("code")]
                                .iter()
                                .flatten()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(" ")
                        });
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
                _ => {}
            }
        }
    }
    parts.join(": ").chars().take(500).collect()
}

fn id_of(j: &Json, what: &str) -> Result<i64, Failure> {
    j.get("id")
        .and_then(Json::as_i64)
        .ok_or_else(|| Failure::Terminal(format!("GitHub returned no {what} id")))
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Whether a terminal failure says the token itself was refused (revoked,
/// expired, or without access to the repository), so another credential
/// may still succeed.
pub fn is_auth_refusal(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("http 401")
        || m.contains("bad credentials")
        || m.contains("authentication failed")
        || (m.contains("http 403") && m.contains("not accessible"))
        || m.contains("token expired")
}

/// Whether a terminal failure message is GitHub's refusal of a self-review
/// ("Can not approve your own pull request" and friends).
pub fn is_self_review_refusal(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("own pull request") || m.contains("can not request changes on your own")
}

/// Whether a terminal failure says the commit is not on GitHub yet, which
/// the mirror push of its branch will fix.
pub fn is_missing_commit(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("no commit found")
        || m.contains("commit_id is not part of the pull request")
        || m.contains("head sha can't be blank")
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

/// Classify `git push` / `git ls-remote` / `git fetch` output and record
/// the outcome on the git lane (a transient failure counts against it,
/// a refusal does not).
pub fn classify_git(stderr: &str, stdout: &str) -> Failure {
    let f = classify_git_text(stderr, stdout);
    match &f {
        Failure::Transient(m) => {
            crate::github_health::record_failure(crate::github_health::Lane::Git, m)
        }
        Failure::Terminal(_) => {
            crate::github_health::record_success(crate::github_health::Lane::Git)
        }
    }
    f
}

fn classify_git_text(stderr: &str, stdout: &str) -> Failure {
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

/// Whether a delivery is signed with the webhook secret of one of the
/// instance's GitHub Apps.
pub fn signed_by_app(apps: &[crate::config::GithubAppConfig], body: &[u8], header: &str) -> bool {
    apps.iter().any(|a| {
        a.webhook_secret
            .as_deref()
            .is_some_and(|w| verify_webhook_signature(w.as_bytes(), body, header))
    })
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
            classify_git_text(
                "fatal: unable to access: Could not resolve host: github.com",
                ""
            ),
            Failure::Transient(_)
        ));
        assert!(matches!(
            classify_git_text("remote: Permission to x/y.git denied to bot.\nfatal: unable to access: The requested URL returned error: 403", ""),
            Failure::Terminal(_)
        ));
        assert!(matches!(
            classify_git_text("", "!\trefs/heads/main:refs/heads/main\t[remote rejected] (protected branch hook declined)"),
            Failure::Terminal(_)
        ));
    }

    #[test]
    fn auth_refusals_are_recognised() {
        assert!(is_auth_refusal(
            "GitHub GET /user: HTTP 401: Bad credentials"
        ));
        assert!(is_auth_refusal(
            "GitHub POST /repos/o/r/pulls: HTTP 403: Resource not accessible by integration"
        ));
        assert!(is_auth_refusal(
            "remote: Authentication failed for 'https://github.com/o/r.git/'"
        ));
        assert!(!is_auth_refusal(
            "GitHub POST /repos/o/r/pulls: HTTP 422: Validation Failed"
        ));
        assert!(!is_auth_refusal(
            "GitHub POST /x: HTTP 403: API rate limit exceeded"
        ));
    }

    #[test]
    fn error_text_keeps_the_errors_list() {
        let j = serde_json::json!({
            "message": "Unprocessable Entity",
            "errors": ["Review Can not approve your own pull request"],
        });
        assert!(is_self_review_refusal(&error_text(&j)));
        let j = serde_json::json!({
            "message": "Validation Failed",
            "errors": [{"resource": "PullRequest", "field": "base", "code": "invalid",
                        "message": "Cannot change the base branch of a closed pull request."}],
        });
        assert_eq!(
            error_text(&j),
            "Validation Failed: Cannot change the base branch of a closed pull request."
        );
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

    #[test]
    fn delivery_signed_by_any_configured_app() {
        let app = |id: &str, w: Option<&str>| crate::config::GithubAppConfig {
            app_id: id.into(),
            client_id: "c".into(),
            client_secret: "s".into(),
            private_key: "k".into(),
            slug: None,
            webhook_secret: w.map(str::to_string),
        };
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        let apps = vec![app("1", None), app("2", Some("It's a Secret to Everybody"))];
        assert!(signed_by_app(&apps, b"Hello, World!", sig));
        assert!(!signed_by_app(&apps[..1], b"Hello, World!", sig));
        assert!(!signed_by_app(&apps, b"Hello, World?", sig));
    }
}
