//! vcsstar as a LinkKeys act-as audience. A grantee application (an agent,
//! a CI runner) presents an `ActAsCredential`; after the full checklist the
//! instance issues a short bearer carrying only the approved scopes. Every
//! action made with it is attributed to the user with the grantee recorded
//! as `acting_via`.

use chrono::{Duration, Utc};
use diesel::prelude::*;
use liblinkkeys::act_as as aa;
use liblinkkeys::application_keys::{self as ak, ApplicationKeyRef, InstanceRef};
use liblinkkeys::crypto::SigningAlgorithm;
use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{self as g};
use sha2::Digest;
use vcsstar_db::models::{Identity, NewApiToken};
use vcsstar_db::schema::{act_as_grants, act_as_nonces, api_tokens, identities};
use vcsstar_types::{ActAsExchangeRequest, ScopeSetRequest, ScopedBearer, SignedActAsScopeSet};

use crate::principal::{identity_to_wire, ReqCtx};
use crate::tokens::{self, ACT_AS_PREFIX};
use crate::{instance_keys, now_ms, to_ms, App, Code, Error};

/// The scopes this audience offers, with consent-screen descriptions.
pub const SCOPES: &[(&str, &str)] = &[
    ("repo:read", "Read repositories you can see"),
    (
        "repo:write",
        "Push and change refs in repositories you can write",
    ),
    (
        "pr:write",
        "Open, update, comment on, and merge pull requests",
    ),
    ("pr:review", "Review pull requests"),
    ("statuses:write", "Report commit statuses"),
    ("org:read", "Read organizations and memberships"),
];

/// Grantees bind each presentation to this digest. The exchange is one
/// operation, so the digest is a constant the grantee can compute.
pub const REQUEST_DIGEST_INPUT: &str = "vcsstar-act-as-exchange-v1";

pub fn request_digest() -> Vec<u8> {
    sha2::Sha256::digest(REQUEST_DIGEST_INPUT.as_bytes()).to_vec()
}

const MAX_BEARER_SECONDS: i64 = 3600;
const SCOPE_SET_SECONDS: i64 = 600;

fn own_application(app: &App) -> Result<lk::ApplicationRef, Error> {
    let (user, domain) = instance_keys::account(app)?;
    Ok(lk::ApplicationRef {
        subject_user_id: user,
        subject_domain: domain,
        application_id: instance_keys::APPLICATION_ID.into(),
    })
}

/// Sign a scope set for one grantee. Unauthenticated: the grantee calls
/// this before it has any standing here.
pub fn scope_set(
    app: &App,
    _ctx: &ReqCtx,
    req: ScopeSetRequest,
) -> Result<SignedActAsScopeSet, Error> {
    let grantee = g::decode_grantee_ref(&req.grantee_cbor)
        .map_err(|e| Error::invalid(format!("grantee_cbor is not a GranteeRef: {e}")))?;
    aa::check_grantee(&grantee).map_err(|e| Error::invalid(format!("grantee: {e}")))?;
    let wanted: Vec<&str> = if req.scopes.is_empty() {
        SCOPES.iter().map(|(s, _)| *s).collect()
    } else {
        for s in &req.scopes {
            if !SCOPES.iter().any(|(k, _)| k == s) {
                return Err(Error::invalid(format!("unknown scope {s}")));
            }
        }
        req.scopes.iter().map(String::as_str).collect()
    };
    let entries: Vec<lk::ActAsScopeEntry> = SCOPES
        .iter()
        .filter(|(s, _)| wanted.contains(s))
        .map(|(s, d)| lk::ActAsScopeEntry {
            scope: s.to_string(),
            description: Some(d.to_string()),
        })
        .collect();
    let now = Utc::now();
    let set = lk::ActAsScopeSet {
        audience: own_application(app)?,
        grantee,
        entries,
        language: Some("en-US".into()),
        audience_handle_claim: None,
        issued_at: aa::format_time(now),
        expires_at: aa::format_time(now + Duration::seconds(SCOPE_SET_SECONDS)),
    };
    let mut conn = app.conn()?;
    let keys = instance_keys::application_signers(app, &mut conn)?;
    if keys.is_empty() {
        return Err(Error::new(
            Code::PreconditionFailed,
            "this instance has no application keys yet (enrollment pending)",
        ));
    }
    let signers: Vec<ak::ApplicationSigner<'_>> = keys
        .iter()
        .map(|k| ak::ApplicationSigner {
            key_id: &k.key_id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &k.seed,
        })
        .collect();
    let signed = aa::sign_scope_set(&set, &instance_keys::instance_id(app), &signers)
        .map_err(|e| Error::internal(format!("sign scope set: {e}")))?;
    Ok(SignedActAsScopeSet {
        scope_set_cbor: g::encode_signed_act_as_scope_set(&signed),
        scopes: wanted.iter().map(|s| s.to_string()).collect(),
        expires_at: to_ms(now + Duration::seconds(SCOPE_SET_SECONDS)),
    })
}

