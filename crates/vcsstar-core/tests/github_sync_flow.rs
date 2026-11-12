//! GitHub -> vcsstar: webhook payloads become pull requests, reviews,
//! comments, and statuses, with shadow actors the dispatcher never echoes.
//! A local bare repository stands in for GitHub's git side
//! (`VCSSTAR__GITHUB_GIT_BASE=file://...`). Runs only when DATABASE_URL
//! points at a migrated dev database and `git` is installed.

use std::process::Command;

use diesel::prelude::*;
use serde_json::{json, Value as Json};
use vcsstar_core::github_sync;
use vcsstar_core::principal::{CredentialKind, Principal, ReqCtx};
use vcsstar_core::services::{mirror, orgs, prs};
use vcsstar_core::{App, Config};
use vcsstar_db::models::{Identity, NewIdentity};
use vcsstar_db::schema::{events, identities, pr_comments, pr_reviews, pull_requests, statuses};
use vcsstar_store::refs;
use vcsstar_transport::Peer;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A")
        .env("GIT_AUTHOR_EMAIL", "a@x")
        .env("GIT_COMMITTER_NAME", "A")
        .env("GIT_COMMITTER_EMAIL", "a@x")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn app(root: &std::path::Path) -> Option<App> {
    app_opts(root, None)
}

/// `app` with the REST base pointed at a fake server.
fn app_with_api(root: &std::path::Path, base: &str) -> Option<App> {
    app_opts(root, Some(base))
}

fn app_opts(root: &std::path::Path, api_base: Option<&str>) -> Option<App> {
    // The environment is process-wide and `Config::from_env` reads it:
    // two tests building their apps at once would read each other's roots.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _env = ENV.lock().unwrap_or_else(|p| p.into_inner());
    match api_base {
        Some(b) => std::env::set_var("VCSSTAR__GITHUB_API_BASE", b),
        None => std::env::remove_var("VCSSTAR__GITHUB_API_BASE"),
    }
    let url = std::env::var("DATABASE_URL").ok()?;
    if Command::new("git").arg("--version").output().is_err() {
        return None;
    }
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__PUBLIC_URL", "http://test.local");
    std::env::set_var("VCSSTAR__REPO_CACHE_DIR", root.join("cache"));
    std::env::set_var(
        "VCSSTAR__HOOK_SOCKET",
        root.join("h.sock").display().to_string(),
    );
    std::env::set_var(
        "VCSSTAR__GITHUB_GIT_BASE",
        format!("file://{}", root.join("github").display()),
    );
    std::env::set_var(
        "VCSSTAR__INSTANCE_ADMINS",
        "0193a000-0000-7000-8000-00000000d0d0@test.local",
    );
    // A sealing key, so linked-account tokens can be kept (a throwaway).
    std::env::set_var(
        "SECRET_VCSSTAR__CREDENTIAL_KEY",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
    );
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::new(config, db))
}

fn principal(app: &App) -> Principal {
    let mut conn = app.db.get().unwrap();
    let user_id = uuid::Uuid::parse_str("0193a000-0000-7000-8000-00000000d0d0").unwrap();
    let existing: Option<Identity> = identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq("test.local"))
        .select(Identity::as_select())
        .first(&mut conn)
        .optional()
        .unwrap();
    let identity = existing.unwrap_or_else(|| {
        diesel::insert_into(identities::table)
            .values(NewIdentity {
                id: vcsstar_db::new_id(),
                lk_user_id: user_id,
                lk_domain: "test.local",
                handle: "syncadmin",
                display_name: None,
                email: None,
                email_verified: false,
            })
            .returning(Identity::as_returning())
            .get_result(&mut conn)
            .unwrap()
    });
    Principal {
        identity,
        scopes: vec!["*".into()],
        kind: CredentialKind::Session,
        acting_via: None,
        credential_id: None,
        expires_at: None,
        instance_role: None,
    }
}

fn wire_ctx() -> vcsstar_types::Ctx {
    vcsstar_types::Ctx {
        request_id: "t".into(),
        deadline_ms: None,
        contract_version: None,
        contract_digest: None,
    }
}

/// A GitHub-like bare repository `<root>/github/<owner>/<name>.git` with
/// `main` (one commit) and `feat` (one more), plus `refs/pull/1/head`.
fn fake_github(
    root: &std::path::Path,
    owner: &str,
    name: &str,
) -> (std::path::PathBuf, String, String) {
    let bare = root.join("github").join(owner).join(format!("{name}.git"));
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-q", "-b", "main"]);
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("README"), "one\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "one"]);
    let main = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["checkout", "-q", "-b", "feat"]);
    std::fs::write(work.join("feat.txt"), "feature\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "feature"]);
    let feat = git(&work, &["rev-parse", "HEAD"]);
    let url = bare.display().to_string();
    git(
        &work,
        &[
            "push",
            "-q",
            &url,
            "main:refs/heads/main",
            "feat:refs/heads/feat",
        ],
    );
    git(&work, &["push", "-q", &url, "feat:refs/pull/1/head"]);
    (work, main, feat)
}

fn pull_json(number: i64, full: &str, head: &str, base: &str, state: &str, merged: bool) -> Json {
    json!({
        "number": number,
        "title": "Add feature",
        "body": "Please merge",
        "state": state,
        "merged": merged,
        "merge_commit_sha": if merged { Json::String(base.to_string()) } else { Json::Null },
        "merged_by": if merged { json!({"id": 501, "login": "merger"}) } else { Json::Null },
        "user": {"id": 500, "login": "octo"},
        "head": {"sha": head, "ref": "feat", "repo": {"full_name": full}},
        "base": {"sha": base, "ref": "main"},
    })
}

/// `import-repo` queues the import; run the task here as a worker would.
fn import_now(
    app: &App,
    r: Result<vcsstar_types::Repo, vcsstar_core::Error>,
) -> Result<vcsstar_types::Repo, vcsstar_core::Error> {
    use vcsstar_core::mirror_import::{self, Outcome};
    let mut repo = r?;
    let mut conn = app.conn().unwrap();
    let id: uuid::Uuid = vcsstar_db::schema::repos::table
        .inner_join(vcsstar_db::schema::orgs::table)
        .filter(vcsstar_db::schema::orgs::name.eq(&repo.org))
        .filter(vcsstar_db::schema::repos::name.eq(&repo.name))
        .select(vcsstar_db::schema::repos::id)
        .first(&mut conn)
        .unwrap();
    drop(conn);
    match mirror_import::run(app, id)? {
        Outcome::Done { .. } => {}
        other => panic!("import did not finish: {other:?}"),
    }
    repo.mirror_enabled = true;
    Ok(repo)
}

fn event_kinds(conn: &mut PgConnection, lane: &str, after: i64) -> Vec<String> {
    events::table
        .filter(events::lane.eq(lane))
        .filter(events::seq.gt(after))
        .order(events::seq.asc())
        .select(events::kind)
        .load(conn)
        .unwrap()
}

fn last_seq(conn: &mut PgConnection, lane: &str) -> i64 {
    events::table
        .filter(events::lane.eq(lane))
        .select(diesel::dsl::max(events::seq))
        .first::<Option<i64>>(conn)
        .unwrap()
        .unwrap_or(0)
}

