//! Credentials with one scope and purpose form a set; an operation picks a
//! member at random and the set survives one member's removal. Runs only
//! when DATABASE_URL points at a migrated dev database.

use std::collections::HashSet;

use vcsstar_core::mirror::cred;
use vcsstar_core::principal::{CredentialKind, Principal, ReqCtx};
use vcsstar_core::services::{mirror, orgs};
use vcsstar_core::{App, Config};
use vcsstar_transport::Peer;

fn app() -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__PUBLIC_URL", "http://test.local");
    std::env::set_var(
        "VCSSTAR__INSTANCE_ADMINS",
        "0193a000-0000-7000-8000-00000000c0c0@test.local",
    );
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::new(config, db))
}

fn principal(app: &App) -> Principal {
    use diesel::prelude::*;
    use vcsstar_db::models::{Identity, NewIdentity};
    use vcsstar_db::schema::identities;
    let mut conn = app.db.get().unwrap();
    let user_id = uuid::Uuid::parse_str("0193a000-0000-7000-8000-00000000c0c0").unwrap();
    let existing: Option<Identity> = identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq("test.local"))
        .select(Identity::as_select())
        .first(&mut conn)
        .optional()
        .unwrap();
    let identity = match existing {
        Some(i) => i,
        None => diesel::insert_into(identities::table)
            .values(NewIdentity {
                id: vcsstar_db::new_id(),
                lk_user_id: user_id,
                lk_domain: "test.local",
                handle: "credadmin",
                display_name: None,
                email: None,
                email_verified: false,
            })
            .returning(Identity::as_returning())
            .get_result(&mut conn)
            .unwrap(),
    };
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

#[test]
fn two_credentials_form_a_set_and_both_get_picked() {
    let Some(app) = app() else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: Some(principal(&app)),
        peer: Peer::default(),
    };
    let suffix = &uuid::Uuid::now_v7().simple().to_string()[..10];
    let org_name = format!("credorg{suffix}");
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
    std::env::set_var("VCSSTAR_TEST_CRED_A", "token-a");
    std::env::set_var("VCSSTAR_TEST_CRED_B", "token-b");
    let mut ids = vec![];
    for var in ["env:VCSSTAR_TEST_CRED_A", "env:VCSSTAR_TEST_CRED_B"] {
        let c = mirror::set_credential(
            &app,
            &ctx,
            vcsstar_types::SetCredentialRequest {
                ctx: wire_ctx(),
                org: org_name.clone(),
                repo: None,
                provider: "github".into(),
                purpose: "mirror_git_push".into(),
                auth_method: "pat".into(),
                host: "github.com".into(),
                secret_ref: var.into(),
                secret: None,
                enabled: None,
            },
        )
        .expect("set credential");
        ids.push(c.id);
    }
    assert_ne!(ids[0], ids[1], "the second set-credential adds, not replaces");

    let listed = mirror::list_credentials(
        &app,
        &ctx,
        vcsstar_types::ListCredentialsRequest {
            ctx: wire_ctx(),
            target: vcsstar_types::OrgRepoRef {
                org: org_name.clone(),
                repo: None,
            },
        },
    )
    .expect("list");
    assert_eq!(listed.credentials.len(), 2);

    let mut conn = app.db.get().unwrap();
    let org = vcsstar_core::authz::load_org(&mut conn, &org_name).unwrap();
    let set = cred::candidates(&mut conn, org.id, uuid::Uuid::nil(), "mirror_git_push").unwrap();
    assert_eq!(set.len(), 2);
    let mut seen = HashSet::new();
    for _ in 0..64 {
        let (purpose, token) =
            cred::token_for(&app, &mut conn, org.id, uuid::Uuid::nil(), "mirror_git_push")
                .expect("token");
        assert_eq!(purpose, "mirror_git_push");
        seen.insert(token);
    }
    assert_eq!(
        seen,
        HashSet::from(["token-a".to_string(), "token-b".to_string()]),
        "both members are picked over 64 draws"
    );

    mirror::remove_credential(
        &app,
        &ctx,
        vcsstar_types::CredentialRef {
            ctx: wire_ctx(),
            credential_id: ids[0].clone(),
        },
    )
    .expect("remove");
    let set = cred::candidates(&mut conn, org.id, uuid::Uuid::nil(), "mirror_git_push").unwrap();
    assert_eq!(set.len(), 1);
    let (_, token) =
        cred::token_for(&app, &mut conn, org.id, uuid::Uuid::nil(), "mirror_git_push").unwrap();
    assert_eq!(token, "token-b");
}
