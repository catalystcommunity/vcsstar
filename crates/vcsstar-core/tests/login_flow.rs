//! Browser login against a fake RP, through the real database. Runs only
//! when DATABASE_URL points at a migrated dev database.

use std::sync::{Arc, Mutex};

use liblinkkeys::claims::{sign_claim, ClaimSigner, ClaimSpec};
use liblinkkeys::crypto::{fingerprint, generate_keypair, SigningAlgorithm};
use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{
    decode_rp_decrypt_request, decode_rp_resolve_domain_keys_request, decode_rp_sign_request,
    decode_rp_user_info_request, decode_rp_verify_request, encode_rp_decrypt_response,
    encode_rp_resolve_domain_keys_response, encode_rp_sign_response, encode_rp_verify_response,
    encode_user_info,
};
use vcsstar_core::linkkeys::RpCaller;
use vcsstar_core::principal::ReqCtx;
use vcsstar_core::services::login;
use vcsstar_core::{App, Config, Error};
use vcsstar_transport::Peer;

const HOME: &str = "todandlorna.com";
const USER: &str = "0193a000-0000-7000-8000-000000000042";

/// The fake home domain's one signing key. Claims are signed with it and
/// `resolve-domain-keys` serves its public half, so the real verification
/// path in `login::complete` runs.
struct HomeKey {
    public: Vec<u8>,
    private: Vec<u8>,
}

impl HomeKey {
    fn new() -> Self {
        let (public, private) = generate_keypair(SigningAlgorithm::Ed25519);
        Self { public, private }
    }

    fn domain_key(&self) -> lk::DomainPublicKey {
        let now = chrono::Utc::now();
        lk::DomainPublicKey {
            key_id: "k1".into(),
            public_key: self.public.clone(),
            fingerprint: fingerprint(&self.public),
            algorithm: "ed25519".into(),
            key_usage: "sign".into(),
            created_at: (now - chrono::Duration::days(1)).to_rfc3339(),
            expires_at: (now + chrono::Duration::days(1)).to_rfc3339(),
            revoked_at: None,
            signed_by_key_id: None,
            key_signature: None,
        }
    }

    fn claim(&self, claim_type: &str, value: &str) -> lk::Claim {
        let attested = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        sign_claim(
            &ClaimSpec {
                claim_id: &format!("c-{claim_type}"),
                claim_type,
                claim_value: value.as_bytes(),
                user_id: USER,
                subject_domain: HOME,
                expires_at: None,
                attested_at: &attested,
            },
            &[ClaimSigner {
                domain: HOME,
                key_id: "k1",
                algorithm: SigningAlgorithm::Ed25519,
                private_key_bytes: &self.private,
            }],
        )
        .expect("sign claim")
    }
}

struct FakeRp {
    nonce: Mutex<Option<String>>,
    verified: bool,
    home: HomeKey,
}

impl RpCaller for FakeRp {
    fn call(&self, op: &str, payload: Vec<u8>) -> Result<Vec<u8>, Error> {
        match op {
            "sign-request" => {
                let req = decode_rp_sign_request(&payload).unwrap();
                assert!(req
                    .requested_claims
                    .as_ref()
                    .unwrap()
                    .required
                    .iter()
                    .any(|c| c.claim_type == "handle"));
                *self.nonce.lock().unwrap() = Some(req.nonce.clone());
                Ok(encode_rp_sign_response(&lk::RpSignResponse {
                    signed_request: format!("signed:{}", req.nonce),
                }))
            }
            "decrypt-token" => {
                let req = decode_rp_decrypt_request(&payload).unwrap();
                Ok(encode_rp_decrypt_response(&lk::RpDecryptResponse {
                    signed_assertion: format!("assertion-for:{}", req.encrypted_token),
                }))
            }
            "verify-assertion" => {
                let req = decode_rp_verify_request(&payload).unwrap();
                let now = chrono::Utc::now();
                let assertion = lk::IdentityAssertion {
                    user_id: USER.into(),
                    domain: req.expected_domain.clone(),
                    audience: "catalystsquad.com".into(),
                    nonce: self.nonce.lock().unwrap().clone().unwrap_or_default(),
                    issued_at: now.to_rfc3339(),
                    expires_at: (now + chrono::Duration::minutes(4)).to_rfc3339(),
                    authorized_claims: vec!["handle".into()],
                    display_name: Some("Alice".into()),
                };
                Ok(encode_rp_verify_response(&lk::RpVerifyResponse {
                    assertion,
                    verified: self.verified,
                }))
            }
            "userinfo-fetch" => {
                let req = decode_rp_user_info_request(&payload).unwrap();
                assert_eq!(req.domain, HOME);
                Ok(encode_user_info(&lk::UserInfo {
                    user_id: USER.into(),
                    domain: HOME.into(),
                    display_name: "Alice".into(),
                    claims: vec![
                        self.home.claim("handle", "alice"),
                        self.home.claim("email", "alice@example.com"),
                    ],
                }))
            }
            "resolve-domain-keys" => {
                let req = decode_rp_resolve_domain_keys_request(&payload).unwrap();
                assert_eq!(req.domain, HOME);
                let now = chrono::Utc::now().to_rfc3339();
                Ok(encode_rp_resolve_domain_keys_response(
                    &lk::RpResolveDomainKeysResponse {
                        domain: HOME.into(),
                        keys: vec![self.home.domain_key()],
                        revocations: vec![],
                        fetched_at: now.clone(),
                        revocations_checked_at: now,
                        cache_status: "fresh".into(),
                    },
                ))
            }
            other => panic!("unexpected rp op {other}"),
        }
    }
}