#[test]
fn github_pull_request_life_cycle_is_imported_without_echo() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(app) = app(tmp.path()) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("ghsync{}", &suffix[suffix.len() - 8..]);
    let full = format!("{org_name}-gh/thing");
    let (work, main, feat) = fake_github(tmp.path(), &format!("{org_name}-gh"), "thing");
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: Some(format!("{org_name}-gh")),
        },
    )
    .expect("org");
    import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                name: "thing".into(),
                github_repo: full.clone(),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("import");
    let mut conn = app.conn().unwrap();
    // A failed earlier run may have left GitHub account 503 linked to an
    // identity of that run; the ingests below must see its shadow first.
    diesel::delete(
        vcsstar_db::schema::github_links::table
            .filter(vcsstar_db::schema::github_links::github_id.eq(503)),
    )
    .execute(&mut conn)
    .unwrap();
    let (org, repo, _) =
        vcsstar_core::authz::load_repo_visible(&app, &mut conn, &ctx, &org_name, "thing").unwrap();
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    let lane = format!("{org_name}/thing");
    let before = last_seq(&mut conn, &lane);

    // opened: a vcsstar PR appears with a shadow author and a head snapshot.
    let opened = json!({
        "action": "opened",
        "sender": {"id": 500, "login": "octo"},
        "pull_request": pull_json(1, &full, &feat, &main, "open", false),
    });
    let note = github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request",
        &opened,
    )
    .expect("opened");
    assert!(note.contains("pr_opened"), "{note}");
    let pr: prs::PrRow = pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .filter(pull_requests::github_number.eq(1))
        .select(prs::PrRow::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(pr.state, "open");
    assert_eq!(pr.head_sha, feat);
    assert_eq!(pr.head_ref, "refs/heads/feat");
    assert_eq!(pr.body.as_deref(), Some("Please merge"));
    assert_eq!(
        refs::get(&mut conn, repo.id, &prs::head_ref_name(pr.number))
            .unwrap()
            .map(|r| r.0),
        Some(feat.clone())
    );
    assert_eq!(pr.mergeable, "clean", "trial merge ran");
    let author: Identity = identities::table
        .find(pr.author_identity_id)
        .select(Identity::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(author.lk_domain, github_sync::SHADOW_DOMAIN);
    assert_eq!(author.handle, "octo");
    assert!(github_sync::is_shadow_actor(&mut conn, Some(author.id)).unwrap());
    assert!(!github_sync::is_shadow_actor(&mut conn, Some(p.identity.id)).unwrap());
    let kinds = event_kinds(&mut conn, &lane, before);
    assert!(kinds.contains(&"pr_opened".to_string()), "{kinds:?}");
    // The event's actor is the shadow, so the dispatcher skips the mirror.
    let actor: Option<uuid::Uuid> = events::table
        .filter(events::lane.eq(&lane))
        .filter(events::kind.eq("pr_opened"))
        .order(events::seq.desc())
        .select(events::actor)
        .first(&mut conn)
        .unwrap();
    assert!(github_sync::is_shadow_actor(&mut conn, actor).unwrap());

    // The same delivery again changes nothing and emits nothing.
    let before2 = last_seq(&mut conn, &lane);
    let note = github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request",
        &opened,
    )
    .expect("replay");
    assert!(note.contains("unchanged"), "{note}");
    assert!(event_kinds(&mut conn, &lane, before2).is_empty());

    // review + conversation comment + inline comment + status + check_run
    let review = json!({
        "action": "submitted",
        "sender": {"id": 502, "login": "rev"},
        "pull_request": pull_json(1, &full, &feat, &main, "open", false),
        "review": {"id": 9001, "user": {"id": 502, "login": "rev"}, "state": "approved",
                   "body": "LGTM", "commit_id": feat},
    });
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request_review",
        &review,
    )
    .expect("review");
    let reviews: i64 = pr_reviews::table
        .filter(pr_reviews::pr_id.eq(pr.id))
        .filter(pr_reviews::github_id.eq(9001))
        .filter(pr_reviews::state.eq("approved"))
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(reviews, 1);
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request_review",
        &review,
    )
    .expect("review replay");
    let reviews: i64 = pr_reviews::table
        .filter(pr_reviews::pr_id.eq(pr.id))
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(reviews, 1, "a replayed review is not duplicated");

    let comment = json!({
        "action": "created",
        "sender": {"id": 503, "login": "talker"},
        "issue": {"number": 1, "pull_request": {"url": "x"}},
        "comment": {"id": 7001, "user": {"id": 503, "login": "talker"}, "body": "nice\r\n"},
    });
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "issue_comment",
        &comment,
    )
    .expect("comment");
    let edited = json!({
        "action": "edited",
        "sender": {"id": 503, "login": "talker"},
        "issue": {"number": 1, "pull_request": {"url": "x"}},
        "comment": {"id": 7001, "user": {"id": 503, "login": "talker"}, "body": "nicer"},
    });
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "issue_comment",
        &edited,
    )
    .expect("edit");
    let inline = json!({
        "action": "created",
        "sender": {"id": 503, "login": "talker"},
        "pull_request": pull_json(1, &full, &feat, &main, "open", false),
        "comment": {"id": 7002, "user": {"id": 503, "login": "talker"}, "body": "here",
                    "path": "feat.txt", "line": 1, "commit_id": feat},
    });
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request_review_comment",
        &inline,
    )
    .expect("inline");
    type CommentView = (Option<i64>, String, Option<String>, Option<i32>);
    let rows: Vec<CommentView> = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .order(pr_comments::created_at.asc())
        .select((
            pr_comments::github_id,
            pr_comments::body,
            pr_comments::path,
            pr_comments::line,
        ))
        .load(&mut conn)
        .unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].0, Some(7001));
    assert_eq!(rows[0].1, "nicer");
    assert_eq!(rows[1].2.as_deref(), Some("feat.txt"));
    assert_eq!(rows[1].3, Some(1));

    // A comment that carries vcsstar's own marker links instead of duplicating.
    let own_id = vcsstar_db::new_id();
    diesel::insert_into(pr_comments::table)
        .values((
            pr_comments::id.eq(own_id),
            pr_comments::pr_id.eq(pr.id),
            pr_comments::author.eq(p.identity.id),
            pr_comments::body.eq("from vcsstar"),
        ))
        .execute(&mut conn)
        .unwrap();
    let echo = json!({
        "action": "created",
        "sender": {"id": 1, "login": "mirror-bot"},
        "issue": {"number": 1, "pull_request": {"url": "x"}},
        "comment": {"id": 7003, "user": {"id": 1, "login": "mirror-bot"},
                    "body": format!("**x** commented:\n\nfrom vcsstar\n<!-- vcsstar:comment:{own_id} -->")},
    });
    let note = github_sync::ingest(&app, &mut conn, &org, &repo, &state, "issue_comment", &echo)
        .expect("echo");
    assert!(note.contains("linked"), "{note}");
    let linked: Option<i64> = pr_comments::table
        .find(own_id)
        .select(pr_comments::github_id)
        .first(&mut conn)
        .unwrap();
    assert_eq!(linked, Some(7003));

    let status = json!({
        "sender": {"id": 504, "login": "ci"},
        "sha": feat, "context": "ci/build", "state": "success",
        "target_url": "https://ci/1", "description": "ok",
    });
    github_sync::ingest(&app, &mut conn, &org, &repo, &state, "status", &status).expect("status");
    let before3 = last_seq(&mut conn, &lane);
    let note = github_sync::ingest(&app, &mut conn, &org, &repo, &state, "status", &status)
        .expect("status replay");
    assert!(note.contains("unchanged"), "{note}");
    assert!(event_kinds(&mut conn, &lane, before3).is_empty());
    let check = json!({
        "action": "completed",
        "sender": {"id": 504, "login": "ci"},
        "check_run": {"name": "unit", "status": "completed", "conclusion": "failure",
                      "head_sha": feat, "html_url": "https://ci/2",
                      "output": {"title": "3 failed"}},
    });
    github_sync::ingest(&app, &mut conn, &org, &repo, &state, "check_run", &check).expect("check");
    let st: Vec<(String, String)> = statuses::table
        .filter(statuses::repo_id.eq(repo.id))
        .filter(statuses::sha.eq(&feat))
        .order(statuses::context.asc())
        .select((statuses::context, statuses::state))
        .load(&mut conn)
        .unwrap();
    assert_eq!(
        st,
        vec![
            ("ci/build".to_string(), "success".to_string()),
            ("unit".to_string(), "failure".to_string())
        ]
    );

    // synchronize: a new head commit on GitHub moves the snapshot.
    std::fs::write(work.join("feat.txt"), "feature 2\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "feature 2"]);
    let feat2 = git(&work, &["rev-parse", "HEAD"]);
    let bare = tmp
        .path()
        .join("github")
        .join(format!("{org_name}-gh"))
        .join("thing.git")
        .display()
        .to_string();
    git(
        &work,
        &[
            "push",
            "-q",
            &bare,
            "feat:refs/heads/feat",
            "feat:refs/pull/1/head",
        ],
    );
    let sync = json!({
        "action": "synchronize",
        "sender": {"id": 500, "login": "octo"},
        "pull_request": pull_json(1, &full, &feat2, &main, "open", false),
    });
    let before4 = last_seq(&mut conn, &lane);
    let note = github_sync::ingest(&app, &mut conn, &org, &repo, &state, "pull_request", &sync)
        .expect("synchronize");
    assert!(note.contains("pr_head_moved"), "{note}");
    let pr: prs::PrRow = pull_requests::table
        .find(pr.id)
        .select(prs::PrRow::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(pr.head_sha, feat2);
    // import_drift: the head branch itself followed GitHub.
    assert_eq!(
        refs::get(&mut conn, repo.id, "refs/heads/feat")
            .unwrap()
            .map(|r| r.0),
        Some(feat2.clone())
    );
    let kinds = event_kinds(&mut conn, &lane, before4);
    assert!(kinds.contains(&"push".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"pr_head_moved".to_string()), "{kinds:?}");

    // merged on GitHub: main moves there, the PR is merged here.
    git(&work, &["checkout", "-q", "main"]);
    git(
        &work,
        &["merge", "-q", "--no-ff", "-m", "Merge feat", "feat"],
    );
    let merged_sha = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", &bare, "main:refs/heads/main"]);
    let mut merged = pull_json(1, &full, &feat2, &main, "closed", true);
    merged["merge_commit_sha"] = json!(merged_sha);
    let closed = json!({
        "action": "closed",
        "sender": {"id": 501, "login": "merger"},
        "pull_request": merged,
    });
    let note = github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request",
        &closed,
    )
    .expect("merged");
    assert!(note.contains("pr_merged"), "{note}");
    let pr: prs::PrRow = pull_requests::table
        .find(pr.id)
        .select(prs::PrRow::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(pr.state, "merged");
    assert_eq!(pr.merged_commit_sha.as_deref(), Some(merged_sha.as_str()));
    assert!(
        refs::get(&mut conn, repo.id, &prs::head_ref_name(pr.number))
            .unwrap()
            .is_none()
    );
    let merger: Identity = identities::table
        .find(pr.merged_by.unwrap())
        .select(Identity::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(merger.handle, "merger");

    // A branch deleted on GitHub (the usual cleanup after a merge) is
    // deleted here too when vcsstar did not move it since the mirror.
    git(&work, &["push", "-q", &bare, ":refs/heads/feat"]);
    let before5 = last_seq(&mut conn, &lane);
    let dir = vcsstar_core::vcs::hydrate(&app, &mut conn, &repo).unwrap();
    let repush = vcsstar_core::mirror::note_drift(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        &dir,
        None,
        "refs/heads/feat",
        None,
    )
    .expect("note_drift");
    assert!(!repush, "an unchanged branch is not pushed back");
    assert!(refs::get(&mut conn, repo.id, "refs/heads/feat")
        .unwrap()
        .is_none());
    let kinds = event_kinds(&mut conn, &lane, before5);
    assert!(kinds.contains(&"ref_delete".to_string()), "{kinds:?}");

    // Everything ingested so far was applied outside `with_origin`, so the
    // echo guard relied on shadow actors alone. From here on the test
    // applies deliveries the way the ingest worker does.
    let (seq_origin_before, talker_shadow) = {
        let talker: Identity = identities::table
            .inner_join(
                vcsstar_db::schema::github_identities::table
                    .on(vcsstar_db::schema::github_identities::identity_id.eq(identities::id)),
            )
            .filter(vcsstar_db::schema::github_identities::github_id.eq(503))
            .select(Identity::as_select())
            .first(&mut conn)
            .unwrap();
        (last_seq(&mut conn, &lane), talker)
    };
    // A vcsstar user links the GitHub account `talker` (id 503). The rows
    // its shadow owned move to the user, and later deliveries by that
    // account are attributed to the user, with the event stamped as
    // GitHub's own so the dispatcher does not mirror it back.
    let linked_user: Identity = diesel::insert_into(identities::table)
        .values(NewIdentity {
            id: vcsstar_db::new_id(),
            lk_user_id: uuid::Uuid::now_v7(),
            lk_domain: "test.local",
            handle: "talkerhuman",
            display_name: None,
            email: None,
            email_verified: false,
        })
        .returning(Identity::as_returning())
        .get_result(&mut conn)
        .unwrap();
    let tok = vcsstar_core::github::OauthTokens {
        access_token: "gho_test_token".into(),
        expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(8)),
        refresh_token: Some("ghr_test_refresh".into()),
        refresh_expires_at: Some(chrono::Utc::now() + chrono::Duration::days(180)),
    };
    let link = vcsstar_core::github_link::store(
        &app,
        &mut conn,
        linked_user.id,
        503,
        "talker",
        "oauth",
        &tok,
    )
    .expect("link");
    assert_eq!(link.login, "talker");
    let moved: i64 = pr_comments::table
        .filter(pr_comments::author.eq(linked_user.id))
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert!(moved >= 1, "the shadow's comments move to the linked user");
    let left: i64 = pr_comments::table
        .filter(pr_comments::author.eq(talker_shadow.id))
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(left, 0);
    // The actor's own token wins over the organization credential.
    let chosen = vcsstar_core::mirror::cred::token_for_actor(
        &app,
        &mut conn,
        org.id,
        repo.id,
        "mirror_api",
        Some(linked_user.id),
    )
    .expect("token for a linked actor");
    assert!(chosen.is_user());
    assert_eq!(chosen.value, "gho_test_token");
    assert_eq!(chosen.label(), "user:talker");
    // The same account comments again: the comment is the user's, and
    // the event carries origin `github`.
    let body = json!({
        "action": "created",
        "repository": {"full_name": full},
        "sender": {"id": 503, "login": "talker"},
        "issue": {"number": 1, "pull_request": {"url": "x"}},
        "comment": {"id": 7009, "user": {"id": 503, "login": "talker"}, "body": "as myself"},
    });
    let note = vcsstar_core::outbox::with_origin("github", || {
        github_sync::ingest(&app, &mut conn, &org, &repo, &state, "issue_comment", &body)
    })
    .expect("ingest as linked user");
    assert!(note.contains("imported"), "{note}");
    let author: uuid::Uuid = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .filter(pr_comments::github_id.eq(7009))
        .select(pr_comments::author)
        .first(&mut conn)
        .unwrap();
    assert_eq!(author, linked_user.id, "attributed to the linked identity");
    let stamped: Vec<(String, Option<uuid::Uuid>, Option<String>)> = events::table
        .filter(events::lane.eq(&lane))
        .filter(events::seq.gt(seq_origin_before))
        .filter(events::kind.eq("pr_comment"))
        .select((events::kind, events::actor, events::origin))
        .load(&mut conn)
        .unwrap();
    assert_eq!(stamped.len(), 1, "{stamped:?}");
    assert_eq!(stamped[0].1, Some(linked_user.id));
    assert_eq!(stamped[0].2.as_deref(), Some("github"));
    assert!(
        !github_sync::is_shadow_actor(&mut conn, stamped[0].1).unwrap(),
        "a linked user is not a shadow; the origin stamp is what stops the echo"
    );
    // A broken link falls back to the organization credential.
    vcsstar_core::github_link::mark_broken(&mut conn, linked_user.id, "HTTP 401: Bad credentials");
    let fallback = vcsstar_core::mirror::cred::token_for_actor(
        &app,
        &mut conn,
        org.id,
        repo.id,
        "mirror_api",
        Some(linked_user.id),
    );
    assert!(
        !matches!(fallback, Ok(ref t) if t.is_user()),
        "a broken link must not be used: {fallback:?}"
    );
    assert!(vcsstar_core::github_link::unlink(&app, &mut conn, linked_user.id).unwrap());
    assert!(vcsstar_core::github_link::load(&mut conn, linked_user.id)
        .unwrap()
        .is_none());

    // The dispatcher's expansion would still produce nothing for the
    // earlier events: every one of them has a shadow actor.
    let shadowed: Vec<(String, Option<uuid::Uuid>)> = events::table
        .filter(events::lane.eq(&lane))
        .filter(events::seq.gt(before))
        .filter(events::seq.le(seq_origin_before))
        .filter(events::kind.like("pr_%"))
        .select((events::kind, events::actor))
        .load(&mut conn)
        .unwrap();
    assert!(!shadowed.is_empty());
    for (kind, actor) in shadowed {
        assert!(
            github_sync::is_shadow_actor(&mut conn, actor).unwrap(),
            "{kind} has a non-shadow actor"
        );
    }
}

// ---------------------------------------------------------------------
// Pull-based sync (no webhooks)
// ---------------------------------------------------------------------

type Routes = std::sync::Arc<std::sync::Mutex<Vec<(String, Json)>>>;

/// A tiny GitHub REST stand-in: answers each registered path (query
/// ignored) with its JSON and rate limit headers, 404 otherwise, and
/// records every path asked. One request per connection.
fn fake_api(routes: Routes, asked: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = s.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&buf);
            let target = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let path = target.split('?').next().unwrap_or("/").to_string();
            asked.lock().unwrap().push(target.clone());
            let found = routes
                .lock()
                .unwrap()
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, j)| j.clone());
            let (status, body) = match found {
                Some(j) => ("200 OK", j.to_string()),
                None => ("404 Not Found", json!({"message": "Not Found"}).to_string()),
            };
            let reset = chrono::Utc::now().timestamp() + 3600;
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nX-RateLimit-Limit: 5000\r\nX-RateLimit-Remaining: 4321\r\nX-RateLimit-Reset: {reset}\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        }
    });
    base
}

