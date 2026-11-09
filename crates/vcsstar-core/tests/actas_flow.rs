//! Act-as audience flow against the real database with a fake RP: a home
//! domain key signs the grantee's attestations and the grant; the grantee
//! presents a credential; the instance issues a scoped bearer once.

use std::sync::Arc;

use chrono::{Duration, Utc};
use diesel::prelude::*;
use liblinkkeys::act_as as aa;
use liblinkkeys::application_keys::{
    self as ak, ApplicationKeyRef, ApplicationSigner, InstanceRef,
};
use liblinkkeys::claims::ClaimSigner;
use liblinkkeys::crypto::{fingerprint, generate_ed25519_keypair, SigningAlgorithm};
use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{self as g};
use vcsstar_core::linkkeys::RpCaller;
use vcsstar_core::principal::{CredentialKind, ReqCtx};
use vcsstar_core::services::actas;
use vcsstar_core::{instance_keys, App, Config, Error};
use vcsstar_db::models::NewIdentity;
use vcsstar_db::schema::{identities, instance_keys as ik};
use vcsstar_transport::Peer;

const DOMAIN: &str = "test.local";

struct Key {
    id: String,
    public: Vec<u8>,
    private: Vec<u8>,
}

fn key(id: &str) -> Key {
    let (vk, sk) = generate_ed25519_keypair();
    Key {
        id: id.into(),
        public: vk.as_bytes().to_vec(),
        private: sk.to_bytes().to_vec(),
    }
}

fn domain_public(k: &Key) -> lk::DomainPublicKey {
    lk::DomainPublicKey {
        key_id: k.id.clone(),
        public_key: k.public.clone(),
        fingerprint: fingerprint(&k.public),
        algorithm: "ed25519".into(),
        key_usage: "sign".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2030-01-01T00:00:00Z".into(),
        revoked_at: None,
        signed_by_key_id: None,
        key_signature: None,
    }
}

fn key_ref(k: &Key) -> ApplicationKeyRef {
    ApplicationKeyRef {
        key_id: k.id.clone(),
        key_usage: "sign".into(),
        algorithm: "ed25519".into(),
        public_key: k.public.clone(),
        fingerprint: fingerprint(&k.public),
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2030-01-01T00:00:00Z".into(),
        revoked_at: None,
    }
}

/// The RP as the audience sees it: home-domain keys, grantee attestations,
/// and no revocations.
struct FakeRp {
    domain: Key,
    grantee_subject: String,
    grantee_instance: String,
    grantee_keys: Vec<Key>,
}

impl RpCaller for FakeRp {
    fn call(&self, op: &str, payload: Vec<u8>) -> Result<Vec<u8>, Error> {
        match op {
            "resolve-domain-keys" => {
                let req = g::decode_rp_resolve_domain_keys_request(&payload).unwrap();
                assert_eq!(req.domain, DOMAIN);
                Ok(g::encode_rp_resolve_domain_keys_response(
                    &lk::RpResolveDomainKeysResponse {
                        domain: DOMAIN.into(),
                        keys: vec![domain_public(&self.domain)],
                        revocations: vec![],
                        fetched_at: Utc::now().to_rfc3339(),
                        revocations_checked_at: Utc::now().to_rfc3339(),
                        cache_status: "fresh".into(),
                    },
                ))
            }
            "resolve-application-keys" => {
                let req = g::decode_rp_resolve_application_keys_request(&payload).unwrap();
                assert_eq!(req.subject_user_id, self.grantee_subject);
                assert_eq!(req.instance_id, self.grantee_instance);
                let instance = InstanceRef {
                    subject_user_id: &self.grantee_subject,
                    subject_domain: DOMAIN,
                    application_id: &req.application_id,
                    instance_id: &self.grantee_instance,
                };
                let signer = ClaimSigner {
                    domain: DOMAIN,
                    key_id: &self.domain.id,
                    algorithm: SigningAlgorithm::Ed25519,
                    private_key_bytes: &self.domain.private,
                };
                let atts = self
                    .grantee_keys
                    .iter()
                    .map(|k| {
                        let a = ak::build_attestation(
                            &instance,
                            &key_ref(k),
                            Utc::now() - Duration::minutes(1),
                            86_400,
                        );
                        ak::sign_attestation(&a, std::slice::from_ref(&signer)).unwrap()
                    })
                    .collect();
                Ok(g::encode_rp_resolve_application_keys_response(
                    &lk::RpResolveApplicationKeysResponse {
                        subject_user_id: self.grantee_subject.clone(),
                        subject_domain: DOMAIN.into(),
                        application_id: req.application_id,
                        instance_id: self.grantee_instance.clone(),
                        application_keys: atts,
                        application_key_revocations: vec![],
                        home_domain_keys: vec![domain_public(&self.domain)],
                        home_domain_key_revocations: vec![],
                        fetched_at: Utc::now().to_rfc3339(),
                        revocations_checked_at: Utc::now().to_rfc3339(),
                        cache_status: "fresh".into(),
                    },
                ))
            }
            "resolve-act-as-revocations" => Ok(g::encode_get_act_as_grant_revocations_response(
                &lk::GetActAsGrantRevocationsResponse {
                    revocations: vec![],
                },
            )),
            other => panic!("unexpected rp op {other}"),
        }
    }
}

