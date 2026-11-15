//! Provider adapters. All mutations use the destination's own permissions.
use super::{encoded, local, store::Connection, text, Item};
use crate::{mirror, App, Code, Error};
use diesel::prelude::*;
use diesel::sql_types::{Binary, Text, Uuid as SqlUuid};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug)]
pub struct ApiError {
    pub status: Option<u16>,
    pub message: String,
}
impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Error::new(
            match e.status {
                Some(401) => Code::Unauthenticated,
                Some(403) => Code::Forbidden,
                Some(429) => Code::ResourceExhausted,
                Some(500..=599) | None => Code::Unavailable,
                _ => Code::PreconditionFailed,
            },
            e.message,
        )
    }
}
pub struct Client {
    pub provider: String,
    pub base: String,
    pub token: String,
    pub user: bool,
    label: String,
    agent: ureq::Agent,
}
impl Client {
    pub fn new(app: &App, c: &Connection, token: String, user: bool) -> Self {
        Self {
            provider: c.provider.clone(),
            base: if c.provider == "github" {
                app.config.github_api_base.trim_end_matches('/').into()
            } else {
                format!("https://{}/api/v4", c.host)
            },
            label: crate::github_health::token_label(&token),
            token,
            user,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(30))
                .redirects(0)
                .build(),
        }
    }
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        use crate::github_health::{self as health, Lane};
        if self.provider == "github"
            && (health::rate_hold(&self.label).is_some() || health::hold(Lane::Api).is_some())
        {
            return Err(ApiError {
                status: Some(429),
                message: "GitHub calls are held for an outage or rate limit; sync will retry"
                    .into(),
            });
        }
        let mut request = self
            .agent
            .request(method, &format!("{}{}", self.base, path))
            .set("Accept", "application/json")
            .set("User-Agent", "vcsstar");
        if !self.token.is_empty() {
            request = if self.provider == "github" {
                request.set("Authorization", &format!("Bearer {}", self.token))
            } else {
                request.set("PRIVATE-TOKEN", &self.token)
            };
        }
        let response = match body {
            Some(b) => request
                .set("Content-Type", "application/json")
                .send_string(&b.to_string()),
            None => request.call(),
        };
        if self.provider == "github" {
            match &response {
                Ok(r) | Err(ureq::Error::Status(_, r)) => {
                    let number =
                        |header: &str| r.header(header).and_then(|v| v.parse::<u64>().ok());
                    if let (Some(left), Some(limit), Some(reset)) = (
                        number("x-ratelimit-remaining"),
                        number("x-ratelimit-limit"),
                        number("x-ratelimit-reset"),
                    ) {
                        if let Some(at) = chrono::DateTime::from_timestamp(reset as i64, 0) {
                            health::note_rate_limit(&self.label, left, limit, at);
                        }
                    }
                    if r.status() == 429
                        || (r.status() == 403
                            && (number("retry-after").is_some()
                                || number("x-ratelimit-remaining") == Some(0)))
                    {
                        health::note_retry_after(
                            &self.label,
                            Duration::from_secs(number("retry-after").unwrap_or(60)),
                        );
                        return Err(ApiError {
                            status: Some(429),
                            message: "GitHub rate limit reached; sync will retry".into(),
                        });
                    }
                    if r.status() >= 500 {
                        health::record_failure(
                            Lane::Api,
                            "GitHub server error during connection sync",
                        );
                    } else {
                        health::record_success(Lane::Api);
                    }
                }
                Err(_) => health::record_failure(Lane::Api, "GitHub connection failed during sync"),
            }
        }
        match response {
            Ok(r) => {
                if r.status() == 204 {
                    return Ok(Value::Null);
                }
                let body = r.into_string().map_err(|_| ApiError {
                    status: None,
                    message: "could not read provider response".into(),
                })?;
                if body.is_empty() {
                    Ok(Value::Null)
                } else {
                    serde_json::from_str(&body).map_err(|_| ApiError {
                        status: None,
                        message: "invalid provider JSON response".into(),
                    })
                }
            }
            Err(ureq::Error::Status(status, _)) => Err(ApiError {
                status: Some(status),
                message: format!(
                    "{} {method} {path}: HTTP {status}{}",
                    self.provider,
                    if matches!(status, 401 | 403) {
                        "; credentials lack access. Configure the required permissions in the destination, or perform setup manually."
                    } else {
                        ""
                    }
                ),
            }),
            Err(_) => Err(ApiError {
                status: None,
                message: format!("{} {method} {path}: connection failed", self.provider),
            }),
        }
    }
    pub fn list(&self, path: &str) -> Result<Vec<Value>, Error> {
        let mut result = vec![];
        for page in 1.. {
            let sep = if path.contains('?') { '&' } else { '?' };
            let value = self.call("GET", &format!("{path}{sep}per_page=100&page={page}"), None)?;
            let batch = value
                .as_array()
                .ok_or_else(|| Error::invalid("provider list response is not an array"))?;
            result.extend(batch.iter().cloned());
            if batch.len() < 100 {
                return Ok(result);
            }
        }
        unreachable!()
    }
    pub fn project_path(&self, c: &Connection) -> String {
        if self.provider == "github" {
            format!("/repos/{}", c.project)
        } else {
            format!("/projects/{}", encoded(&c.project))
        }
    }
}

/// Credentials are selected by provider, host, purpose, and repository.
/// A linked user's failure can fall back to the bot, but never changes ACLs.
pub fn client(
    app: &App,
    conn: &mut PgConnection,
    c: &Connection,
    actor: Option<Uuid>,
) -> Result<Client, Error> {
    client_for(app, conn, c, actor, "mirror_api")
}