#[test]
fn pull_based_sync_imports_what_webhooks_missed() {
    let tmp = tempfile::tempdir().unwrap();
    let routes: Routes = Default::default();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = fake_api(routes.clone(), asked.clone());
    let Some(app) = app_with_api(tmp.path(), &base) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("ghpull{}", &suffix[suffix.len() - 8..]);
    let full = format!("{org_name}-gh/thing");
    let (_work, main, feat) = fake_github(tmp.path(), &format!("{org_name}-gh"), "thing");
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: Some(format!("{org_name}-gh")),
        },
    )
    .expect("org");
    // The pass needs an API credential; a stored PAT of the organization.
    mirror::set_credential(
        &app,
        &ctx,
        vcsstar_types::SetCredentialRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            repo: None,
            provider: "github".into(),
            purpose: "mirror_api".into(),
            auth_method: "pat".into(),
            host: "github.com".into(),
            secret_ref: "stored".into(),
            secret: Some("ghp_test_not_real".into()),
            enabled: Some(true),
        },
    )
    .expect("credential");
    import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                name: "thing".into(),
                github_repo: full.clone(),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("import");
    let mut conn = app.conn().unwrap();
    let (org, repo, _) =
        vcsstar_core::authz::load_repo_visible(&app, &mut conn, &ctx, &org_name, "thing").unwrap();
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    assert!(state.last_pull_sync_at.is_none());
    let lane = format!("{org_name}/thing");
    let before = last_seq(&mut conn, &lane);

    // What GitHub holds that no webhook told us about: one open PR with
    // an approval, a conversation comment, and a CI status.
    let t_open = chrono::Utc::now() - chrono::Duration::minutes(30);
    let mut pr1 = pull_json(1, &full, &feat, &main, "open", false);
    pr1["updated_at"] = json!(t_open.to_rfc3339());
    {
        let mut r = routes.lock().unwrap();
        r.push((format!("/repos/{full}/pulls"), json!([pr1.clone()])));
        r.push((
            format!("/repos/{full}/pulls/1/reviews"),
            json!([{"id": 9001, "state": "APPROVED", "body": "LGTM", "user": {"id": 502, "login": "rev"}, "commit_id": feat}]),
        ));
        r.push((
            format!("/repos/{full}/issues/1/comments"),
            json!([{"id": 9101, "body": "written during the outage", "user": {"id": 500, "login": "octo"}}]),
        ));
        r.push((format!("/repos/{full}/pulls/1/comments"), json!([])));
        r.push((
            format!("/repos/{full}/commits/{feat}/status"),
            json!({"statuses": [{"context": "ci/x", "state": "success", "target_url": "https://ci/1", "creator": {"id": 600, "login": "ci"}}]}),
        ));
        r.push((
            format!("/repos/{full}/commits/{feat}/check-runs"),
            json!({"check_runs": []}),
        ));
    }
    let out =
        github_sync::reconcile_pulls(&app, &mut conn, &org, &repo, &state, 150).expect("pass 1");
    assert!(out.complete, "{out:?}");
    assert_eq!(out.checked, 1);
    assert!(out.changed >= 4, "pr, review, comment, status: {out:?}");
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("state=open")),
        "first pass lists open PRs"
    );
    let pr: prs::PrRow = pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .filter(pull_requests::github_number.eq(1))
        .select(prs::PrRow::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(pr.state, "open");
    assert_eq!(pr.head_sha, feat);
    let review_count: i64 = pr_reviews::table
        .filter(pr_reviews::pr_id.eq(pr.id))
        .filter(pr_reviews::github_id.eq(9001))
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(review_count, 1);
    let comment_body: String = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .filter(pr_comments::github_id.eq(9101))
        .select(pr_comments::body)
        .first(&mut conn)
        .unwrap();
    assert_eq!(comment_body, "written during the outage");
    let status_state: String = statuses::table
        .filter(statuses::repo_id.eq(repo.id))
        .filter(statuses::sha.eq(&feat))
        .filter(statuses::context.eq("ci/x"))
        .select(statuses::state)
        .first(&mut conn)
        .unwrap();
    assert_eq!(status_state, "success");
    // Everything the pass wrote carries origin github: nothing goes back.
    let origins: Vec<Option<String>> = events::table
        .filter(events::lane.eq(&lane))
        .filter(events::seq.gt(before))
        .select(events::origin)
        .load(&mut conn)
        .unwrap();
    assert!(!origins.is_empty());
    assert!(
        origins.iter().all(|o| o.as_deref() == Some("github")),
        "{origins:?}"
    );
    let kinds = event_kinds(&mut conn, &lane, before);
    for k in ["pr_opened", "pr_review", "pr_comment", "status_set"] {
        assert!(kinds.contains(&k.to_string()), "{k} in {kinds:?}");
    }
    // The watermark is set and the rate limit headers were noted.
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    let watermark = state.last_pull_sync_at.expect("watermark");
    let (_, rates) = vcsstar_core::github_health::snapshot();
    assert!(
        rates
            .iter()
            .any(|r| r.token == "mirror_api" && r.remaining == 4321 && !r.held),
        "{rates:?}"
    );

    // Nothing newer than the watermark: a pass changes nothing.
    let after_first = last_seq(&mut conn, &lane);
    let out =
        github_sync::reconcile_pulls(&app, &mut conn, &org, &repo, &state, 150).expect("pass 2");
    assert!(out.complete);
    assert_eq!(out.checked, 0, "{out:?}");
    assert_eq!(last_seq(&mut conn, &lane), after_first, "no events");
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("state=all")),
        "later passes list all states"
    );

    // GitHub closed the PR while we were not listening.
    let mut closed = pull_json(1, &full, &feat, &main, "closed", false);
    closed["updated_at"] = json!((watermark + chrono::Duration::seconds(5)).to_rfc3339());
    routes
        .lock()
        .unwrap()
        .retain(|(p, _)| *p != format!("/repos/{full}/pulls"));
    routes
        .lock()
        .unwrap()
        .push((format!("/repos/{full}/pulls"), json!([closed])));
    let out =
        github_sync::reconcile_pulls(&app, &mut conn, &org, &repo, &state, 150).expect("pass 3");
    assert_eq!(out.checked, 1);
    assert!(out.changed >= 1, "{out:?}");
    let pr: prs::PrRow = pull_requests::table
        .find(pr.id)
        .select(prs::PrRow::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(pr.state, "closed");
    let kinds = event_kinds(&mut conn, &lane, after_first);
    assert!(kinds.contains(&"pr_closed".to_string()), "{kinds:?}");
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    assert!(
        state.last_pull_sync_at.unwrap() > watermark,
        "watermark advanced"
    );

    // A one-call budget lists but cannot go on: not complete, watermark kept.
    let mut again = pull_json(1, &full, &feat, &main, "closed", false);
    again["updated_at"] = json!((chrono::Utc::now() + chrono::Duration::seconds(5)).to_rfc3339());
    routes
        .lock()
        .unwrap()
        .retain(|(p, _)| *p != format!("/repos/{full}/pulls"));
    routes
        .lock()
        .unwrap()
        .push((format!("/repos/{full}/pulls"), json!([again])));
    let wm = state.last_pull_sync_at.unwrap();
    let out =
        github_sync::reconcile_pulls(&app, &mut conn, &org, &repo, &state, 1).expect("pass 4");
    assert!(!out.complete, "{out:?}");
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    assert_eq!(state.last_pull_sync_at.unwrap(), wm, "watermark kept");
}