/// Our own keys as the verifier must see them: every key, including
/// expired and revoked ones, so a grant signed by a rotated key still
/// verifies at its `issued_at`.
fn own_key_refs(
    app: &App,
    conn: &mut PgConnection,
    instance: &str,
) -> Result<Vec<ApplicationKeyRef>, Error> {
    let _ = app;
    Ok(instance_keys::load(conn)?
        .into_iter()
        .filter(|k| k.instance_id == instance)
        .map(|k| ApplicationKeyRef {
            key_id: k.key_id,
            key_usage: ak::KEY_USAGE_SIGN.into(),
            algorithm: "ed25519".into(),
            fingerprint: liblinkkeys::crypto::fingerprint(&k.public_key),
            public_key: k.public_key,
            created_at: aa::format_time(k.created_at),
            expires_at: aa::format_time(k.created_at + Duration::days(365)),
            revoked_at: k.revoked_at.map(aa::format_time),
        })
        .collect())
}

fn rp_domain_keys(app: &App, domain: &str) -> Result<Vec<lk::DomainPublicKey>, Error> {
    let rp = app.rp()?;
    let raw = rp.caller.call(
        "resolve-domain-keys",
        g::encode_rp_resolve_domain_keys_request(&lk::RpResolveDomainKeysRequest {
            domain: domain.to_string(),
            max_cache_age_seconds: Some(300),
        }),
    )?;
    let resp = g::decode_rp_resolve_domain_keys_response(&raw)
        .map_err(|e| Error::new(Code::Unavailable, format!("resolve-domain-keys: {e}")))?;
    if resp.cache_status == "stale" {
        return Err(Error::new(
            Code::Unavailable,
            format!("home domain {domain} unreachable"),
        ));
    }
    // The RP verified these against the DNS anchors of the home domain.
    Ok(resp.keys)
}

fn rp_grantee_keys(
    app: &App,
    grantee: &lk::ApplicationRef,
    instance: &str,
) -> Result<Vec<ApplicationKeyRef>, Error> {
    let rp = app.rp()?;
    let raw = rp.caller.call(
        "resolve-application-keys",
        g::encode_rp_resolve_application_keys_request(&lk::RpResolveApplicationKeysRequest {
            subject_user_id: grantee.subject_user_id.clone(),
            subject_domain: grantee.subject_domain.clone(),
            application_id: grantee.application_id.clone(),
            instance_id: instance.to_string(),
            max_cache_age_seconds: Some(300),
        }),
    )?;
    let resp = g::decode_rp_resolve_application_keys_response(&raw)
        .map_err(|e| Error::new(Code::Unavailable, format!("resolve-application-keys: {e}")))?;
    if resp.cache_status == "stale" {
        return Err(Error::new(
            Code::Unavailable,
            "grantee home domain unreachable",
        ));
    }
    let set = ak::verify_application_key_set(
        &resp.application_keys,
        &resp.application_key_revocations,
        &resp.home_domain_keys,
        &InstanceRef {
            subject_user_id: &grantee.subject_user_id,
            subject_domain: &grantee.subject_domain,
            application_id: &grantee.application_id,
            instance_id: instance,
        },
        Utc::now(),
        300,
    );
    Ok(set
        .keys
        .iter()
        .map(|k| ApplicationKeyRef {
            key_id: k.attestation.key_id.clone(),
            key_usage: k.attestation.key_usage.clone(),
            algorithm: k.attestation.algorithm.clone(),
            public_key: k.attestation.public_key.clone(),
            fingerprint: k.attestation.fingerprint.clone(),
            created_at: k.attestation.key_created_at.clone(),
            expires_at: k.attestation.key_expires_at.clone(),
            revoked_at: match &k.status {
                ak::KeyStatus::Revoked { revoked_at } => Some(revoked_at.clone()),
                _ => None,
            },
        })
        .collect())
}

