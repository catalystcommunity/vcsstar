//! Browser login against a fake RP, through the real database. Runs only
//! when DATABASE_URL points at a migrated dev database.

use std::sync::{Arc, Mutex};

use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{
    decode_rp_decrypt_request, decode_rp_sign_request, decode_rp_user_info_request,
    decode_rp_verify_request, encode_rp_decrypt_response, encode_rp_sign_response,
    encode_rp_verify_response, encode_user_info,
};
use vcsstar_core::linkkeys::RpCaller;
use vcsstar_core::principal::ReqCtx;
use vcsstar_core::services::login;
use vcsstar_core::{App, Config, Error};
use vcsstar_transport::Peer;

struct FakeRp {
    nonce: Mutex<Option<String>>,
    verified: bool,
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
                    user_id: "0193a000-0000-7000-8000-000000000042".into(),
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
                assert_eq!(req.domain, "todandlorna.com");
                let claim = |t: &str, v: &str| lk::Claim {
                    claim_id: format!("c-{t}"),
                    user_id: "0193a000-0000-7000-8000-000000000042".into(),
                    claim_type: t.into(),
                    claim_value: v.as_bytes().to_vec(),
                    signatures: vec![lk::ClaimSignature {
                        domain: "todandlorna.com".into(),
                        signed_by_key_id: "k1".into(),
                        signature: vec![1, 2, 3],
                    }],
                    attested_at: "2026-01-01T00:00:00Z".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                    expires_at: None,
                    revoked_at: None,
                };
                Ok(encode_user_info(&lk::UserInfo {
                    user_id: "0193a000-0000-7000-8000-000000000042".into(),
                    domain: "todandlorna.com".into(),
                    display_name: "Alice".into(),
                    claims: vec![
                        claim("handle", "alice"),
                        claim("email", "alice@example.com"),
                    ],
                }))
            }
            other => panic!("unexpected rp op {other}"),
        }
    }
}

fn app(verified: bool) -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    std::env::set_var("VCSSTAR__LINKKEYS_RP_DOMAIN", "catalystsquad.com");
    std::env::set_var("VCSSTAR__PUBLIC_URL", "http://test.local");
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::with_rp(
        config,
        db,
        Arc::new(FakeRp {
            nonce: Mutex::new(None),
            verified,
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