#[test]
fn history_import_brings_every_pull_request_with_github_numbers_and_times() {
    let tmp = tempfile::tempdir().unwrap();
    let routes: Routes = Default::default();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = fake_api(routes.clone(), asked.clone());
    let Some(app) = app_with_api(tmp.path(), &base) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("ghback{}", &suffix[suffix.len() - 8..]);
    let owner = format!("{org_name}-gh");
    let full = format!("{owner}/thing");
    let (work, main, feat) = fake_github(tmp.path(), &owner, "thing");
    // An old pull request whose branch is gone: only refs/pull/7/head
    // still holds its head on GitHub.
    let bare = tmp.path().join("github").join(&owner).join("thing.git");
    let bare_url = bare.display().to_string();
    git(&work, &["checkout", "-q", "-b", "old", &main]);
    std::fs::write(work.join("old.txt"), "old work\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "old work"]);
    let old = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", &bare_url, "old:refs/pull/7/head"]);
    git(&work, &["push", "-q", &bare_url, "feat:refs/pull/9/head"]);
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: Some(owner.clone()),
        },
    )
    .expect("org");
    mirror::set_credential(
        &app,
        &ctx,
        vcsstar_types::SetCredentialRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            repo: None,
            provider: "github".into(),
            purpose: "mirror_api".into(),
            auth_method: "pat".into(),
            host: "github.com".into(),
            secret_ref: "stored".into(),
            secret: Some("ghp_test_not_real".into()),
            enabled: Some(true),
        },
    )
    .expect("credential");
    import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                name: "thing".into(),
                github_repo: full.clone(),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("import");
    let mut conn = app.conn().unwrap();
    let (org, repo, _) =
        vcsstar_core::authz::load_repo_visible(&app, &mut conn, &ctx, &org_name, "thing").unwrap();
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    assert!(
        state.backfill_done_at.is_none(),
        "a new mirror has its history to import"
    );

    let mut pr7 = pull_json(7, &full, &old, &main, "closed", true);
    pr7["head"]["ref"] = json!("old");
    pr7["created_at"] = json!("2022-03-01T10:00:00Z");
    pr7["updated_at"] = json!("2022-03-02T10:00:00Z");
    pr7["closed_at"] = json!("2022-03-02T10:00:00Z");
    pr7["merged_at"] = json!("2022-03-02T10:00:00Z");
    // No merge commit here (the fake base is a root commit): the merge
    // base comes from GitHub's base of the time.
    pr7["merge_commit_sha"] = Json::Null;
    let mut pr9 = pull_json(9, &full, &feat, &main, "open", false);
    pr9["created_at"] = json!("2024-01-01T00:00:00Z");
    pr9["updated_at"] = json!("2024-01-02T00:00:00Z");
    {
        let mut r = routes.lock().unwrap();
        r.push((
            format!("/repos/{full}/pulls"),
            json!([pr7.clone(), pr9.clone()]),
        ));
        r.push((
            format!("/repos/{full}/pulls/7/reviews"),
            json!([{"id": 7001, "state": "APPROVED", "body": "ship it", "user": {"id": 502, "login": "rev"}, "commit_id": old, "submitted_at": "2022-03-01T12:00:00Z"}]),
        ));
        r.push((
            format!("/repos/{full}/issues/7/comments"),
            json!([{"id": 7101, "body": "an old remark", "user": {"id": 500, "login": "octo"}, "created_at": "2022-03-01T11:00:00Z", "updated_at": "2022-03-01T11:00:00Z"}]),
        ));
        r.push((format!("/repos/{full}/pulls/7/comments"), json!([])));
        r.push((format!("/repos/{full}/pulls/9/reviews"), json!([])));
        r.push((format!("/repos/{full}/issues/9/comments"), json!([])));
        r.push((format!("/repos/{full}/pulls/9/comments"), json!([])));
        r.push((
            format!("/repos/{full}/commits/{feat}/status"),
            json!({"statuses": []}),
        ));
        r.push((
            format!("/repos/{full}/commits/{feat}/check-runs"),
            json!({"check_runs": []}),
        ));
    }
    let lane = format!("{org_name}/thing");
    let out =
        github_sync::backfill_pulls(&app, &mut conn, &org, &repo, &state, 300).expect("backfill");
    assert!(out.done, "{out:?}");
    assert_eq!(out.pulls, 2, "{out:?}");
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("sort=created")),
        "the import lists in creation order"
    );
    let rows: Vec<prs::PrRow> = pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .order(pull_requests::number.asc())
        .select(prs::PrRow::as_select())
        .load(&mut conn)
        .unwrap();
    let numbers: Vec<(i32, Option<i32>)> =
        rows.iter().map(|r| (r.number, r.github_number)).collect();
    assert_eq!(
        numbers,
        vec![(7, Some(7)), (9, Some(9))],
        "GitHub's numbers are kept"
    );
    let r7 = &rows[0];
    assert_eq!(r7.state, "merged");
    assert_eq!(
        r7.base_sha, main,
        "GitHub's base of the time, not today's branch"
    );
    assert!(
        r7.merge_base_sha.is_some(),
        "a history pull request gets its merge base, so its Files view is right"
    );
    assert_eq!(r7.created_at.to_rfc3339(), "2022-03-01T10:00:00+00:00");
    assert_eq!(
        r7.closed_at.map(|t| t.to_rfc3339()).as_deref(),
        Some("2022-03-02T10:00:00+00:00")
    );
    let review_at: chrono::DateTime<chrono::Utc> = pr_reviews::table
        .filter(pr_reviews::pr_id.eq(r7.id))
        .select(pr_reviews::created_at)
        .first(&mut conn)
        .unwrap();
    assert_eq!(review_at.to_rfc3339(), "2022-03-01T12:00:00+00:00");
    let comment_at: chrono::DateTime<chrono::Utc> = pr_comments::table
        .filter(pr_comments::pr_id.eq(r7.id))
        .select(pr_comments::created_at)
        .first(&mut conn)
        .unwrap();
    assert_eq!(comment_at.to_rfc3339(), "2022-03-01T11:00:00+00:00");
    // The old head came with the bulk fetch; its deleted branch did not
    // come back (it would be pushed to GitHub again).
    let dir = app.store.repo_dir(repo.id);
    git(&dir, &["cat-file", "-e", &old]);
    assert!(
        refs::get(&mut conn, repo.id, "refs/heads/old")
            .unwrap()
            .is_none(),
        "no branch for a closed pull request"
    );
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    assert!(state.backfill_done_at.is_some());
    assert!(state.backfill_heads_at.is_some());
    assert_eq!(state.backfill_pulls, 2);
    let next: i32 = vcsstar_db::schema::pr_counters::table
        .find(repo.id)
        .select(vcsstar_db::schema::pr_counters::next_number)
        .first(&mut conn)
        .unwrap();
    assert_eq!(
        next, 10,
        "vcsstar's own numbers continue after the imported ones"
    );

    // Again from the start: everything matches, nothing is emitted.
    github_sync::restart_backfill(&mut conn, repo.id).unwrap();
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    let before = last_seq(&mut conn, &lane);
    let out = github_sync::backfill_pulls(&app, &mut conn, &org, &repo, &state, 300)
        .expect("backfill again");
    assert!(out.done);
    assert_eq!(
        out.changed,
        0,
        "{out:?} {:?}",
        event_kinds(&mut conn, &lane, before)
    );
    assert_eq!(
        last_seq(&mut conn, &lane),
        before,
        "no events the second time"
    );

    // A budget too small for one pull request keeps the position.
    github_sync::restart_backfill(&mut conn, repo.id).unwrap();
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    let out =
        github_sync::backfill_pulls(&app, &mut conn, &org, &repo, &state, 3).expect("short pass");
    assert!(!out.done);
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    assert!(state.backfill_done_at.is_none());
    assert_eq!((state.backfill_page, state.backfill_index), (1, 0));
    let out = github_sync::backfill_pulls(&app, &mut conn, &org, &repo, &state, 300).expect("rest");
    assert!(out.done, "{out:?}");
}

