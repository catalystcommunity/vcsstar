//! Authentication and branch protection against PostgreSQL and real Git objects.
use diesel::prelude::*;
use vcsstar_core::{
    principal::{self, CredentialKind, Principal, ReqCtx},
    protection::{self, Source},
    services::{local_login, login, orgs, prs, repos, statuses, vcs_ops},
    App, Code, Config,
};
use vcsstar_db::{
    models::{Identity, NewIdentity},
    schema::{branch_protections, identities},
};
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_transport::Peer;
use vcsstar_types as t;
use vcsstar_vcs::RevId;

fn wire() -> t::Ctx {
    t::Ctx {
        request_id: "test".into(),
        deadline_ms: None,
        contract_version: None,
        contract_digest: None,
    }
}
fn ctx(p: Option<&Principal>) -> ReqCtx {
    ReqCtx {
        request_id: "test".into(),
        principal: p.cloned(),
        peer: Peer::default(),
    }
}
fn setup() -> Option<(App, tempfile::TempDir, Principal)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.repo_cache_dir = tmp.path().join("cache");
    config.hook_socket = tmp.path().join("hook.sock");
    config.rate_limits.login_per_minute = 1000;
    config.instance_admins = vec![];
    let db = vcsstar_db::pool(&url, 4).unwrap();
    vcsstar_db::run_migrations(&mut db.get().unwrap()).unwrap();
    let app = App::new(config, db);
    let admin = person(&app, true);
    Some((app, tmp, admin))
}
fn person(app: &App, admin: bool) -> Principal {
    let i: Identity = diesel::insert_into(identities::table)
        .values(NewIdentity {
            id: uuid::Uuid::now_v7(),
            lk_user_id: uuid::Uuid::now_v7(),
            lk_domain: "test.invalid",
            handle: "tester",
            display_name: None,
            email: None,
            email_verified: false,
        })
        .returning(Identity::as_returning())
        .get_result(&mut app.conn().unwrap())
        .unwrap();
    Principal {
        identity: i,
        scopes: vec!["*".into()],
        kind: CredentialKind::Session,
        acting_via: None,
        credential_id: None,
        expires_at: None,
        instance_role: admin.then_some(principal::InstanceRole::Admin),
    }
}
#[test]
fn local_passwords_stop_permanently_after_linkkeys_binding() {
    let Some((app, _tmp, admin)) = setup() else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let username = format!("local{}", uuid::Uuid::now_v7().simple());
    let password = "a long local test password";
    let create = || t::CreateLocalAccountRequest {
        ctx: wire(),
        username: username.clone(),
        password: password.into(),
        display_name: None,
    };
    assert_eq!(
        local_login::create(&app, &ctx(None), create())
            .unwrap_err()
            .code,
        Code::Unauthenticated
    );
    let outsider = person(&app, false);
    assert_eq!(
        local_login::create(&app, &ctx(Some(&outsider)), create())
            .unwrap_err()
            .code,
        Code::Forbidden
    );
    let account = local_login::create(&app, &ctx(Some(&admin)), create()).unwrap();
    assert!(account.password_login);
    let request = |password: &str| t::LocalLoginRequest {
        ctx: wire(),
        username: username.clone(),
        password: password.into(),
        return_to: Some("/me".into()),
        device_user_code: None,
    };
    assert_eq!(
        local_login::sign_in(&app, &ctx(None), request("incorrect"))
            .unwrap_err()
            .code,
        Code::Unauthenticated
    );
    let session = local_login::sign_in(&app, &ctx(None), request(password)).unwrap();
    let peer = Peer {
        credential: Some(session.session_token.clone()),
        ..Peer::default()
    };
    let p = principal::resolve(&app, &peer).unwrap().unwrap();
    assert!(!p.identity.linkkeys_bound);
    // Binding is represented by verified login code changing identity coordinates.
    let new_id = uuid::Uuid::now_v7();
    diesel::update(identities::table.find(p.identity.id))
        .set((
            identities::lk_user_id.eq(new_id),
            identities::lk_domain.eq("linked.example"),
        ))
        .execute(&mut app.conn().unwrap())
        .unwrap();
    assert!(
        principal::resolve(&app, &peer).is_err()
            || principal::resolve(&app, &peer).unwrap().is_none()
    );
    assert_eq!(
        local_login::sign_in(&app, &ctx(None), request(password))
            .unwrap_err()
            .code,
        Code::Unauthenticated
    );
    assert_eq!(
        local_login::reset(
            &app,
            &ctx(Some(&admin)),
            t::ResetLocalPasswordRequest {
                ctx: wire(),
                identity: format!("{new_id}@linked.example"),
                password: password.into()
            }
        )
        .unwrap_err()
        .code,
        Code::Forbidden
    );
    assert!(diesel::update(identities::table.find(p.identity.id))
        .set(identities::linkkeys_bound.eq(false))
        .execute(&mut app.conn().unwrap())
        .is_err());
    // Even restoring a hash in settings cannot reopen the login path.
    diesel::sql_query("UPDATE local_accounts SET password_hash='not-a-hash' WHERE identity_id=$1")
        .bind::<diesel::sql_types::Uuid, _>(p.identity.id)
        .execute(&mut app.conn().unwrap())
        .unwrap();
    assert!(local_login::sign_in(&app, &ctx(None), request(password)).is_err());
    assert!(login::check_return_to(Some("/\\evil.example")).is_err());
}
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
#[test]
fn protected_refs_need_the_authority_and_current_approval_and_status() {
    let Some((app, _tmp, admin)) = setup() else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let a = ctx(Some(&admin));
    let name = format!("trust{}", uuid::Uuid::now_v7().simple());
    orgs::create(
        &app,
        &a,
        t::CreateOrgRequest {
            ctx: wire(),
            name: name.clone(),
            display_name: None,
            github_owner: None,
        },
    )
    .unwrap();
    repos::create(
        &app,
        &a,
        t::CreateRepoRequest {
            ctx: wire(),
            org: name.clone(),
            name: "r".into(),
            vcs_type: None,
            visibility: Some("public".into()),
            default_branch: None,
            description: None,
        },
    )
    .unwrap();
    let r = t::RepoRef {
        instance: None,
        org: name,
        name: "r".into(),
    };
    let mut conn = app.conn().unwrap();
    let (org, repo, _) =
        vcsstar_core::authz::load_repo_visible(&app, &mut conn, &a, &r.org, &r.name).unwrap();
    let rules = repos::protections_for(&mut conn, repo.id).unwrap();
    assert_eq!(rules.len(), 6);
    assert!(rules
        .iter()
        .all(|r| r.required_signature_level == "none" && r.required_approvals == 1));
    let dir = vcsstar_core::vcs::hydrate(&app, &mut conn, &repo).unwrap();
    let tree = git(&dir, &["mktree"]);
    let base = git(&dir, &["commit-tree", &tree, "-m", "base"]);
    let head = git(&dir, &["commit-tree", &tree, "-p", &base, "-m", "head"]);
    refs::apply(
        &mut conn,
        repo.id,
        &[
            RefUpdate {
                name: "refs/heads/main".into(),
                old: None,
                new: Some(RevId(base.clone())),
            },
            RefUpdate {
                name: "refs/heads/topic".into(),
                old: None,
                new: Some(RevId(head.clone())),
            },
        ],
        Some(admin.identity.id),
    )
    .unwrap();
    drop(conn);
    let update = || t::UpdateRefRequest {
        ctx: wire(),
        repo: r.clone(),
        name: "refs/heads/main".into(),
        expected: Some(base.clone()),
        target: Some(head.clone()),
    };
    // Admin has no implicit override. The RPC must enforce PR policy too.
    assert_eq!(
        vcs_ops::update_ref(&app, &a, update()).unwrap_err().code,
        Code::Forbidden
    );
    let push_id = uuid::Uuid::now_v7();
    let _push = app.pushes.begin(vcsstar_core::push::PushCtx {
        request_id: "trust-test".into(),
        id: push_id,
        org: org.clone(),
        repo: repo.clone(),
        dir: dir.clone(),
        principal: admin.clone(),
        role: vcsstar_core::authz::RepoRole::Admin,
    });
    let denied = vcsstar_core::push::handle(
        &app,
        vcsstar_store::hook::HookRequest {
            push_id: push_id.to_string(),
            hook: "pre-receive".into(),
            updates: vec![vcsstar_store::hook::HookRefUpdate {
                name: "refs/heads/main".into(),
                old: base.clone(),
                new: head.clone(),
            }],
            env: vec![],
        },
    );
    assert!(!denied.ok, "an admin needs an explicit account override");
    let pr = prs::open(
        &app,
        &a,
        t::OpenPrRequest {
            ctx: wire(),
            repo: r.clone(),
            title: "change".into(),
            body: None,
            base_ref: "main".into(),
            head: t::PrHead {
                repo: r.clone(),
                r#ref: "topic".into(),
            },
        },
    )
    .unwrap();
    let reviewer = person(&app, false);
    let reviewer_key = reviewer.key();
    // An explicit allowed account can review without write permission.
    let mut conn = app.conn().unwrap();
    diesel::update(branch_protections::table.filter(branch_protections::repo_id.eq(repo.id)))
        .set((
            branch_protections::allowed_approvers.eq(vec![Some(reviewer_key)]),
            branch_protections::required_checks.eq(vec![Some("ci".to_string())]),
            branch_protections::required_check_reporters.eq(vec![Some(admin.key())]),
        ))
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let review = |state: &str, base_sha: &str| t::ReviewRequest {
        ctx: wire(),
        repo: r.clone(),
        number: pr.number,
        state: state.into(),
        body: None,
        head_sha: head.clone(),
        base_sha: Some(base_sha.into()),
        approval_scope: Some("linkkeys-trust-anchors".into()),
    };
    let merge = || t::MergeRequest {
        ctx: wire(),
        repo: r.clone(),
        number: pr.number,
        merge_kind: "merge".into(),
        expected_head_sha: head.clone(),
        message: None,
    };
    prs::review(&app, &ctx(Some(&reviewer)), review("approved", &base)).unwrap();
    assert!(prs::merge(&app, &a, merge()).is_err()); // No check.
    let forged_reporter = person(&app, true);
    statuses::set(
        &app,
        &ctx(Some(&forged_reporter)),
        t::SetStatusRequest {
            ctx: wire(),
            repo: r.clone(),
            sha: head.clone(),
            context: "ci".into(),
            state: "success".into(),
            target_url: None,
            description: None,
            base_sha: Some(base.clone()),
        },
    )
    .unwrap();
    assert!(prs::merge(&app, &a, merge()).is_err());
    statuses::set(
        &app,
        &a,
        t::SetStatusRequest {
            ctx: wire(),
            repo: r.clone(),
            sha: head.clone(),
            context: "ci".into(),
            state: "success".into(),
            target_url: None,
            description: None,
            base_sha: Some(base.clone()),
        },
    )
    .unwrap();
    prs::review(
        &app,
        &ctx(Some(&reviewer)),
        review("changes_requested", &base),
    )
    .unwrap();
    assert!(prs::merge(&app, &a, merge()).is_err()); // Approval withdrawn.
    prs::review(&app, &ctx(Some(&reviewer)), review("approved", &base)).unwrap();
    let next = git(
        &dir,
        &["commit-tree", &tree, "-p", &base, "-m", "base moved"],
    );
    let mut conn = app.conn().unwrap();
    let advance = RefUpdate {
        name: "refs/heads/main".into(),
        old: Some(RevId(base.clone())),
        new: Some(RevId(next.clone())),
    };
    refs::apply(
        &mut conn,
        repo.id,
        std::slice::from_ref(&advance),
        Some(admin.identity.id),
    )
    .unwrap();
    prs::on_push(
        &app,
        &mut conn,
        &org,
        &repo,
        Some(&admin),
        std::slice::from_ref(&advance),
    )
    .unwrap();
    drop(conn);
    assert!(prs::merge(&app, &a, merge()).is_err());
    assert!(prs::review(&app, &ctx(Some(&reviewer)), review("approved", &base)).is_err());
    prs::review(&app, &ctx(Some(&reviewer)), review("approved", &next)).unwrap();
    assert!(prs::merge(&app, &a, merge()).is_err()); // Old-base CI still fails.
    statuses::set(
        &app,
        &a,
        t::SetStatusRequest {
            ctx: wire(),
            repo: r.clone(),
            sha: head.clone(),
            context: "ci".into(),
            state: "success".into(),
            target_url: None,
            description: None,
            base_sha: Some(next.clone()),
        },
    )
    .unwrap();
    prs::merge(&app, &a, merge())
        .expect("current approvals and checks accept unsigned Git commits");
    let mut conn = app.conn().unwrap();
    let update = RefUpdate {
        name: "refs/heads/main".into(),
        old: None,
        new: Some(RevId(head)),
    };
    assert!(protection::check_change(
        &mut conn,
        &repo,
        None,
        &update,
        false,
        Source::Connection("other")
    )
    .is_err());
    // Account overrides are explicit and still cannot change source authority.
    diesel::update(branch_protections::table.filter(branch_protections::repo_id.eq(repo.id)))
        .set(branch_protections::override_accounts.eq(vec![Some(admin.key())]))
        .execute(&mut conn)
        .unwrap();
    assert!(protection::check_change(
        &mut conn,
        &repo,
        Some(&admin),
        &update,
        false,
        Source::Direct
    )
    .is_ok());
    assert!(protection::check_change(
        &mut conn,
        &repo,
        Some(&admin),
        &update,
        false,
        Source::Connection("other")
    )
    .is_err());
}
