//! Service accounts: an organization admin creates one, mints a token
//! for it, the token resolves to the account's identity, disabling the
//! account refuses the token, and revocation is final. Runs only when
//! DATABASE_URL points at a migrated dev database.

use diesel::prelude::*;
use vcsstar_core::principal::{self, CredentialKind, Principal, ReqCtx};
use vcsstar_core::services::{auth, orgs, service_accounts};
use vcsstar_core::{App, Code, Config};
use vcsstar_db::models::{Identity, NewIdentity};
use vcsstar_db::schema::identities;
use vcsstar_transport::Peer;

fn app() -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__PUBLIC_URL", "https://vcsstar.test.local:8443");
    std::env::set_var(
        "VCSSTAR__INSTANCE_ADMINS",
        "0193a000-0000-7000-8000-00000000d0d1@test.local",
    );
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::new(config, db))
}

fn admin(app: &App) -> Principal {
    let mut conn = app.db.get().unwrap();
    let user_id = uuid::Uuid::parse_str("0193a000-0000-7000-8000-00000000d0d1").unwrap();
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
                handle: "saadmin",
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

fn ctx_for(p: &Principal) -> ReqCtx {
    ReqCtx {
        request_id: "t".into(),
        principal: Some(p.clone()),
        peer: Peer::default(),
    }
}

fn resolve(app: &App, secret: &str) -> Result<Option<Principal>, vcsstar_core::Error> {
    let peer = Peer {
        credential: Some(secret.to_string()),
        ..Peer::default()
    };
    principal::resolve(app, &peer)
}

#[test]
fn service_account_tokens_act_as_the_account_until_disabled_or_revoked() {
    let Some(app) = app() else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let p = admin(&app);
    let ctx = ctx_for(&p);
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("svcorg{}", &suffix[suffix.len() - 8..]);
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

    let account = service_accounts::create(
        &app,
        &ctx,
        vcsstar_types::CreateServiceAccountRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
            description: Some("deploys things".into()),
        },
    )
    .expect("create");
    assert_eq!(account.identity.domain, "svc.vcsstar.test.local-8443");
    assert_eq!(account.identity.handle.as_deref(), Some("deploy-bot"));
    assert_eq!(account.active_tokens, 0);
    assert!(account.disabled_at.is_none());
    let dup = service_accounts::create(
        &app,
        &ctx,
        vcsstar_types::CreateServiceAccountRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
            description: None,
        },
    )
    .expect_err("duplicate name");
    assert_eq!(dup.code, Code::AlreadyExists);

    // A token with a subset of the caller's scopes.
    let token = service_accounts::create_token(
        &app,
        &ctx,
        vcsstar_types::CreateServiceAccountTokenRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
            token_name: "ci".into(),
            scopes: vec!["repo:read".into(), "pr:write".into()],
            client_label: Some("test".into()),
            expires_at: None,
        },
    )
    .expect("token");
    let secret = token.secret.clone().expect("secret shown once");
    let resolved = resolve(&app, &secret).expect("resolve").expect("principal");
    assert_eq!(resolved.identity.id.to_string(), {
        let mut conn = app.conn().unwrap();
        let id: uuid::Uuid = vcsstar_db::schema::service_accounts::table
            .filter(vcsstar_db::schema::service_accounts::name.eq("deploy-bot"))
            .select(vcsstar_db::schema::service_accounts::identity_id)
            .order(vcsstar_db::schema::service_accounts::created_at.desc())
            .first(&mut conn)
            .unwrap();
        id.to_string()
    });
    assert_eq!(resolved.kind, CredentialKind::ApiToken);
    assert!(resolved.has_scope("repo:read"));
    assert!(resolved.has_scope("pr:read"), "pr:write implies pr:read");
    assert!(!resolved.has_scope("mirror:manage"));
    let me = auth::whoami(
        &app,
        &ctx_for(&resolved),
        vcsstar_types::Empty { ctx: wire_ctx() },
    )
    .expect("whoami");
    assert_eq!(me.credential_kind, "api_token");
    assert_eq!(me.acting_via.as_deref(), Some("test"));

    // The account's token cannot mint tokens for the account: it lacks
    // org:admin, and an API token also needs tokens:manage.
    let escalate = service_accounts::create_token(
        &app,
        &ctx_for(&resolved),
        vcsstar_types::CreateServiceAccountTokenRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
            token_name: "more".into(),
            scopes: vec!["admin".into()],
            client_label: None,
            expires_at: None,
        },
    )
    .expect_err("no escalation");
    assert_eq!(escalate.code, Code::Forbidden);

    let listed = service_accounts::list(
        &app,
        &ctx,
        vcsstar_types::GetOrgRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
        },
    )
    .expect("list");
    assert_eq!(listed.accounts.len(), 1);
    assert_eq!(listed.accounts[0].active_tokens, 1);

    // Disabled: the token is refused; enabled again: it works.
    service_accounts::disable(
        &app,
        &ctx,
        vcsstar_types::ServiceAccountRef {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
        },
    )
    .expect("disable");
    let refused = resolve(&app, &secret).expect_err("disabled identity");
    assert_eq!(refused.code, Code::Forbidden);
    service_accounts::enable(
        &app,
        &ctx,
        vcsstar_types::ServiceAccountRef {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
        },
    )
    .expect("enable");
    assert!(resolve(&app, &secret).expect("resolve again").is_some());

    // Revoked: gone for good, and the active count follows.
    service_accounts::revoke_token(
        &app,
        &ctx,
        vcsstar_types::RevokeServiceAccountTokenRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
            token_id: token.id.clone(),
        },
    )
    .expect("revoke");
    let gone = resolve(&app, &secret).expect_err("revoked");
    assert_eq!(gone.code, Code::Unauthenticated);
    let tokens = service_accounts::list_tokens(
        &app,
        &ctx,
        vcsstar_types::ServiceAccountRef {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "deploy-bot".into(),
        },
    )
    .expect("list tokens");
    assert!(tokens.tokens.is_empty());

    // A plain member of the organization may not manage service accounts.
    let mut member = p.clone();
    member.scopes = vec!["org:read".into(), "repo:read".into()];
    let denied = service_accounts::list(
        &app,
        &ctx_for(&member),
        vcsstar_types::GetOrgRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
        },
    )
    .expect_err("scope");
    assert_eq!(denied.code, Code::Forbidden);
}