#[test]
fn stale_snapshots_do_not_overwrite_newer_state() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(app) = app(tmp.path()) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("ghstale{}", &suffix[suffix.len() - 8..]);
    let full = format!("{org_name}-gh/thing");
    let (_work, main, feat) = fake_github(tmp.path(), &format!("{org_name}-gh"), "thing");
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: Some(format!("{org_name}-gh")),
        },
    )
    .expect("org");
    import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                name: "thing".into(),
                github_repo: full.clone(),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("import");
    let mut conn = app.conn().unwrap();
    let (org, repo, _) =
        vcsstar_core::authz::load_repo_visible(&app, &mut conn, &ctx, &org_name, "thing").unwrap();
    let state = vcsstar_core::mirror::state_for(&mut conn, repo.id)
        .unwrap()
        .unwrap();
    let lane = format!("{org_name}/thing");
    let t1 = chrono::Utc::now() - chrono::Duration::minutes(10);
    let t2 = t1 + chrono::Duration::minutes(1);
    let stamped = |mut j: Json, t: chrono::DateTime<chrono::Utc>| {
        j["updated_at"] = json!(t.to_rfc3339());
        j
    };

    // Newer snapshot first: the PR is closed with title "v2".
    let mut newer = pull_json(1, &full, &feat, &main, "closed", false);
    newer["title"] = json!("v2");
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request",
        &json!({"action": "closed", "sender": {"id": 500, "login": "octo"}, "pull_request": stamped(newer, t2)}),
    )
    .expect("newer");
    let before = last_seq(&mut conn, &lane);
    // Then the older one arrives (open, title "v1"): ignored.
    let mut older = pull_json(1, &full, &feat, &main, "open", false);
    older["title"] = json!("v1");
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "pull_request",
        &json!({"action": "opened", "sender": {"id": 500, "login": "octo"}, "pull_request": stamped(older, t1)}),
    )
    .expect("older");
    let pr: prs::PrRow = pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .filter(pull_requests::github_number.eq(1))
        .select(prs::PrRow::as_select())
        .first(&mut conn)
        .unwrap();
    assert_eq!(pr.state, "closed");
    assert_eq!(pr.title, "v2");
    assert_eq!(
        last_seq(&mut conn, &lane),
        before,
        "the stale snapshot emits nothing"
    );

    // Comments: an edit with an older updated_at never wins.
    let comment = |body: &str, t: chrono::DateTime<chrono::Utc>| {
        json!({
            "action": "edited",
            "sender": {"id": 500, "login": "octo"},
            "issue": {"number": 1, "pull_request": {}},
            "comment": {"id": 7777, "body": body, "user": {"id": 500, "login": "octo"}, "updated_at": t.to_rfc3339()},
        })
    };
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "issue_comment",
        &comment("second", t2),
    )
    .expect("comment");
    let note = github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "issue_comment",
        &comment("first", t1),
    )
    .expect("stale comment");
    assert!(note.contains("older"), "{note}");
    let body: String = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .filter(pr_comments::github_id.eq(7777))
        .select(pr_comments::body)
        .first(&mut conn)
        .unwrap();
    assert_eq!(body, "second");
    // A later edit still applies.
    github_sync::ingest(
        &app,
        &mut conn,
        &org,
        &repo,
        &state,
        "issue_comment",
        &comment("third", t2 + chrono::Duration::seconds(1)),
    )
    .expect("later comment");
    let body: String = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .filter(pr_comments::github_id.eq(7777))
        .select(pr_comments::body)
        .first(&mut conn)
        .unwrap();
    assert_eq!(body, "third");
}