#[test]
fn act_as_credential_becomes_a_scoped_bearer_once() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let instance_account = uuid::Uuid::now_v7();
    let user_id = uuid::Uuid::now_v7();
    let grantee_subject = uuid::Uuid::now_v7();
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__PUBLIC_URL", "http://audience.test.local");
    std::env::set_var("VCSSTAR__INSTANCE_DOMAIN", DOMAIN);
    std::env::set_var(
        "VCSSTAR__INSTANCE_IDENTITY",
        format!("{instance_account}@{DOMAIN}"),
    );
    std::env::set_var("VCSSTAR__LINKKEYS_RP_DOMAIN", DOMAIN);
    std::env::set_var(
        "VCSSTAR__REPO_CACHE_DIR",
        std::env::temp_dir().join("vcsstar-actas-test"),
    );
    let mut key_bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key_bytes);
    std::env::set_var(
        "SECRET_VCSSTAR__CREDENTIAL_KEY",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key_bytes),
    );
    let config = Config::from_env().unwrap();
    let db = vcsstar_db::pool(&url, 2).unwrap();
    vcsstar_db::run_migrations(&mut db.get().unwrap()).unwrap();
    let domain = key("dom-1");
    let domain_key_for_signing = (domain.id.clone(), domain.private.clone());
    let grantee_keys = vec![key("g-1"), key("g-2")];
    let (gk_id, gk_priv) = (grantee_keys[0].id.clone(), grantee_keys[0].private.clone());
    let grantee_instance = "agent-box-1".to_string();
    let rp = FakeRp {
        domain,
        grantee_subject: grantee_subject.to_string(),
        grantee_instance: grantee_instance.clone(),
        grantee_keys,
    };
    let app = App::with_rp(config, db, Arc::new(rp));
    let mut conn = app.conn().unwrap();

    // The user signed in once before.
    diesel::insert_into(identities::table)
        .values(&NewIdentity {
            id: vcsstar_db::new_id(),
            lk_user_id: user_id,
            lk_domain: DOMAIN,
            handle: "alice",
            display_name: None,
            email: None,
            email_verified: false,
        })
        .execute(&mut conn)
        .unwrap();
    // The instance's own keys, as enrollment would have stored them.
    diesel::delete(ik::table).execute(&mut conn).unwrap();
    let instance_id = instance_keys::instance_id(&app);
    for role in ["signing-a", "signing-b"] {
        let k = key(role);
        diesel::insert_into(ik::table)
            .values((
                ik::key_id.eq(&k.id),
                ik::instance_id.eq(&instance_id),
                ik::role.eq(role),
                ik::public_key.eq(&k.public),
                ik::private_sealed.eq(vcsstar_core::mirror::cred::seal(
                    &app,
                    hex::encode(&k.private).as_bytes(),
                )
                .unwrap()),
                ik::attested_at.eq(Utc::now()),
                ik::attestation_expires_at.eq(Utc::now() + Duration::days(1)),
            ))
            .execute(&mut conn)
            .unwrap();
    }
    let ctx = ReqCtx {
        request_id: "t".into(),
        principal: None,
        peer: Peer::default(),
    };
    let wire_ctx = vcsstar_types::Ctx {
        request_id: "t".into(),
        deadline_ms: None,
        contract_version: None,
        contract_digest: None,
    };

    // 1. The grantee asks the audience for its scope set.
    let grantee_app = lk::ApplicationRef {
        subject_user_id: grantee_subject.to_string(),
        subject_domain: DOMAIN.into(),
        application_id: "agent-cli".into(),
    };
    let grantee_ref = lk::GranteeRef {
        application: Some(grantee_app.clone()),
        local_rp_descriptor_fingerprint: None,
    };
    let set = actas::scope_set(
        &app,
        &ctx,
        vcsstar_types::ScopeSetRequest {
            ctx: wire_ctx.clone(),
            grantee_cbor: g::encode_grantee_ref(&grantee_ref),
            scopes: vec!["repo:read".into(), "pr:write".into()],
        },
    )
    .expect("scope set");
    let signed_set = g::decode_signed_act_as_scope_set(&set.scope_set_cbor).unwrap();

    // 2. The home domain issues the grant (the user approved repo:read only).
    let audience = lk::ApplicationRef {
        subject_user_id: instance_account.to_string(),
        subject_domain: DOMAIN.into(),
        application_id: "vcsstar".into(),
    };
    let now = Utc::now();
    let approved = vec!["repo:read".to_string()];
    let grant = aa::build_grant(&aa::NewGrant {
        grant_id: "grant-1",
        user_id: &user_id.to_string(),
        subject_domain: DOMAIN,
        grantee: &grantee_ref,
        audience: &audience,
        scope_set: &signed_set,
        approved_scope: &approved,
        lifetime_seconds: 1800,
        renewal_window_seconds: 3600,
        now,
    })
    .unwrap();
    let signed_grant = aa::sign_grant(
        &grant,
        &[ClaimSigner {
            domain: DOMAIN,
            key_id: &domain_key_for_signing.0,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &domain_key_for_signing.1,
        }],
    )
    .unwrap();

    // Sanity: the pieces verify on their own before the audience checklist.
    let dom_pub = domain_public(&Key {
        id: domain_key_for_signing.0.clone(),
        public: {
            let sk = ed25519_dalek::SigningKey::from_bytes(
                domain_key_for_signing.1.as_slice().try_into().unwrap(),
            );
            sk.verifying_key().as_bytes().to_vec()
        },
        private: domain_key_for_signing.1.clone(),
    });
    aa::verify_grant_signature(&signed_grant, std::slice::from_ref(&dom_pub))
        .expect("grant signature verifies");
    let instance_ref = InstanceRef {
        subject_user_id: &grantee_subject.to_string(),
        subject_domain: DOMAIN,
        application_id: "agent-cli",
        instance_id: &grantee_instance,
    };
    let gref = ApplicationKeyRef {
        key_id: gk_id.clone(),
        key_usage: "sign".into(),
        algorithm: "ed25519".into(),
        public_key: ed25519_dalek::SigningKey::from_bytes(gk_priv.as_slice().try_into().unwrap())
            .verifying_key()
            .as_bytes()
            .to_vec(),
        fingerprint: String::new(),
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2030-01-01T00:00:00Z".into(),
        revoked_at: None,
    };
    let gref = ApplicationKeyRef {
        fingerprint: fingerprint(&gref.public_key),
        ..gref
    };
    let att = ak::build_attestation(
        &instance_ref,
        &gref,
        Utc::now() - Duration::minutes(1),
        86_400,
    );
    let signed_att = ak::sign_attestation(
        &att,
        &[ClaimSigner {
            domain: DOMAIN,
            key_id: &domain_key_for_signing.0,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &domain_key_for_signing.1,
        }],
    )
    .unwrap();
    let set = ak::verify_application_key_set(
        &[signed_att],
        &[],
        std::slice::from_ref(&dom_pub),
        &instance_ref,
        Utc::now(),
        300,
    );
    assert!(
        set.rejected.is_empty(),
        "attestations rejected: {:?}",
        set.rejected
    );
    assert!(
        set.keys.iter().all(|k| k.is_usable()),
        "keys not usable: {:?}",
        set.keys.iter().map(|k| &k.status).collect::<Vec<_>>()
    );

    // 3. The grantee presents the credential for the exchange.
    let signer = aa::GranteeSigner::Application {
        instance_id: &grantee_instance,
        signer: ApplicationSigner {
            key_id: &gk_id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &gk_priv,
        },
    };
    let nonce: [u8; 16] = rand::random();
    let credential = aa::present(
        &signed_grant,
        &audience,
        &actas::request_digest(),
        now,
        &nonce,
        &signer,
    )
    .unwrap();
    let bearer = actas::exchange(
        &app,
        &ctx,
        vcsstar_types::ActAsExchangeRequest {
            ctx: wire_ctx.clone(),
            credential_cbor: g::encode_act_as_credential(&credential),
            requested_scopes: vec![],
        },
    )
    .expect("exchange");
    assert_eq!(bearer.scopes, vec!["repo:read".to_string()]);
    assert!(bearer.bearer.starts_with("vsa_"));
    assert_eq!(bearer.subject.user_id, user_id.to_string());

    // The bearer resolves to the user, flagged as act-as.
    let p = vcsstar_core::principal::resolve(
        &app,
        &Peer {
            credential: Some(bearer.bearer.clone()),
            ..Peer::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(p.kind, CredentialKind::ActAs);
    assert!(p.has_scope("repo:read"));
    assert!(!p.has_scope("repo:write"));
    assert!(p.acting_via.as_deref().unwrap_or("").contains("agent-cli"));

    // 4. Replaying the same presentation is refused.
    let again = actas::exchange(
        &app,
        &ctx,
        vcsstar_types::ActAsExchangeRequest {
            ctx: wire_ctx.clone(),
            credential_cbor: g::encode_act_as_credential(&credential),
            requested_scopes: vec![],
        },
    );
    assert!(again.is_err());

    // 5. A presentation bound to another request digest is refused.
    let bad = aa::present(
        &signed_grant,
        &audience,
        b"other",
        now,
        &rand::random::<[u8; 16]>(),
        &signer,
    )
    .unwrap();
    assert!(actas::exchange(
        &app,
        &ctx,
        vcsstar_types::ActAsExchangeRequest {
            ctx: wire_ctx,
            credential_cbor: g::encode_act_as_credential(&bad),
            requested_scopes: vec![],
        },
    )
    .is_err());
}