fn rp_revocations(
    app: &App,
    domain: &str,
    grant_id: &str,
    domain_keys: &[lk::DomainPublicKey],
) -> Result<Vec<String>, Error> {
    let rp = app.rp()?;
    let raw = rp.caller.call(
        "resolve-act-as-revocations",
        g::encode_rp_resolve_act_as_revocations_request(&lk::RpResolveActAsRevocationsRequest {
            subject_domain: domain.to_string(),
            grant_ids: vec![grant_id.to_string()],
        }),
    )?;
    let resp = g::decode_get_act_as_grant_revocations_response(&raw).map_err(|e| {
        Error::new(
            Code::Unavailable,
            format!("resolve-act-as-revocations: {e}"),
        )
    })?;
    Ok(aa::revoked_grant_ids(
        &resp.revocations,
        domain_keys,
        domain,
    ))
}

pub fn exchange(app: &App, ctx: &ReqCtx, req: ActAsExchangeRequest) -> Result<ScopedBearer, Error> {
    let credential = g::decode_act_as_credential(&req.credential_cbor)
        .map_err(|e| Error::invalid(format!("credential_cbor is not an ActAsCredential: {e}")))?;
    let grant =
        aa::decode_grant(&credential.grant).map_err(|e| Error::invalid(format!("grant: {e}")))?;
    let own = own_application(app)?;
    if !aa::same_application(&grant.audience, &own) {
        return Err(Error::forbidden("this credential is for another audience"));
    }
    let grantee_app = grant.grantee.application.clone().ok_or_else(|| {
        Error::new(
            Code::PreconditionFailed,
            "local-RP grantees are not accepted here",
        )
    })?;
    let signer_instance = aa::credential_signer_instance(&credential)
        .ok_or_else(|| Error::invalid("presentation names no application instance"))?
        .to_string();
    let scope_set_signer = aa::credential_scope_set_signer(&credential)
        .map_err(|e| Error::invalid(format!("scope set: {e}")))?;
    let mut conn = app.conn()?;
    let issuer_domain_keys = rp_domain_keys(app, &grant.subject_domain)?;
    let grantee_keys = rp_grantee_keys(app, &grantee_app, &signer_instance)?;
    let own_keys = own_key_refs(app, &mut conn, &scope_set_signer)?;
    let revoked = rp_revocations(
        app,
        &grant.subject_domain,
        &grant.grant_id,
        &issuer_domain_keys,
    )?;
    let digest = request_digest();
    let verified = aa::verify_credential(
        &credential,
        &aa::AudienceContext {
            own_application: &own,
            own_scope_set_keys: &own_keys,
            issuer_domain_keys: &issuer_domain_keys,
            grantee_instance_keys: &grantee_keys,
            expected_request_digest: &digest,
            revoked_grant_ids: &revoked,
            max_presentation_age_seconds: 300,
            now: Utc::now(),
            skew_seconds: 300,
            revoked_key_policy: aa::RevokedKeyPolicy::AcceptBeforeRevocation,
        },
    )
    .map_err(|e| Error::new(Code::Forbidden, format!("act-as credential refused: {e}")))?;
    // Replay guard: one presentation nonce, once.
    let nonce_hex = hex::encode(&verified.nonce);
    let inserted = diesel::insert_into(act_as_nonces::table)
        .values(act_as_nonces::nonce.eq(&nonce_hex))
        .on_conflict_do_nothing()
        .execute(&mut conn)?;
    if inserted == 0 {
        return Err(Error::forbidden("presentation replayed"));
    }
    // `VerifiedActAs.user_id` may carry `user_id@domain`; the UUID is the key.
    let bare_user = verified
        .user_id
        .split('@')
        .next()
        .unwrap_or(&verified.user_id);
    let user_id = uuid::Uuid::parse_str(bare_user)
        .map_err(|_| Error::invalid(format!("grant user id {bare_user} is not a UUID")))?;
    let identity: Identity = identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq(&verified.subject_domain))
        .select(Identity::as_select())
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| {
            Error::new(
                Code::PreconditionFailed,
                "the user has never signed in to this instance; one browser or CLI login is needed first",
            )
        })?;
    if identity.disabled_at.is_some() {
        return Err(Error::forbidden("identity disabled"));
    }
    let approved: Vec<String> = verified.approved_scope.clone();
    let scopes: Vec<String> = if req.requested_scopes.is_empty() {
        approved.clone()
    } else {
        let narrowed: Vec<String> = req
            .requested_scopes
            .iter()
            .filter(|s| approved.contains(s))
            .cloned()
            .collect();
        if narrowed.is_empty() {
            return Err(Error::forbidden(
                "none of the requested scopes were approved",
            ));
        }
        narrowed
    };
    let grant_expires = chrono::DateTime::parse_from_rfc3339(&verified.expires_at)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let expires_at = std::cmp::min(
        grant_expires,
        Utc::now() + Duration::seconds(MAX_BEARER_SECONDS),
    );
    let grantee_label = format!(
        "{}@{}/{}#{}",
        grantee_app.subject_user_id,
        grantee_app.subject_domain,
        grantee_app.application_id,
        signer_instance
    );
    let secret = tokens::new_secret(ACT_AS_PREFIX);
    conn.transaction::<(), Error, _>(|conn| {
        diesel::insert_into(api_tokens::table)
            .values(&NewApiToken {
                id: vcsstar_db::new_id(),
                identity_id: identity.id,
                token_hash: tokens::hash(&secret),
                name: "act-as",
                scopes: scopes.iter().cloned().map(Some).collect(),
                client_label: Some(&grantee_label),
                expires_at: Some(expires_at),
            })
            .execute(conn)?;
        diesel::insert_into(act_as_grants::table)
            .values((
                act_as_grants::grant_id.eq(&verified.grant_id),
                act_as_grants::identity_id.eq(identity.id),
                act_as_grants::grantee.eq(&grantee_label),
                act_as_grants::scopes.eq(approved.iter().cloned().map(Some).collect::<Vec<_>>()),
                act_as_grants::expires_at.eq(grant_expires),
                act_as_grants::last_presentation_nonce_at.eq(Utc::now()),
            ))
            .on_conflict(act_as_grants::grant_id)
            .do_update()
            .set((
                act_as_grants::scopes.eq(approved.iter().cloned().map(Some).collect::<Vec<_>>()),
                act_as_grants::expires_at.eq(grant_expires),
                act_as_grants::last_presentation_nonce_at.eq(Utc::now()),
            ))
            .execute(conn)?;
        Ok(())
    })?;
    let _ = ctx;
    let _ = now_ms();
    Ok(ScopedBearer {
        bearer: secret,
        scopes,
        expires_at: to_ms(expires_at),
        subject: identity_to_wire(&identity),
        acting_via: grantee_label,
    })
}

/// Prune replay nonces older than the window.
pub fn prune_nonces(conn: &mut PgConnection) -> Result<usize, Error> {
    Ok(diesel::delete(
        act_as_nonces::table.filter(act_as_nonces::seen_at.lt(Utc::now() - Duration::hours(1))),
    )
    .execute(conn)?)
}