#[test]
fn several_github_apps_share_the_load_by_rate_room() {
    let tmp = tempfile::tempdir().unwrap();
    let routes: Routes = Default::default();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = fake_api(routes.clone(), asked);
    let Some(app) = app_with_api(tmp.path(), &base) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    // Two Apps with ids no other test uses, both installed for the repo.
    let n = uuid::Uuid::now_v7().as_u128() % 1_000_000_000;
    let (a, b) = (
        format!("{}", 1_000_000_000 + n),
        format!("{}", 2_000_000_000 + n),
    );
    let key = include_str!("fixtures/github-app-test-key.pem");
    let list = json!([
        {"app_id": a, "client_id": "Iv1.a", "client_secret": "sa", "private_key": key},
        {"app_id": b, "client_id": "Iv1.b", "client_secret": "sb", "private_key": key},
    ]);
    let mut cfg = (*app.config).clone();
    cfg.github_apps = vcsstar_core::config::parse_github_apps(&list.to_string());
    assert_eq!(cfg.github_apps.len(), 2);
    let app = App::new(cfg, app.db.clone());
    let full = format!("multi{n}/thing");
    {
        let mut r = routes.lock().unwrap();
        r.push((format!("/repos/{full}/installation"), json!({"id": 4242})));
        r.push((
            "/app/installations/4242/access_tokens".into(),
            json!({"token": "ghs_not_real", "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()}),
        ));
    }
    let reset = chrono::Utc::now() + chrono::Duration::hours(1);
    let (la, lb) = (format!("app:{a}/4242"), format!("app:{b}/4242"));
    // App A is below its reserve: B is used.
    vcsstar_core::github_health::note_rate_limit(&la, 10, 5000, reset);
    let t = vcsstar_core::mirror::cred::instance_app_token_for(&app, &full)
        .expect("lookup")
        .expect("installed");
    assert_eq!(t.label(), lb);
    // B has spent most of its window, A has room again: A is used.
    vcsstar_core::github_health::note_rate_limit(&la, 4000, 5000, reset);
    vcsstar_core::github_health::note_rate_limit(&lb, 300, 5000, reset);
    let t = vcsstar_core::mirror::cred::instance_app_token_for(&app, &full)
        .expect("lookup")
        .expect("installed");
    assert_eq!(t.label(), la);
    // The scrape shows both, with the App id as a label.
    vcsstar_core::metrics::refresh_rate_limits();
    let text = vcsstar_core::metrics::render();
    assert!(
        text.contains(&format!("vcsstar_github_rate_limit_remaining{{app_id=\"{a}\",installation=\"4242\",token=\"{la}\"}} 4000")),
        "{text}"
    );
    assert!(
        text.contains(&format!("vcsstar_github_rate_limit_limit{{app_id=\"{b}\"")),
        "{text}"
    );
}

#[test]
fn import_continues_an_unfinished_import_and_sends_nothing_back() {
    use vcsstar_db::schema::{mirror_state, orgs as orgs_t, repos};
    let tmp = tempfile::tempdir().unwrap();
    let Some(app) = app(tmp.path()) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("ghresume{}", &suffix[suffix.len() - 8..]);
    let full = format!("{org_name}-gh/thing");
    let (_work, _main, _feat) = fake_github(tmp.path(), &format!("{org_name}-gh"), "thing");
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: Some(format!("{org_name}-gh")),
        },
    )
    .expect("org");
    let mut conn = app.conn().unwrap();
    // What an import leaves when the pod stops during its fetch: the row,
    // its paused mirror state, no refs, mirror not enabled.
    let org_id: uuid::Uuid = orgs_t::table
        .filter(orgs_t::name.eq(&org_name))
        .select(orgs_t::id)
        .first(&mut conn)
        .unwrap();
    let stub_id = vcsstar_db::new_id();
    diesel::insert_into(repos::table)
        .values(&vcsstar_db::models::NewRepo {
            id: stub_id,
            org_id,
            name: "thing",
            visibility: "public",
            vcs_type: "git",
            default_branch: "main",
            description: None,
        })
        .execute(&mut conn)
        .unwrap();
    diesel::insert_into(mirror_state::table)
        .values((
            mirror_state::repo_id.eq(stub_id),
            mirror_state::github_repo.eq(&full),
            mirror_state::mode.eq("import_drift"),
            mirror_state::paused.eq(true),
        ))
        .execute(&mut conn)
        .unwrap();
    let lane = format!("{org_name}/thing");
    let before = last_seq(&mut conn, &lane);
    let repo = import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                name: "thing".into(),
                github_repo: full.clone(),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("the import continues");
    assert!(repo.mirror_enabled);
    let (id, enabled): (uuid::Uuid, bool) = repos::table
        .filter(repos::org_id.eq(org_id))
        .filter(repos::name.eq("thing"))
        .select((repos::id, repos::mirror_enabled))
        .first(&mut conn)
        .unwrap();
    assert_eq!(id, stub_id, "the stub row is used, not a second one");
    assert!(enabled);
    assert!(!refs::load(&mut conn, stub_id).unwrap().is_empty());
    assert!(
        event_kinds(&mut conn, &lane, before)
            .iter()
            .all(|k| k != "repo_created"),
        "a continued import emits no second repo_created"
    );

    // A fresh import: its repo_created mirrors GitHub, so it carries origin
    // `github` and the dispatcher plans no settings action from it.
    std::fs::remove_dir_all(tmp.path().join("work")).unwrap();
    let (_w2, _m2, _f2) = fake_github(tmp.path(), &format!("{org_name}-gh"), "other");
    import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                name: "other".into(),
                github_repo: format!("{org_name}-gh/other"),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("import");
    let origins: Vec<Option<String>> = events::table
        .filter(events::lane.eq(format!("{org_name}/other")))
        .filter(events::kind.eq("repo_created"))
        .select(events::origin)
        .load(&mut conn)
        .unwrap();
    assert_eq!(origins, vec![Some("github".to_string())]);
}

