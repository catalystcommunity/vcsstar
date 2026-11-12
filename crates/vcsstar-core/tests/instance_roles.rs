//! Instance roles: only an instance admin sets them; a "viewer" reads
//! private repositories and nothing more; "admin" acts as an instance
//! admin; the role is read when the credential resolves; the caller's
//! role shows on WhoAmI, Org, and Repo. Runs only when DATABASE_URL points
//! at a migrated dev database.

use diesel::prelude::*;
use vcsstar_core::principal::{self, CredentialKind, Principal, ReqCtx};
use vcsstar_core::services::{auth, instance, orgs, repos};
use vcsstar_core::{App, Code, Config};
use vcsstar_db::models::{Identity, NewIdentity};
use vcsstar_db::schema::identities;
use vcsstar_transport::Peer;

fn app() -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__PUBLIC_URL", "https://vcsstar.test.local:8443");
    // Repositories made here go to a scratch cache, not the source tree.
    let cache = std::env::temp_dir().join(format!("vcsstar-instance-roles-{}", std::process::id()));
    std::env::set_var("VCSSTAR__REPO_CACHE_DIR", &cache);
    std::env::set_var(
        "VCSSTAR__INSTANCE_ADMINS",
        "0193a000-0000-7000-8000-00000000d0d2@test.local",
    );
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::new(config, db))
}

fn admin(app: &App) -> Principal {
    let mut conn = app.db.get().unwrap();
    let user_id = uuid::Uuid::parse_str("0193a000-0000-7000-8000-00000000d0d2").unwrap();
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
                handle: "iradmin",
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

fn person(app: &App) -> Identity {
    let mut conn = app.db.get().unwrap();
    diesel::insert_into(identities::table)
        .values(NewIdentity {
            id: vcsstar_db::new_id(),
            lk_user_id: uuid::Uuid::now_v7(),
            lk_domain: "test.local",
            handle: "someone",
            display_name: None,
            email: None,
            email_verified: false,
        })
        .returning(Identity::as_returning())
        .get_result(&mut conn)
        .unwrap()
}

/// A real API token of the identity, resolved the way a request is.
fn token_of(app: &App, identity: &Identity) -> String {
    let p = Principal {
        identity: identity.clone(),
        scopes: vec!["*".into()],
        kind: CredentialKind::Session,
        acting_via: None,
        credential_id: None,
        expires_at: None,
        instance_role: None,
    };
    auth::create_token(
        app,
        &ctx_for(&p),
        vcsstar_types::CreateTokenRequest {
            ctx: wire_ctx(),
            name: "t".into(),
            scopes: vec!["admin".into(), "repo:admin".into()],
            client_label: None,
            expires_at: None,
        },
    )
    .unwrap()
    .secret
    .unwrap()
}

#[test]
fn instance_roles_are_set_by_admins_and_shape_what_a_caller_sees() {
    let Some(app) = app() else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let admin_p = admin(&app);
    let admin_ctx = ctx_for(&admin_p);
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let org_name = format!("irorg{}", &suffix[suffix.len() - 8..]);
    orgs::create(
        &app,
        &admin_ctx,
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
        &admin_ctx,
        vcsstar_types::CreateRepoRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
            name: "secret".into(),
            vcs_type: None,
            visibility: Some("private".into()),
            default_branch: None,
            description: None,
        },
    )
    .expect("repo");
    let get = || vcsstar_types::GetRepoRequest {
        ctx: wire_ctx(),
        repo: vcsstar_types::RepoRef {
            org: org_name.clone(),
            name: "secret".into(),
            instance: None,
        },
    };
    let someone = person(&app);
    let key = format!("{}@{}", someone.lk_user_id, someone.lk_domain);
    let secret = token_of(&app, &someone);
    let as_someone = || ctx_for(&resolve(&app, &secret).unwrap().unwrap());

    // No role: the private repository does not exist for them, and they
    // cannot hand out roles.
    assert_eq!(
        repos::get(&app, &as_someone(), get()).unwrap_err().code,
        Code::NotFound
    );
    let refused = instance::set_role(
        &app,
        &as_someone(),
        vcsstar_types::SetInstanceRoleRequest {
            ctx: wire_ctx(),
            identity: key.clone(),
            role: "admin".into(),
        },
    )
    .unwrap_err();
    assert_eq!(refused.code, Code::Forbidden);

    let set = |role: &str| {
        instance::set_role(
            &app,
            &admin_ctx,
            vcsstar_types::SetInstanceRoleRequest {
                ctx: wire_ctx(),
                identity: key.clone(),
                role: role.into(),
            },
        )
    };
    assert!(set("owner").is_err(), "unknown role");
    let g = set("viewer").expect("viewer");
    assert_eq!((g.role.as_str(), g.source.as_str()), ("viewer", "manual"));
    let r = repos::get(&app, &as_someone(), get()).expect("a viewer reads it");
    assert_eq!(r.viewer_role.as_deref(), Some("read"));
    let who = auth::whoami(
        &app,
        &as_someone(),
        vcsstar_types::Empty { ctx: wire_ctx() },
    )
    .unwrap();
    assert_eq!(who.instance_role.as_deref(), Some("viewer"));
    let org = orgs::get(
        &app,
        &as_someone(),
        vcsstar_types::GetOrgRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
        },
    )
    .unwrap();
    assert_eq!(org.viewer_role, None, "a viewer is no member");
    // A viewer cannot change the repository.
    let changed = repos::update(
        &app,
        &as_someone(),
        vcsstar_types::UpdateRepoRequest {
            ctx: wire_ctx(),
            repo: get().repo,
            description: Some("x".into()),
            default_branch: None,
            visibility: None,
            archived: None,
            new_name: None,
        },
    );
    assert_eq!(changed.unwrap_err().code, Code::Forbidden);

    // Admin: acts as an instance admin.
    set("admin").expect("admin");
    let org = orgs::get(
        &app,
        &as_someone(),
        vcsstar_types::GetOrgRequest {
            ctx: wire_ctx(),
            org: org_name.clone(),
        },
    )
    .unwrap();
    assert_eq!(org.viewer_role.as_deref(), Some("owner"));
    let r = repos::get(&app, &as_someone(), get()).unwrap();
    assert_eq!(r.viewer_role.as_deref(), Some("admin"));
    let listed = instance::list_roles(
        &app,
        &as_someone(),
        vcsstar_types::Empty { ctx: wire_ctx() },
    )
    .unwrap();
    assert!(listed
        .grants
        .iter()
        .any(|g| g.identity.user_id == someone.lk_user_id.to_string() && g.role == "admin"));
    assert!(listed
        .configured_admins
        .iter()
        .any(|a| a.ends_with("d0d2@test.local")));
    // They cannot remove their own admin role.
    let own = instance::remove_role(
        &app,
        &as_someone(),
        vcsstar_types::RemoveInstanceRoleRequest {
            ctx: wire_ctx(),
            identity: key.clone(),
        },
    );
    assert!(own.is_err());

    instance::remove_role(
        &app,
        &admin_ctx,
        vcsstar_types::RemoveInstanceRoleRequest {
            ctx: wire_ctx(),
            identity: key.clone(),
        },
    )
    .expect("removed");
    assert_eq!(resolve(&app, &secret).unwrap().unwrap().instance_role, None);
    assert_eq!(
        repos::get(&app, &as_someone(), get()).unwrap_err().code,
        Code::NotFound
    );
}