fn app(verified: bool) -> Option<App> {
    app_with(verified, |_| {})
}

fn app_with(verified: bool, tune: impl FnOnce(&mut Config)) -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__LINKKEYS_RP_DOMAIN", "catalystsquad.com");
    std::env::set_var("VCSSTAR__PUBLIC_URL", "http://test.local");
    let mut config = Config::from_env().ok()?;
    tune(&mut config);
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::with_rp(
        config,
        db,
        Arc::new(FakeRp {
            nonce: Mutex::new(None),
            verified,
            home: HomeKey::new(),
        }),
    ))
}

fn ctx() -> ReqCtx {
    ReqCtx {
        request_id: "t".into(),
        principal: None,
        peer: Peer::default(),
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
fn browser_login_round_trip() {
    let Some(app) = app(true) else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let begun = login::begin(
        &app,
        &ctx(),
        vcsstar_types::BeginLoginRequest {
            ctx: wire_ctx(),
            identity_input: "alice@todandlorna.com".into(),
            return_to: Some("/repos".into()),
            device_user_code: None,
        },
    )
    .expect("begin");
    assert!(
        begun.redirect_url.starts_with("https://"),
        "{}",
        begun.redirect_url
    );
    assert!(begun
        .redirect_url
        .contains("/auth/authorize?signed_request=signed%3A"));
    assert!(begun.redirect_url.ends_with("&username=alice"));

    let session = login::complete(
        &app,
        &ctx(),
        vcsstar_types::CompleteLoginRequest {
            ctx: wire_ctx(),
            attempt_token: begun.attempt_token.clone(),
            encrypted_token: "enc".into(),
        },
    )
    .expect("complete");
    assert_eq!(session.identity.handle.as_deref(), Some("alice"));
    assert_eq!(session.identity.domain, "todandlorna.com");
    assert_eq!(session.return_to.as_deref(), Some("/repos"));
    assert!(session.session_token.starts_with("vss_"));

    // The attempt is single use.
    let again = login::complete(
        &app,
        &ctx(),
        vcsstar_types::CompleteLoginRequest {
            ctx: wire_ctx(),
            attempt_token: begun.attempt_token,
            encrypted_token: "enc".into(),
        },
    );
    assert!(again.is_err());

    // The session resolves to the identity with full scopes.
    let peer = Peer {
        credential: Some(session.session_token),
        ..Peer::default()
    };
    let p = vcsstar_core::principal::resolve(&app, &peer)
        .expect("resolve")
        .expect("principal");
    assert_eq!(p.identity.handle, "alice");
    assert_eq!(p.identity.email.as_deref(), Some("alice@example.com"));
    assert!(p.identity.email_verified);
    assert!(p.has_scope("repo:admin"));
}

#[test]
fn unverified_assertion_is_refused() {
    let Some(app) = app(false) else {
        return;
    };
    let begun = login::begin(
        &app,
        &ctx(),
        vcsstar_types::BeginLoginRequest {
            ctx: wire_ctx(),
            identity_input: "alice@todandlorna.com".into(),
            return_to: None,
            device_user_code: None,
        },
    )
    .unwrap();
    let err = login::complete(
        &app,
        &ctx(),
        vcsstar_types::CompleteLoginRequest {
            ctx: wire_ctx(),
            attempt_token: begun.attempt_token,
            encrypted_token: "enc".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, vcsstar_core::Code::Unauthenticated);
}

#[test]
fn login_begin_is_rate_limited_per_peer_and_identity() {
    let Some(app) = app_with(true, |c| c.rate_limits.login_per_minute = 2) else {
        return;
    };
    let begin = |identity: &str| {
        login::begin(
            &app,
            &ctx(),
            vcsstar_types::BeginLoginRequest {
                ctx: wire_ctx(),
                identity_input: identity.into(),
                return_to: None,
                device_user_code: None,
            },
        )
    };
    begin("alice@todandlorna.com").expect("first");
    begin("alice@todandlorna.com").expect("second");
    let err = begin("alice@todandlorna.com").expect_err("third is over the limit");
    assert_eq!(err.code, vcsstar_core::Code::ResourceExhausted);
    // The peer bucket is spent too, so another identity from the same peer waits.
    let err = begin("bob@todandlorna.com").expect_err("peer bucket");
    assert_eq!(err.code, vcsstar_core::Code::ResourceExhausted);
}