#[test]
fn queued_import_finishes_once_and_schedule_is_per_repository() {
    use vcsstar_core::mirror_import::{self, Outcome};
    let tmp = tempfile::tempdir().unwrap();
    let Some(app) = app(tmp.path()) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("ghqueue{}", &suffix[suffix.len() - 8..]);
    let full = format!("{org_name}-gh/thing");
    let _gh = fake_github(tmp.path(), &format!("{org_name}-gh"), "thing");
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: Some(format!("{org_name}-gh")),
        },
    )
    .expect("org");
    let req = || vcsstar_types::ImportRequest {
        ctx: wire_ctx(),
        org: org_name.clone(),
        name: "thing".into(),
        github_repo: full.clone(),
        mode: Some("import_drift".into()),
        visibility: Some("public".into()),
    };
    let repo = mirror::import_repo(&app, &ctx, req()).expect("queued");
    assert!(!repo.mirror_enabled, "the answer comes before the fetch");
    let get = || vcsstar_types::GetRepoRequest {
        ctx: wire_ctx(),
        repo: vcsstar_types::RepoRef {
            org: org_name.clone(),
            name: "thing".into(),
            instance: None,
        },
    };
    let s = mirror::status(&app, &ctx, get()).unwrap();
    assert_eq!(s.import_state.as_deref(), Some("queued"));
    // A second request while queued changes nothing and is no error.
    mirror::import_repo(&app, &ctx, req()).expect("still queued");
    let id = {
        let mut conn = app.conn().unwrap();
        vcsstar_db::schema::repos::table
            .inner_join(vcsstar_db::schema::orgs::table)
            .filter(vcsstar_db::schema::orgs::name.eq(&org_name))
            .select(vcsstar_db::schema::repos::id)
            .first::<uuid::Uuid>(&mut conn)
            .unwrap()
    };
    assert!(matches!(mirror_import::run(&app, id).unwrap(), Outcome::Done { refs } if refs >= 2));
    // The task given out again (lease expiry) does nothing more.
    assert_eq!(
        mirror_import::run(&app, id).unwrap(),
        Outcome::Done { refs: 0 }
    );
    let s = mirror::status(&app, &ctx, get()).unwrap();
    assert_eq!(s.import_state.as_deref(), Some("done"));
    assert!(!s.paused);
    assert_eq!(
        (s.pull_sync_interval_s, s.ref_reconcile_interval_s),
        (900, 900)
    );
    // A finished import of the name is refused.
    let again = mirror::import_repo(&app, &ctx, req()).unwrap_err();
    assert_eq!(again.code, vcsstar_core::Code::AlreadyExists);

    let sched = |pull: Option<u64>, refs_s: Option<u64>| {
        mirror::set_schedule(
            &app,
            &ctx,
            vcsstar_types::SetMirrorScheduleRequest {
                ctx: wire_ctx(),
                repo: get().repo,
                pull_sync_interval_s: pull,
                ref_reconcile_interval_s: refs_s,
            },
        )
    };
    assert!(sched(Some(30), None).is_err(), "below the minimum");
    let s = sched(Some(120), Some(3600)).unwrap();
    assert_eq!(
        (s.pull_sync_interval_s, s.ref_reconcile_interval_s),
        (120, 3600)
    );
    let s = sched(Some(0), None).unwrap();
    assert_eq!(
        (s.pull_sync_interval_s, s.ref_reconcile_interval_s),
        (900, 3600)
    );
}