pub fn client_for(
    app: &App,
    conn: &mut PgConnection,
    c: &Connection,
    actor: Option<Uuid>,
    purpose: &str,
) -> Result<Client, Error> {
    #[derive(QueryableByName)]
    struct Link {
        #[diesel(sql_type=Binary)]
        token: Vec<u8>,
    }
    if let Some(actor) = actor {
        let link:Option<Link>=diesel::sql_query("SELECT token FROM sync_links WHERE identity_id=$1 AND provider=$2 AND host=$3 AND broken_reason IS NULL")
            .bind::<SqlUuid,_>(actor).bind::<Text,_>(&c.provider).bind::<Text,_>(&c.host).get_result(conn).optional()?;
        if let Some(link) = link {
            return Ok(Client::new(
                app,
                c,
                mirror::cred::open(app, &link.token)?,
                true,
            ));
        }
        if c.provider == "github" {
            if let Some((_, token)) = crate::github_link::user_token(app, conn, actor)? {
                return Ok(Client::new(app, c, token, true));
            }
        }
    }
    if c.provider == "github" {
        if let Some(token) = mirror::cred::instance_app_token_for(app, &c.project)? {
            let label = token.label();
            let mut client = Client::new(app, c, token.value, false);
            client.label = label;
            return Ok(client);
        }
    }
    let (_, repo) = local::load_repo(conn, c.repo_id)?;
    use vcsstar_db::schema::credentials as cr;
    let purposes = match purpose {
        "mirror_git_push" => ["mirror_git_push", "mirror_api", "import_fetch"],
        "import_fetch" => ["import_fetch", "mirror_git_push", "mirror_api"],
        _ => ["mirror_api", "mirror_git_push", "import_fetch"],
    };
    for purpose in purposes {
        let rows = cr::table
            .filter(cr::org_id.eq(repo.org_id))
            .filter(cr::provider.eq(&c.provider))
            .filter(cr::host.eq(&c.host))
            .filter(cr::purpose.eq(purpose))
            .filter(cr::enabled.eq(true))
            .filter(cr::repo_id.eq(repo.id).or(cr::repo_id.is_null()))
            .order(cr::repo_id.desc().nulls_last())
            .select(mirror::CredentialRow::as_select())
            .load::<mirror::CredentialRow>(conn)?;
        use rand::seq::SliceRandom;
        let scoped: Vec<_> = if rows.iter().any(|r| r.repo_id == Some(repo.id)) {
            rows.iter().filter(|r| r.repo_id == Some(repo.id)).collect()
        } else {
            rows.iter().collect()
        };
        if let Some(row) = scoped.choose(&mut rand::thread_rng()) {
            let value = mirror::cred::value(app, row)?;
            let value = if row.auth_method == "github_app" {
                crate::github_app::installation_token(app, row.id, &value)?
            } else {
                value
            };
            return Ok(Client::new(app, c, value, false));
        }
    }
    Ok(Client::new(app, c, String::new(), false))
}
fn time(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}
pub fn marker(repo: Uuid, key: &str) -> String {
    format!("<!-- vcsstar-sync:{repo}:{key} -->")
}
fn find_marker(body: &str, repo: Uuid) -> Option<String> {
    let p = format!("<!-- vcsstar-sync:{repo}:");
    let s = body.split_once(&p)?.1;
    Some(s.split_once(" -->")?.0.to_string())
}
fn clean(body: &str) -> String {
    body.split("\n\n<!-- vcsstar-sync-attribution -->")
        .next()
        .unwrap_or(body)
        .lines()
        .filter(|l| !l.starts_with("<!-- vcsstar-sync:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
#[derive(Clone, Debug)]
pub struct RemoteItem {
    pub id: String,
    pub key: Option<String>,
    pub item: Item,
    pub raw: Value,
}
fn make(
    conn: &mut PgConnection,
    c: &Connection,
    id: String,
    kind: &str,
    data: Value,
    raw: &Value,
    body: &str,
) -> Result<RemoteItem, Error> {
    let user = raw
        .get("user")
        .or_else(|| raw.get("author"))
        .or_else(|| raw.get("creator"))
        .filter(|u| u.is_object() && u.get("id").is_some_and(|id| !id.is_null()));
    let actor = match user {
        Some(user) => Some(local::identity(conn, c, user)?.id),
        None if matches!(kind, "pr" | "comment" | "review") => Some(
            local::identity(
                conn,
                c,
                &json!({"id":0,"login":"former-user","username":"former-user"}),
            )?
            .id,
        ),
        None => None,
    };
    let author = user
        .and_then(|u| u.get("username").or_else(|| u.get("login")))
        .and_then(Value::as_str)
        .map(|s| format!("{s}@{}", c.host))
        .unwrap_or_else(|| c.host.clone());
    Ok(RemoteItem {
        id,
        key: find_marker(body, c.repo_id),
        item: Item {
            kind: kind.into(),
            data,
            actor,
            author,
            created_at: if raw.get("created_at").is_some() {
                time(raw, "created_at")
            } else {
                time(raw, "submitted_at")
            },
            url: raw
                .get("web_url")
                .or_else(|| raw.get("html_url"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        raw: raw.clone(),
    })
}

pub fn setup(
    app: &App,
    conn: &mut PgConnection,
    c: &Connection,
    client: &Client,
) -> Result<(), Error> {
    let path = client.project_path(c);
    let mut created = false;
    match client.call("GET", &path, None) {
        Ok(_) => {}
        Err(e) if e.status == Some(404) && c.create_remote => {
            created = true;
            let (namespace, name) = c
                .project
                .rsplit_once('/')
                .ok_or_else(|| Error::invalid("project path"))?;
            if c.provider == "github" {
                let user = client.call("GET", "/user", None).ok();
                let path = if user
                    .as_ref()
                    .is_some_and(|u| text(u, "login").eq_ignore_ascii_case(namespace))
                {
                    "/user/repos".into()
                } else {
                    format!("/orgs/{namespace}/repos")
                };
                client.call("POST", &path, Some(&json!({"name":name,"private":true})))?;
            } else {
                let namespaces = client.list(&format!(
                    "/namespaces?search={}",
                    encoded(namespace.rsplit('/').next().unwrap_or(namespace))
                ))?;
                let id=namespaces.iter().find(|n|text(n,"full_path")==namespace).and_then(|n|n.get("id")).ok_or_else(||Error::forbidden("namespace is not available to this token; create the project manually or grant project creation permission"))?;
                client.call(
                    "POST",
                    "/projects",
                    Some(
                        &json!({"name":name,"path":name,"namespace_id":id,"visibility":"private"}),
                    ),
                )?;
            }
        }
        Err(e) => return Err(e.into()),
    }
    if created {
        let info = client.call("GET", &path, None)?;
        let remote = Item {
            kind: "settings".into(),
            data: json!({"description":text(&info,"description"),"default_branch":info.get("default_branch").and_then(Value::as_str).unwrap_or("main"),"archived":info.get("archived").and_then(Value::as_bool).unwrap_or(false)}),
            actor: None,
            author: c.host.clone(),
            created_at: 0,
            url: None,
        };
        super::store::save_replica(
            conn,
            c.id,
            "settings",
            Some("settings"),
            None,
            Some(&remote),
            "pending",
            None,
        )?;
    }
    if c.automatic_webhook {
        let url = c.webhook_url(app);
        let hooks = client.list(&format!("{path}/hooks"))?;
        let secret = mirror::cred::open(app, &c.webhook_secret)?;
        let body = if c.provider == "github" {
            json!({"name":"web","active":true,"events":["push","pull_request","pull_request_review","pull_request_review_comment","issue_comment","status","check_run","repository"],"config":{"url":url,"content_type":"json","secret":secret,"insecure_ssl":"0"}})
        } else {
            json!({"url":url,"token":secret,"push_events":true,"tag_push_events":true,"merge_requests_events":true,"note_events":true,"pipeline_events":true,"job_events":true,"enable_ssl_verification":true})
        };
        if let Some(hook) = hooks.iter().find(|h| {
            h.pointer("/config/url")
                .or_else(|| h.get("url"))
                .and_then(Value::as_str)
                == Some(url.as_str())
        }) {
            client.call(
                if c.provider == "github" {
                    "PATCH"
                } else {
                    "PUT"
                },
                &format!("{path}/hooks/{}", hook["id"]),
                Some(&body),
            )?;
        } else {
            client.call("POST", &format!("{path}/hooks"), Some(&body))?;
        }
    }
    diesel::sql_query("UPDATE sync_connections SET setup_done=true WHERE id=$1")
        .bind::<SqlUuid, _>(c.id)
        .execute(conn)?;
    Ok(())
}

/// Complete reads precede deletion detection. An error on any page aborts
/// the pass; a partial list must never be interpreted as remote deletion.
pub fn snapshot(
    app: &App,
    conn: &mut PgConnection,
    c: &Connection,
    client: &Client,
    dir: &std::path::Path,
) -> Result<Vec<RemoteItem>, Error> {
    let git_client = client_for(app, conn, c, None, "import_fetch")?;
    let token = (!git_client.token.is_empty()).then_some(git_client.token.as_str());
    let refs = mirror::ls_remote_for(dir, &c.git_url(app), token, c.provider == "github")?;
    snapshot_records(conn, c, client, refs)
}

/// Read provider records for a complete set of branches and tags.
pub fn snapshot_records(
    conn: &mut PgConnection,
    c: &Connection,
    client: &Client,
    refs: Vec<(String, String)>,
) -> Result<Vec<RemoteItem>, Error> {
    let mut out = vec![];
    let path = client.project_path(c);
    let info = client.call("GET", &path, None)?;
    out.push(make(conn,c,"settings".into(),"settings",json!({"description":info.get("description").and_then(Value::as_str).unwrap_or(""),"default_branch":info.get("default_branch").and_then(Value::as_str).unwrap_or("main"),"archived":info.get("archived").and_then(Value::as_bool).unwrap_or(false)}),&info,"")?);
    let mut shas = BTreeSet::new();
    let mut branches = Vec::new();
    for (name, sha) in refs {
        branches.push(name.clone());
        shas.insert(sha.clone());
        out.push(make(
            conn,
            c,
            format!("ref:{name}"),
            "ref",
            json!({"sha":sha}),
            &Value::Null,
            "",
        )?);
    }
    let pulls = client.list(&format!(
        "{path}/{}",
        if c.provider == "github" {
            "pulls?state=all&sort=created&direction=asc"
        } else {
            "merge_requests?scope=all&state=all&order_by=created_at&sort=asc"
        }
    ))?;
    for mut pull in pulls {
        let number = pull[if c.provider == "github" {
            "number"
        } else {
            "iid"
        }]
        .as_u64()
        .ok_or_else(|| Error::invalid("request number missing"))?;
        if c.provider == "gitlab" {
            pull = client.call("GET", &format!("{path}/merge_requests/{number}"), None)?;
        }
        let pr_id = format!("pr:{number}");
        let body = pull[if c.provider == "github" {
            "body"
        } else {
            "description"
        }]
        .as_str()
        .unwrap_or("");
        let data = request_data(c, &pull);
        let head_sha = text(&data, "head_sha").to_string();
        if !head_sha.is_empty() {
            shas.insert(head_sha.clone());
        }
        out.push(make(conn, c, pr_id.clone(), "pr", data, &pull, body)?);
        let commits_path = if c.provider == "github" {
            format!("{path}/pulls/{number}/commits")
        } else {
            format!("{path}/merge_requests/{number}/commits")
        };
        for commit in client.list(&commits_path)? {
            if let Some(sha) = commit
                .get("sha")
                .or_else(|| commit.get("id"))
                .and_then(Value::as_str)
            {
                shas.insert(sha.to_string());
            }
        }
        let comments_path = if c.provider == "github" {
            format!("{path}/issues/{number}/comments")
        } else {
            format!("{path}/merge_requests/{number}/notes")
        };
        for note in client.list(&comments_path)? {
            if note["system"].as_bool() == Some(true) {
                continue;
            }
            let body = text(&note, "body");
            out.push(make(conn,c,format!("comment:{number}:{}",note["id"]),"comment",json!({"parent":pr_id,"body":clean(body),"path":null,"line":null,"commit_sha":null}),&note,body)?);
        }
        if c.provider == "github" {
            for review in client.list(&format!("{path}/pulls/{number}/reviews"))? {
                if text(&review, "state") == "PENDING" {
                    continue;
                }
                let body = text(&review, "body");
                let state = match text(&review, "state") {
                    "APPROVED" => "approved",
                    "CHANGES_REQUESTED" => "changes_requested",
                    _ => "comment",
                };
                out.push(make(conn,c,format!("review:{number}:{}",review["id"]),"review",json!({"parent":pr_id,"body":clean(body),"state":state,"head_sha":text(&review,"commit_id")}),&review,body)?);
            }
            for note in client.list(&format!("{path}/pulls/{number}/comments"))? {
                let body = text(&note, "body");
                out.push(make(conn,c,format!("inline:{number}:{}",note["id"]),"comment",json!({"parent":pr_id,"body":clean(body),"path":note["path"],"line":note.get("line").or_else(||note.get("original_line")),"commit_sha":note["commit_id"]}),&note,body)?);
            }
        } else {
            for discussion in client.list(&format!("{path}/merge_requests/{number}/discussions"))? {
                for note in discussion["notes"].as_array().into_iter().flatten() {
                    if note["type"].as_str() != Some("DiffNote")
                        || note["system"].as_bool() == Some(true)
                    {
                        continue;
                    }
                    let body = text(note, "body");
                    // Notes also lists diff notes; replace that projection.
                    out.retain(|r| r.id != format!("comment:{number}:{}", note["id"]));
                    out.push(make(conn,c,format!("inline:{number}:{}:{}",discussion["id"].as_str().unwrap_or(""),note["id"]),"comment",json!({"parent":pr_id,"body":clean(body),"path":note.pointer("/position/new_path").or_else(||note.pointer("/position/old_path")),"line":note.pointer("/position/new_line").or_else(||note.pointer("/position/old_line")),"commit_sha":note.pointer("/position/head_sha")}),note,body)?);
                }
            }
            match client.call(
                "GET",
                &format!("{path}/merge_requests/{number}/approvals"),
                None,
            ) {
                Ok(approvals) => {
                    for entry in approvals["approved_by"].as_array().into_iter().flatten() {
                        let raw = json!({"author":entry["user"],"created_at":pull["updated_at"]});
                        out.push(make(conn,c,format!("approval:{number}:{}",entry["user"]["id"]),"review",json!({"parent":pr_id,"body":"","state":"approved","head_sha":head_sha}),&raw,"")?);
                    }
                }
                Err(e) if matches!(e.status, Some(404 | 405 | 501)) => {
                    for replica in super::store::replicas(conn, c.id)? {
                        if let Some(id) = replica
                            .remote_id
                            .filter(|id| id.starts_with(&format!("approval:{number}:")))
                        {
                            if let Some(item) = super::store::item(&replica.remote_base)? {
                                out.push(RemoteItem {
                                    id,
                                    key: Some(replica.object_key),
                                    item,
                                    raw: Value::Null,
                                });
                            }
                        }
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    // Include API-created history summaries so a retry can find its marker.
    for issue in client.list(&format!("{path}/issues?state=all"))? {
        if issue.get("pull_request").is_some() {
            continue;
        }
        let body = issue[if c.provider == "github" {
            "body"
        } else {
            "description"
        }]
        .as_str()
        .unwrap_or("");
        if find_marker(body, c.repo_id).is_some() {
            let n = issue[if c.provider == "github" {
                "number"
            } else {
                "iid"
            }]
            .as_u64()
            .ok_or_else(|| Error::invalid("summary number missing"))?;
            let children = if c.provider == "github" {
                format!("{path}/issues/{n}/comments")
            } else {
                format!("{path}/issues/{n}/notes")
            };
            for note in client.list(&children)? {
                if note["system"].as_bool() == Some(true) {
                    continue;
                }
                let body = text(&note, "body");
                out.push(make(conn,c,format!("history-comment:{n}:{}",note["id"]),"comment",json!({"parent":format!("summary:{n}"),"body":clean(body),"path":null,"line":null,"commit_sha":null}),&note,body)?);
            }
            out.push(make(
                conn,
                c,
                format!(
                    "summary:{}",
                    issue[if c.provider == "github" {
                        "number"
                    } else {
                        "iid"
                    }]
                ),
                "summary",
                json!({"body":body}),
                &issue,
                body,
            )?);
        }
    }
    for branch in branches {
        let endpoint = if c.provider == "github" {
            format!("{path}/commits?sha={}", encoded(&branch))
        } else {
            format!("{path}/repository/commits?ref_name={}", encoded(&branch))
        };
        for commit in client.list(&endpoint)? {
            if let Some(sha) = commit
                .get("sha")
                .or_else(|| commit.get("id"))
                .and_then(Value::as_str)
            {
                shas.insert(sha.to_string());
            }
        }
    }
    // Include formerly reachable revisions so they do not appear deleted.
    for replica in super::store::replicas(conn, c.id)? {
        if replica.object_key.starts_with("status:") {
            if let Some(sha) = replica.object_key.split(':').nth(1) {
                shas.insert(sha.to_string());
            }
        }
    }
    for sha in shas {
        let path = if c.provider == "github" {
            format!("{path}/commits/{sha}/statuses")
        } else {
            format!("{path}/repository/commits/{sha}/statuses?all=true")
        };
        let records = client.list(&path)?;
        let mut histories: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
        for status in &records {
            let context = status
                .get("context")
                .or_else(|| status.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("default");
            histories
                .entry(context.into())
                .or_default()
                .push(status.clone());
        }
        for (context, records) in histories {
            let id = format!(
                "history:{}",
                Uuid::new_v5(&c.id, format!("statuses:{sha}:{context}").as_bytes())
            );
            let data = json!({"title":format!("Status history: {context} on {sha}"),"body":format!("Status history from {}/{}",c.host,c.project),"sha":sha,"context":context,"records":records});
            out.push(make(conn, c, id, "history", data.clone(), &data, "")?);
        }
        let mut seen = BTreeSet::new();
        for status in records {
            super::store::archive(
                conn,
                c.id,
                &format!("status-history:{sha}:{}", status["id"]),
                &status,
            )?;
            let context = status
                .get("context")
                .or_else(|| status.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("default");
            if !seen.insert(context.to_owned()) {
                continue;
            }
            let state = match status
                .get("state")
                .or_else(|| status.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase()
                .as_str()
            {
                "success" => "success",
                "failed" | "failure" => "failure",
                "error" | "canceled" => "error",
                _ => "pending",
            };
            let key = format!("status:{sha}:{}", encoded(context));
            out.push(make(conn,c,key,"status",json!({"sha":sha,"context":context,"state":state,"description":status["description"],"target_url":status["target_url"]}),&status,"")?);
        }
    }
    for item in out
        .iter()
        .filter(|r| r.key.as_ref().is_some_and(|k| k.starts_with("tombstone:")))
    {
        super::store::archive(conn, c.id, &item.id, &item.raw)?;
    }
    out.retain(|r| !r.key.as_ref().is_some_and(|k| k.starts_with("tombstone:")));
    Ok(out)
}

pub struct Written {
    pub id: Option<String>,
    pub remote: Option<Item>,
    pub skipped: bool,
}
fn content(c: &Connection, key: &str, item: &Item, user: bool) -> String {
    let mut body = text(&item.data, "body").to_string();
    body.push_str(&format!("\n\n{}", marker(c.repo_id, key)));
    if !user || item.created_at > 0 {
        body.push_str("\n\n<!-- vcsstar-sync-attribution -->\n");
        body.push_str(&format!(
            "Synced from VCSStar. Original author: {}. Original time: {}.",
            item.author, item.created_at
        ));
        if let Some(url) = &item.url {
            body.push_str(&format!(" Source: {url}"));
        }
    }
    body
}
fn number(id: &str) -> Option<&str> {
    id.split(':').nth(1)
}
fn unavailable(e: &ApiError) -> bool {
    matches!(e.status, Some(405 | 422 | 501))
}

/// On an unsupported representation, try a readable record. Authentication
/// failures, missing permissions, throttling, and outages are never success.
fn summary(
    client: &Client,
    c: &Connection,
    key: &str,
    item: &Item,
    parent: Option<&str>,
    current: Option<&RemoteItem>,
) -> Result<Written, Error> {
    let mut summary = item.clone();
    let details = serde_json::to_string_pretty(&item.data).map_err(Error::internal)?;
    summary.data["body"] = json!(format!(
        "VCSStar {} record\n\n```json\n{details}\n```",
        item.kind
    ));
    let body = content(c, key, &summary, false);
    let path = client.project_path(c);
    let (endpoint, payload) = if let Some(parent) = parent {
        let n = number(parent).ok_or_else(|| Error::invalid("remote parent id"))?;
        let endpoint = if client.provider == "github" {
            format!("{path}/issues/{n}/comments")
        } else if parent.starts_with("summary:") {
            format!("{path}/issues/{n}/notes")
        } else {
            format!("{path}/merge_requests/{n}/notes")
        };
        (endpoint, json!({"body":body}))
    } else {
        (
            format!("{path}/issues"),
            if client.provider == "github" {
                json!({"title":format!("[VCSStar history] {}",text(&item.data,"title")),"body":body})
            } else {
                json!({"title":format!("[VCSStar history] {}",text(&item.data,"title")),"description":body})
            },
        )
    };
    let (method, endpoint) = if let Some(current) = current {
        (
            if c.provider == "github" {
                "PATCH"
            } else {
                "PUT"
            },
            comment_endpoint(client, c, &current.id)?,
        )
    } else {
        ("POST", endpoint)
    };
    match client.call(method, &endpoint, Some(&payload)) {
        Ok(response) => {
            let (id, remote) = if let Some(parent) = parent {
                let id = format!(
                    "{}:{}:{}",
                    if parent.starts_with("summary:") {
                        "history-comment"
                    } else {
                        "comment"
                    },
                    number(parent).unwrap_or(""),
                    response["id"]
                );
                let mut remote = item.clone();
                remote.kind = "comment".into();
                remote.data = json!({"parent":item.data["parent"],"body":clean(&body),"path":null,"line":null,"commit_sha":null});
                (id, remote)
            } else {
                let id = format!(
                    "summary:{}",
                    response[if client.provider == "github" {
                        "number"
                    } else {
                        "iid"
                    }]
                );
                let mut remote = item.clone();
                remote.kind = "summary".into();
                remote.data = json!({"body":body});
                (id, remote)
            };
            Ok(Written {
                id: Some(id),
                remote: Some(remote),
                skipped: false,
            })
        }
        Err(e) if unavailable(&e) => Ok(Written {
            id: None,
            remote: None,
            skipped: true,
        }),
        Err(e) => Err(e.into()),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn write(
    app: &App,
    c: &Connection,
    client: &Client,
    dir: &std::path::Path,
    key: &str,
    item: Option<&Item>,
    current: Option<&RemoteItem>,
    parent: Option<&str>,
) -> Result<Written, Error> {
    let path = client.project_path(c);
    let kind = key.split(':').next().unwrap_or(key);
    if kind == "ref" {
        let name = key
            .strip_prefix("ref:")
            .ok_or_else(|| Error::invalid("ref key"))?;
        vcsstar_vcs::check_ref_name(name)?;
        let sha = item.map(|i| text(&i.data, "sha"));
        let lease = current.map(|r| text(&r.item.data, "sha"));
        match mirror::push_ref_for(
            dir,
            &c.git_url(app),
            &client.token,
            name,
            sha,
            lease,
            false,
            c.provider == "github",
        ) {
            mirror::PushResult::Ok => {
                return Ok(Written {
                    id: Some(key.into()),
                    remote: item.cloned(),
                    skipped: false,
                })
            }
            mirror::PushResult::StaleLease => {
                return Err(Error::new(
                    Code::Conflict,
                    "remote ref changed during sync; read it again",
                ))
            }
            mirror::PushResult::Failed(e) => return Err(e.into()),
        }
    }
    let Some(item) = item else {
        let Some(current) = current else {
            return Ok(Written {
                id: None,
                remote: None,
                skipped: false,
            });
        };
        let ids: Vec<_> = current.id.split(':').collect();
        let endpoint = match (c.provider.as_str(), ids.as_slice()) {
            ("github", ["comment" | "history-comment", _, id]) => {
                Some(format!("{path}/issues/comments/{id}"))
            }
            ("github", ["inline", _, id]) => Some(format!("{path}/pulls/comments/{id}")),
            ("gitlab", ["history-comment", pr, id]) => {
                Some(format!("{path}/issues/{pr}/notes/{id}"))
            }
            ("gitlab", ["comment", pr, id]) => {
                Some(format!("{path}/merge_requests/{pr}/notes/{id}"))
            }
            ("gitlab", ["inline", pr, discussion, id]) => Some(format!(
                "{path}/merge_requests/{pr}/discussions/{discussion}/notes/{id}"
            )),
            _ => None,
        };
        if let Some(endpoint) = endpoint {
            match client.call("DELETE", &endpoint, None) {
                Ok(_) => {}
                Err(e) if e.status == Some(404) => {}
                Err(e) => return Err(e.into()),
            }
            return Ok(Written {
                id: Some(current.id.clone()),
                remote: None,
                skipped: false,
            });
        }
        if current.item.kind == "pr" {
            if c.provider == "gitlab" {
                match client.call(
                    "DELETE",
                    &format!(
                        "{path}/merge_requests/{}",
                        number(&current.id).unwrap_or("")
                    ),
                    None,
                ) {
                    Ok(_) => {
                        return Ok(Written {
                            id: Some(current.id.clone()),
                            remote: None,
                            skipped: false,
                        })
                    }
                    Err(e) if e.status == Some(404) => {
                        return Ok(Written {
                            id: Some(current.id.clone()),
                            remote: None,
                            skipped: false,
                        })
                    }
                    Err(e) if unavailable(&e) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let mut deleted = current.item.clone();
            deleted.data["body"] = json!(format!(
                "{}\n\nThis request was deleted at VCSStar. The stored history remains available.",
                text(&deleted.data, "body")
            ));
            let body = content(c, key, &deleted, false);
            let response = client.call(
                if c.provider == "github" {
                    "PATCH"
                } else {
                    "PUT"
                },
                &format!(
                    "{path}/{}/{}",
                    if c.provider == "github" {
                        "pulls"
                    } else {
                        "merge_requests"
                    },
                    number(&current.id).unwrap_or("")
                ),
                Some(&if c.provider == "github" {
                    json!({"state":"closed","body":body})
                } else {
                    json!({"state_event":"close","description":body})
                }),
            )?;
            deleted.data = request_data(c, &response);
            return Ok(Written {
                id: Some(current.id.clone()),
                remote: Some(deleted),
                skipped: false,
            });
        }
        if current.item.kind == "summary" {
            let mut projection = current.item.clone();
            let body = format!(
                "{}\n\nThis record was deleted at VCSStar. The stored history remains available.",
                text(&projection.data, "body")
            );
            let payload = if c.provider == "github" {
                json!({"state":"closed","body":body})
            } else {
                json!({"state_event":"close","description":body})
            };
            client.call(
                if c.provider == "github" {
                    "PATCH"
                } else {
                    "PUT"
                },
                &format!("{path}/issues/{}", number(&current.id).unwrap_or("")),
                Some(&payload),
            )?;
            projection.data["body"] = json!(body);
            return Ok(Written {
                id: Some(current.id.clone()),
                remote: Some(projection),
                skipped: false,
            });
        }
        if current.item.kind == "review" {
            let n = number(&current.id).ok_or_else(|| Error::invalid("review parent id"))?;
            let endpoint = if c.provider == "github" {
                format!("{path}/issues/{n}/comments")
            } else {
                format!("{path}/merge_requests/{n}/notes")
            };
            let tombstone = format!("tombstone:{key}");
            if !client.list(&endpoint)?.iter().any(|note| {
                find_marker(text(note, "body"), c.repo_id).as_deref() == Some(&tombstone)
            }) {
                let mut notice = current.item.clone();
                notice.data["body"]=json!(format!("This review was deleted at VCSStar. Its original state was {}. The stored history remains available.",text(&notice.data,"state")));
                match client.call(
                    "POST",
                    &endpoint,
                    Some(&json!({"body":content(c,&tombstone,&notice,false)})),
                ) {
                    Ok(_) => {}
                    Err(e) if unavailable(&e) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        // Review/request deletion has no faithful universal operation.
        // Retain the full archived record locally and mark this projection done.
        return Ok(Written {
            id: Some(current.id.clone()),
            remote: Some(current.item.clone()),
            skipped: true,
        });
    };
    if kind == "settings" {
        if c.provider == "gitlab"
            && current.is_some_and(|r| r.item.data["archived"] == true)
            && item.data["archived"] == false
        {
            client.call("POST", &format!("{path}/unarchive"), None)?;
        }
        let mut body = json!({"description":item.data["description"],"default_branch":item.data["default_branch"]});
        if c.provider == "github" {
            body["archived"] = item.data["archived"].clone();
        }
        client.call(
            if c.provider == "github" {
                "PATCH"
            } else {
                "PUT"
            },
            &path,
            Some(&body),
        )?;
        if c.provider == "gitlab"
            && current.is_none_or(|r| r.item.data["archived"] != item.data["archived"])
        {
            client.call(
                "POST",
                &format!(
                    "{path}/{}",
                    if item.data["archived"].as_bool() == Some(true) {
                        "archive"
                    } else {
                        "unarchive"
                    }
                ),
                None,
            )?;
        }
        return Ok(Written {
            id: Some("settings".into()),
            remote: Some(item.clone()),
            skipped: false,
        });
    }
    if kind == "status" {
        let sha = text(&item.data, "sha");
        let endpoint = if c.provider == "github" {
            format!("{path}/statuses/{sha}")
        } else {
            format!("{path}/statuses/{}", encoded(sha))
        };
        let mut body = item.data.clone();
        if !client.user {
            let attribution = format!("[VCSStar: {}]", item.author);
            let attribution: String = attribution.chars().take(100).collect();
            let remaining = 140usize.saturating_sub(attribution.chars().count() + 1);
            let description: String = text(&item.data, "description")
                .chars()
                .take(remaining)
                .collect();
            body["description"] = json!(format!("{description} {attribution}").trim());
        }
        if c.provider == "gitlab" {
            body["name"] = body["context"].clone();
            if body["state"] == "failure" || body["state"] == "error" {
                body["state"] = json!("failed");
            }
        }
        client.call("POST", &endpoint, Some(&body))?;
        let mut projection = item.clone();
        projection.data["description"] = body["description"].clone();
        if c.provider == "gitlab" && text(&item.data, "state") == "error" {
            projection.data["state"] = json!("failure");
        }
        return Ok(Written {
            id: Some(key.into()),
            remote: Some(projection),
            skipped: false,
        });
    }
    if current.is_some_and(|r| r.item.kind == "summary") {
        let old = current.expect("present");
        let mut summary = item.clone();
        summary.data["body"] = json!(format!(
            "VCSStar {} record\n\n```json\n{}\n```",
            item.kind,
            serde_json::to_string_pretty(&item.data).map_err(Error::internal)?
        ));
        let body = content(c, key, &summary, false);
        let payload = if c.provider == "github" {
            json!({"body":body})
        } else {
            json!({"description":body})
        };
        match client.call(
            if c.provider == "github" {
                "PATCH"
            } else {
                "PUT"
            },
            &format!("{path}/issues/{}", number(&old.id).unwrap_or("")),
            Some(&payload),
        ) {
            Ok(_) => {}
            Err(e) if unavailable(&e) => {
                return Ok(Written {
                    id: Some(old.id.clone()),
                    remote: Some(old.item.clone()),
                    skipped: true,
                })
            }
            Err(e) => return Err(e.into()),
        }
        let mut remote = item.clone();
        remote.kind = "summary".into();
        remote.data = json!({"body":body});
        return Ok(Written {
            id: Some(old.id.clone()),
            remote: Some(remote),
            skipped: false,
        });
    }
    if kind == "history" {
        return summary(client, c, key, item, None, None);
    }
    if kind == "pr" {
        if current.is_none()
            && (text(&item.data, "state") != "open" || !text(&item.data, "head_remote").is_empty())
        {
            return summary(client, c, key, item, None, None);
        }
        let mut described = item.clone();
        if text(&item.data, "state") == "merged"
            && current.is_some_and(|r| text(&r.item.data, "state") != "merged")
        {
            described.data["body"] = json!(format!(
                "{}\n\nVCSStar records this request as merged at commit {}.",
                text(&item.data, "body"),
                text(&item.data, "merge_sha")
            ));
        }
        let body = content(c, key, &described, client.user);
        let endpoint = if c.provider == "github" {
            format!("{path}/pulls")
        } else {
            format!("{path}/merge_requests")
        };
        let mut payload = if c.provider == "github" {
            json!({"title":item.data["title"],"body":body,"head":text(&item.data,"head_ref").trim_start_matches("refs/heads/"),"base":text(&item.data,"base_ref").trim_start_matches("refs/heads/")})
        } else {
            json!({"title":item.data["title"],"description":body,"source_branch":text(&item.data,"head_ref").trim_start_matches("refs/heads/"),"target_branch":text(&item.data,"base_ref").trim_start_matches("refs/heads/")})
        };
        let (method, endpoint) = if let Some(current) = current {
            let state = if text(&item.data, "state") == "open" {
                "open"
            } else {
                "closed"
            };
            if text(&current.item.data, "state") == "merged" {
                // A merged request remains merged; its history cannot be reopened.
            } else if c.provider == "github" {
                payload["state"] = json!(state);
            } else if text(&current.item.data, "state") != state {
                payload["state_event"] = json!(if state == "open" { "reopen" } else { "close" });
            }
            (
                if c.provider == "github" {
                    "PATCH"
                } else {
                    "PUT"
                },
                format!("{endpoint}/{}", number(&current.id).unwrap_or("")),
            )
        } else {
            ("POST", endpoint)
        };
        match client.call(method, &endpoint, Some(&payload)) {
            Ok(response) => {
                let id = format!(
                    "pr:{}",
                    response[if c.provider == "github" {
                        "number"
                    } else {
                        "iid"
                    }]
                );
                let mut remote = item.clone();
                remote.data = request_data(c, &response);
                return Ok(Written {
                    id: Some(id),
                    remote: Some(remote),
                    skipped: false,
                });
            }
            Err(e) if unavailable(&e) && current.is_none() => {
                return summary(client, c, key, item, None, None)
            }
            Err(e) => return Err(e.into()),
        }
    }
    let parent = parent.ok_or_else(|| {
        Error::new(
            Code::PreconditionFailed,
            "parent request is not synced to this destination yet",
        )
    })?;
    let n = number(parent).ok_or_else(|| Error::invalid("remote parent"))?;
    if kind == "review" && current.is_some_and(|r| r.item.kind == "comment") {
        return summary(client, c, key, item, Some(parent), current);
    }
    if kind == "review" && c.provider == "gitlab" {
        return summary(
            client,
            c,
            key,
            item,
            Some(parent),
            current.filter(|r| r.item.kind == "comment"),
        );
    }
    let body = content(c, key, item, client.user);
    let result = if kind == "review"
        && current.is_none()
        && c.provider == "github"
        && !parent.starts_with("summary:")
    {
        let event = match text(&item.data, "state") {
            "approved" => "APPROVE",
            "changes_requested" => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        client
            .call(
                "POST",
                &format!("{path}/pulls/{n}/reviews"),
                Some(&json!({"event":event,"body":body,"commit_id":item.data["head_sha"]})),
            )
            .map(|r| (format!("review:{n}:{}", r["id"]), item.clone()))
    } else if kind == "review" && current.is_some_and(|r| r.item.kind == "review") {
        let current = current.expect("present");
        client
            .call(
                "PUT",
                &format!(
                    "{path}/pulls/{n}/reviews/{}",
                    current.id.rsplit(':').next().unwrap_or("")
                ),
                Some(&json!({"body":format!("Review state: {}\n\n{}",text(&item.data,"state"),body)})),
            )
            .map(|_| { let mut projection=current.item.clone(); projection.data["body"]=json!(format!("Review state: {}\n\n{}",text(&item.data,"state"),clean(&body))); (current.id.clone(), projection) })
    } else {
        let (method, endpoint) = if let Some(current) = current {
            (
                if c.provider == "github" {
                    "PATCH"
                } else {
                    "PUT"
                },
                comment_endpoint(client, c, &current.id)?,
            )
        } else {
            (
                "POST",
                if c.provider == "github" {
                    format!("{path}/issues/{n}/comments")
                } else {
                    format!(
                        "{path}/{}/{n}/notes",
                        if parent.starts_with("summary:") {
                            "issues"
                        } else {
                            "merge_requests"
                        }
                    )
                },
            )
        };
        // Conversation comments preserve the original diff location in text
        // when the destination cannot place a historical inline comment.
        let inline = current.is_some_and(|r| r.id.starts_with("inline:"));
        let body = if !inline && !item.data["path"].is_null() {
            format!(
                "{}:{}\n\n{body}",
                text(&item.data, "path"),
                item.data["line"]
            )
        } else {
            body
        };
        client.call(method,&endpoint,Some(&json!({"body":body}))).map(|r|{
            let mut remote=item.clone();remote.kind="comment".into();
            if inline { remote.data["body"]=json!(clean(&body)); }
            else { remote.data=json!({"parent":item.data["parent"],"body":clean(&body),"path":null,"line":null,"commit_sha":null}); }
            (current.map(|r|r.id.clone()).unwrap_or_else(||format!("{}:{n}:{}",if parent.starts_with("summary:"){ "history-comment" }else{"comment"},r["id"])),remote)
        })
    };
    match result {
        Ok((id, remote)) => Ok(Written {
            id: Some(id),
            remote: Some(remote),
            skipped: false,
        }),
        Err(e) if unavailable(&e) => summary(
            client,
            c,
            key,
            item,
            Some(parent),
            current.filter(|r| r.item.kind == "comment"),
        ),
        Err(e) => Err(e.into()),
    }
}

fn comment_endpoint(client: &Client, c: &Connection, id: &str) -> Result<String, Error> {
    let path = client.project_path(c);
    let ids: Vec<_> = id.split(':').collect();
    match (c.provider.as_str(), ids.as_slice()) {
        ("github", ["comment" | "history-comment", _, id]) => {
            Ok(format!("{path}/issues/comments/{id}"))
        }
        ("github", ["inline", _, id]) => Ok(format!("{path}/pulls/comments/{id}")),
        ("gitlab", ["comment", pr, id]) => Ok(format!("{path}/merge_requests/{pr}/notes/{id}")),
        ("gitlab", ["history-comment", pr, id]) => Ok(format!("{path}/issues/{pr}/notes/{id}")),
        ("gitlab", ["inline", pr, discussion, id]) => Ok(format!(
            "{path}/merge_requests/{pr}/discussions/{discussion}/notes/{id}"
        )),
        _ => Err(Error::invalid("unsupported remote comment id")),
    }
}
fn request_data(c: &Connection, pull: &Value) -> Value {
    let github = c.provider == "github";
    let body = text(pull, if github { "body" } else { "description" });
    let merged = pull.get("merged_at").is_some_and(|v| !v.is_null())
        || pull["merged"] == true
        || text(pull, "state") == "merged";
    let state = if merged {
        "merged"
    } else if ["closed", "locked"].contains(&text(pull, "state")) {
        "closed"
    } else {
        "open"
    };
    let head = if github {
        pull.pointer("/head/ref")
            .and_then(Value::as_str)
            .unwrap_or("")
    } else {
        text(pull, "source_branch")
    };
    let base = if github {
        pull.pointer("/base/ref")
            .and_then(Value::as_str)
            .unwrap_or("")
    } else {
        text(pull, "target_branch")
    };
    let head_remote = if github {
        pull.pointer("/head/repo/full_name")
            .and_then(Value::as_str)
            .filter(|s| *s != c.project)
            .map(|s| format!("github:{s}"))
            .unwrap_or_default()
    } else if pull["source_project_id"] != pull["target_project_id"] {
        format!("gitlab:{}:{}", c.host, pull["source_project_id"])
    } else {
        String::new()
    };
    json!({"title":text(pull,"title"),"body":clean(body),"head_ref":format!("refs/heads/{head}"),"base_ref":format!("refs/heads/{base}"),
        "head_sha":pull.pointer("/head/sha").or_else(||pull.get("sha")).and_then(Value::as_str).unwrap_or(""),
        "base_sha":pull.pointer("/base/sha").or_else(||pull.pointer("/diff_refs/start_sha")).or_else(||pull.pointer("/diff_refs/base_sha")).and_then(Value::as_str).unwrap_or(""),
        "state":state,"merge_sha":if merged {text(pull,"merge_commit_sha")}else{""},"head_remote":head_remote})
}

/// Restore the complete object from an attributed history summary. A free
/// text edit that removes the record needs a resolver instead of data loss.
pub fn expand_projection(original: &Item, projection: &Item) -> Option<Item> {
    if original.kind == projection.kind {
        return Some(projection.clone());
    }
    let body = text(&projection.data, "body");
    let prefix = format!("VCSStar {} record\n\n```json\n", original.kind);
    let json = body.strip_prefix(&prefix)?.split_once("\n```")?.0;
    let data: Value = serde_json::from_str(json).ok()?;
    if !data.is_object()
        || original
            .data
            .as_object()?
            .keys()
            .any(|key| data.get(key).is_none())
    {
        return None;
    }
    let mut full = original.clone();
    full.data = data;
    Some(full)
}

/// Apply changes to a destination copy without losing full local fields
/// which that copy could not represent (for example an inline location).
pub fn import_projection(
    original: &Item,
    projection: &Item,
    baseline: Option<&Item>,
) -> Option<Item> {
    if original.kind != projection.kind {
        return expand_projection(original, projection);
    }
    let Some(baseline) = baseline else {
        return Some(projection.clone());
    };
    if baseline.kind != original.kind {
        return expand_projection(original, projection);
    }
    let mut full = original.clone();
    for (key, value) in projection.data.as_object()? {
        if baseline.data.get(key) != Some(value) {
            full.data[key] = value.clone();
        }
    }
    if original.kind == "comment"
        && !original.data["path"].is_null()
        && baseline.data["path"].is_null()
    {
        let prefix = format!(
            "{}:{}\n\n",
            text(&original.data, "path"),
            original.data["line"]
        );
        if let Some(body) = text(&full.data, "body").strip_prefix(&prefix) {
            full.data["body"] = json!(body);
        }
    }
    Some(full)
}
