//! The push transaction through the hook handler, against the real
//! database and a real git repository. Runs only when DATABASE_URL points
//! at a migrated dev database and `git` is installed.

use std::process::Command;

use diesel::prelude::*;
use vcsstar_core::authz::RepoRole;
use vcsstar_core::principal::{CredentialKind, Principal, ReqCtx};
use vcsstar_core::push::{self, PushCtx};
use vcsstar_core::services::{orgs, repos};
use vcsstar_core::{App, Config};
use vcsstar_db::models::{Identity, NewIdentity};
use vcsstar_db::schema::{events, identities};
use vcsstar_store::hook::{HookRefUpdate, HookRequest, ZERO_ID};
use vcsstar_store::refs;
use vcsstar_transport::Peer;
use vcsstar_vcs::{Env, RevId};

fn app(cache: &std::path::Path) -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    if Command::new("git").arg("--version").output().is_err() {
        return None;
    }
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__PUBLIC_URL", "http://test.local");
    std::env::set_var("VCSSTAR__REPO_CACHE_DIR", cache);
    std::env::set_var(
        "VCSSTAR__HOOK_SOCKET",
        cache.join("h.sock").display().to_string(),
    );
    std::env::set_var(
        "VCSSTAR__INSTANCE_ADMINS",
        "0193a000-0000-7000-8000-00000000a0a0@test.local",
    );
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::new(config, db))
}