#[test]
fn a_fork_in_another_org_is_listed_and_opens_a_pull_request_back() {
    use vcsstar_core::services::{repos as repo_svc, vcs_ops};
    let tmp = tempfile::tempdir().unwrap();
    let Some(app) = app(tmp.path()) else {
        eprintln!("DATABASE_URL unset or git missing; skipping");
        return;
    };
    let p = principal(&app);
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    };
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let up = format!("ghup{}", &suffix[suffix.len() - 8..]);
    let down = format!("ghdown{}", &suffix[suffix.len() - 8..]);
    let _gh = fake_github(tmp.path(), &format!("{up}-gh"), "thing");
    for (name, owner) in [(&up, Some(format!("{up}-gh"))), (&down, None)] {
        orgs::create(
            &app,
            &ctx,
            vcsstar_types::CreateOrgRequest {
                ctx: wire_ctx(),
                name: name.clone(),
                display_name: None,
                github_owner: owner,
            },
        )
        .expect("org");
    }
    import_now(
        &app,
        mirror::import_repo(
            &app,
            &ctx,
            vcsstar_types::ImportRequest {
                ctx: wire_ctx(),
                org: up.clone(),
                name: "thing".into(),
                github_repo: format!("{up}-gh/thing"),
                mode: Some("import_drift".into()),
                visibility: Some("public".into()),
            },
        ),
    )
    .expect("import");
    let source = vcsstar_types::RepoRef {
        org: up.clone(),
        name: "thing".into(),
        instance: None,
    };
    let fork = vcs_ops::fork(
        &app,
        &ctx,
        vcsstar_types::ForkRequest {
            ctx: wire_ctx(),
            source: source.clone(),
            org: down.clone(),
            name: None,
        },
    )
    .expect("fork");
    let fork_ref = vcsstar_types::RepoRef {
        org: fork.org.clone(),
        name: fork.name.clone(),
        instance: None,
    };
    let got = repo_svc::get(
        &app,
        &ctx,
        vcsstar_types::GetRepoRequest {
            ctx: wire_ctx(),
            repo: fork_ref.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        got.fork_of
            .as_ref()
            .map(|r| (r.org.as_str(), r.name.as_str())),
        Some((up.as_str(), "thing"))
    );
    let forks = repo_svc::list_forks(
        &app,
        &ctx,
        vcsstar_types::GetRepoRequest {
            ctx: wire_ctx(),
            repo: source.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        forks
            .repos
            .iter()
            .map(|r| r.org.as_str())
            .collect::<Vec<_>>(),
        vec![down.as_str()]
    );
    // A pull request from the fork's branch, in the other organization,
    // into the source repository.
    let pr = prs::open(
        &app,
        &ctx,
        vcsstar_types::OpenPrRequest {
            ctx: wire_ctx(),
            repo: source.clone(),
            title: "From the fork".into(),
            body: None,
            base_ref: "main".into(),
            head: vcsstar_types::PrHead {
                repo: fork_ref.clone(),
                r#ref: "feat".into(),
            },
        },
    )
    .expect("cross-org pull request");
    assert_eq!(
        (pr.head.repo.org.as_str(), pr.repo.org.as_str()),
        (down.as_str(), up.as_str())
    );
}