fn principal(app: &App) -> Principal {
    let mut conn = app.db.get().unwrap();
    let user_id = uuid::Uuid::parse_str("0193a000-0000-7000-8000-00000000a0a0").unwrap();
    let existing: Option<Identity> = identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq("test.local"))
        .select(Identity::as_select())
        .first(&mut conn)
        .optional()
        .unwrap();
    let identity = match existing {
        Some(i) => i,
        None => {
            let row = NewIdentity {
                id: vcsstar_db::new_id(),
                lk_user_id: user_id,
                lk_domain: "test.local",
                handle: "pusher",
                display_name: None,
                email: None,
                email_verified: false,
            };
            diesel::insert_into(identities::table)
                .values(&row)
                .returning(Identity::as_returning())
                .get_result(&mut conn)
                .unwrap()
        }
    };
    Principal {
        identity,
        scopes: vec!["admin".into()],
        kind: CredentialKind::ApiToken,
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

#[test]
fn hook_transaction_moves_refs_and_writes_events() {
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
    let suffix = &uuid::Uuid::now_v7().simple().to_string()[..10];
    let org_name = format!("pushorg{suffix}");
    orgs::create(
        &app,
        &ctx,
        vcsstar_types::CreateOrgRequest {
            ctx: wire_ctx(),
            name: org_name.clone(),
            display_name: None,
            github_owner: None,
        },
    )
    .expect("org");
    repos::create(
        &app,
        &ctx,
        vcsstar_types::CreateRepoRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "r".into(),
            vcs_type: None,
            visibility: Some("public".into()),
            default_branch: None,
            description: None,
        },
    )
    .expect("repo");
    let mut conn = app.db.get().unwrap();
    let (org, repo, _role) =
        vcsstar_core::authz::load_repo_visible(&app, &mut conn, &ctx, &org_name, "r").unwrap();
    let dir = vcsstar_core::vcs::hydrate(&app, &mut conn, &repo).unwrap();

    // A work repo with two commits; move its objects into the cache dir the
    // way receive-pack would (a pack in the object store).
    let work = tmp.path().join("w");
    std::fs::create_dir(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a"), "1\n").unwrap();
    git(&work, &["add", "a"]);
    git(&work, &["commit", "-q", "-m", "one"]);
    let c1 = RevId(git(&work, &["rev-parse", "HEAD"]));
    std::fs::write(work.join("a"), "2\n").unwrap();
    git(&work, &["commit", "-q", "-am", "two"]);
    let c2 = RevId(git(&work, &["rev-parse", "HEAD"]));
    let backend = app.store.backend();
    let mut pack = Vec::new();
    backend
        .export_pack(&work, &[], std::slice::from_ref(&c2), &mut pack)
        .unwrap();
    backend.import_pack(&dir, &mut pack.as_slice()).unwrap();
    assert!(backend.object_exists(&dir, &c2, &Env::new()));

    let push_id = uuid::Uuid::now_v7();
    let _guard = app.pushes.begin(PushCtx {
        id: push_id,
        org: org.clone(),
        repo: repo.clone(),
        dir: dir.clone(),
        principal: p.clone(),
        role: RepoRole::Admin,
        request_id: "t".into(),
    });
    let req = |old: &str, new: &str, name: &str| HookRequest {
        push_id: push_id.to_string(),
        hook: "pre-receive".into(),
        updates: vec![HookRefUpdate {
            old: old.into(),
            new: new.into(),
            name: name.into(),
        }],
        env: vec![],
    };

    // Create main at c2.
    let resp = push::handle(&app, req(ZERO_ID, &c2.0, "refs/heads/main"));
    assert!(resp.ok, "{:?}", resp.messages);
    assert_eq!(
        refs::get(&mut conn, repo.id, "refs/heads/main").unwrap(),
        Some(c2.clone())
    );
    let kinds: Vec<(String, Option<String>)> = events::table
        .filter(events::repo_id.eq(repo.id))
        .order(events::seq.asc())
        .select((events::kind, events::subject))
        .load(&mut conn)
        .unwrap();
    assert_eq!(
        kinds.last().unwrap(),
        &("push".to_string(), Some("refs/heads/main".to_string()))
    );

    // Stale expectation: git thinks main is absent; Postgres says c2.
    let resp = push::handle(&app, req(ZERO_ID, &c1.0, "refs/heads/main"));
    assert!(!resp.ok);
    assert!(
        resp.messages[0].contains("fetch first"),
        "{:?}",
        resp.messages
    );

    // Protect main, then try a non-fast-forward and a delete.
    repos::set_protection(
        &app,
        &ctx,
        vcsstar_types::SetProtectionRequest {
            ctx: wire_ctx(),
            repo: vcsstar_types::RepoRef {
                instance: None,
                org: org_name.clone(),
                name: "r".into(),
            },
            protection: vcsstar_types::BranchProtection {
                ref_pattern: "main".into(),
                required_approvals: 0,
                dismiss_stale: false,
                required_signature_level: "none".into(),
                required_checks: vec![],
                allowed_merge_kinds: vec!["merge".into()],
                allow_force_push: false,
                allow_delete: false,
            },
        },
    )
    .unwrap();
    let resp = push::handle(&app, req(&c2.0, &c1.0, "refs/heads/main"));
    assert!(!resp.ok);
    assert!(resp.messages[0].contains("force"), "{:?}", resp.messages);
    let resp = push::handle(&app, req(&c2.0, ZERO_ID, "refs/heads/main"));
    assert!(!resp.ok);
    assert!(
        resp.messages[0].contains("default branch"),
        "{:?}",
        resp.messages
    );

    // A tag and a second branch in one push, both fine.
    let resp = push::handle(
        &app,
        HookRequest {
            push_id: push_id.to_string(),
            hook: "pre-receive".into(),
            updates: vec![
                HookRefUpdate {
                    old: ZERO_ID.into(),
                    new: c1.0.clone(),
                    name: "refs/heads/dev".into(),
                },
                HookRefUpdate {
                    old: ZERO_ID.into(),
                    new: c2.0.clone(),
                    name: "refs/tags/v1".into(),
                },
            ],
            env: vec![],
        },
    );
    assert!(resp.ok, "{:?}", resp.messages);
    assert_eq!(refs::load(&mut conn, repo.id).unwrap().len(), 3);
    let kinds: Vec<String> = events::table
        .filter(events::repo_id.eq(repo.id))
        .order(events::seq.asc())
        .select(events::kind)
        .load(&mut conn)
        .unwrap();
    assert!(kinds.contains(&"tag_create".to_string()));

    // Internal refs are never writable from a push.
    let resp = push::handle(&app, req(ZERO_ID, &c1.0, "refs/vcsstar/pr/1/head"));
    assert!(!resp.ok);

    // Unknown push id.
    let resp = push::handle(
        &app,
        HookRequest {
            push_id: uuid::Uuid::now_v7().to_string(),
            hook: "pre-receive".into(),
            updates: vec![],
            env: vec![],
        },
    );
    assert!(!resp.ok);
}
