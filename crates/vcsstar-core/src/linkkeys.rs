//! Client for the domain linkkeys server's `Rp` service over CSIL-RPC on
//! TCP with pinned TLS. vcsstar holds no RP keys; every RP operation is
//! delegated here. Do not use `linkkeys_rpc_client::send_request`: it
//! refuses private addresses, and the RP server is an in-cluster Service.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use csilgen_transport::carrier::StreamCarrier;
use csilgen_transport::rpc::RpcClient;
use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{
    decode_rp_decrypt_response, decode_rp_sign_response, decode_rp_verify_response,
    decode_user_info, encode_rp_decrypt_request, encode_rp_sign_request,
    encode_rp_user_info_request, encode_rp_verify_request,
};

use crate::config::LinkKeysConfig;
use crate::{Code, Error};

const MAX_FRAME: usize = 1024 * 1024;

/// The seam between the login flow and the network, so tests can fake the RP.
pub trait RpCaller: Send + Sync {
    /// Call an `Rp` operation with the API key.
    fn call(&self, op: &str, payload: Vec<u8>) -> Result<Vec<u8>, Error>;
    /// Call any service on the home domain server; `with_api_key` adds the
    /// account API key (needed for `Account/*`, not for public ops).
    fn call_service(
        &self,
        service: &str,
        op: &str,
        payload: Vec<u8>,
        with_api_key: bool,
    ) -> Result<Vec<u8>, Error> {
        let _ = (service, op, payload, with_api_key);
        Err(Error::new(
            Code::Unavailable,
            "this RP caller supports Rp operations only",
        ))
    }
}

/// Real caller: one TLS connection per call, pinned to the RP server's
/// raw-Ed25519-SPKI fingerprints, API key in the envelope `auth` field.
pub struct TcpRpCaller {
    address: String,
    fingerprints: Vec<String>,
    api_key: String,
    tls: Arc<rustls::ClientConfig>,
}

impl TcpRpCaller {
    pub fn from_config(cfg: &LinkKeysConfig) -> Result<Self, Error> {
        let address = cfg.rp_addr.clone().ok_or_else(|| {
            Error::new(
                Code::Unavailable,
                "LinkKeys login is not configured (VCSSTAR__LINKKEYS_RP_ADDR)",
            )
        })?;
        let api_key = cfg
            .api_key
            .clone()
            .ok_or_else(|| Error::new(Code::Unavailable, "LinkKeys API key is not configured"))?;
        if cfg.rp_fingerprints.is_empty() {
            return Err(Error::new(
                Code::Unavailable,
                "LinkKeys RP fingerprints are not configured",
            ));
        }
        let tls = linkkeys_rpc_client::tls::client_config(cfg.rp_fingerprints.clone(), None)
            .map_err(|e| Error::new(Code::Internal, format!("rp tls config: {e}")))?;
        Ok(Self {
            address,
            fingerprints: cfg.rp_fingerprints.clone(),
            api_key,
            tls,
        })
    }

    fn connect(&self) -> Result<TcpStream, Error> {
        let addr = self
            .address
            .to_socket_addrs()
            .map_err(|e| {
                Error::new(
                    Code::Unavailable,
                    format!("resolve rp {}: {e}", self.address),
                )
            })?
            .next()
            .ok_or_else(|| Error::new(Code::Unavailable, "rp address resolves to nothing"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
            .map_err(|e| Error::new(Code::Unavailable, format!("connect rp: {e}")))?;
        stream.set_read_timeout(Some(Duration::from_secs(20))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
        Ok(stream)
    }
}

/// `StreamOwned` implements Read and Write; the newtype keeps the carrier's
/// type parameter local to this module.
struct Pinned(rustls::StreamOwned<rustls::ClientConnection, TcpStream>);

impl Read for Pinned {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl Write for Pinned {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl RpCaller for TcpRpCaller {
    fn call(&self, op: &str, payload: Vec<u8>) -> Result<Vec<u8>, Error> {
        self.call_service("Rp", op, payload, true)
    }

    fn call_service(
        &self,
        service: &str,
        op: &str,
        payload: Vec<u8>,
        with_api_key: bool,
    ) -> Result<Vec<u8>, Error> {
        let host = linkkeys_rpc_client::extract_hostname(&self.address).to_string();
        let name = rustls::pki_types::ServerName::try_from(host)
            .map_err(|e| Error::new(Code::Internal, format!("rp server name: {e}")))?;
        let conn = rustls::ClientConnection::new(Arc::clone(&self.tls), name)
            .map_err(|e| Error::new(Code::Internal, format!("rp tls: {e}")))?;
        let stream = rustls::StreamOwned::new(conn, self.connect()?);
        let carrier = StreamCarrier::with_max_frame(Pinned(stream), MAX_FRAME)
            .map_err(|e| Error::new(Code::Internal, format!("rp carrier: {e}")))?;
        let mut client = RpcClient::new(carrier, false);
        let auth = with_api_key.then(|| self.api_key.clone());
        let reply = client
            .call(service, op, payload, auth)
            .map_err(|e| Error::new(Code::Unavailable, format!("{service}/{op}: {e}")))?;
        if reply.status != csilgen_transport::conventions::Status::Ok {
            return Err(Error::new(
                Code::Unavailable,
                format!(
                    "rp {op}: transport status {:?}: {}",
                    reply.status,
                    reply.error.unwrap_or_default()
                ),
            ));
        }
        if reply.variant.as_deref() == Some("ServiceError")
            || reply
                .variant
                .as_deref()
                .is_some_and(|v| v.ends_with("Error"))
        {
            return Err(Error::new(
                Code::Unavailable,
                format!("rp {op} failed ({})", reply.variant.unwrap_or_default()),
            ));
        }
        let _ = &self.fingerprints;
        Ok(reply.payload)
    }
}

/// Claims vcsstar asks for. `handle` is REQUIRED; a login without it fails.
pub fn requested_claims() -> lk::ClaimRequest {
    lk::ClaimRequest {
        required: vec![lk::RequestedClaim {
            claim_type: "handle".into(),
            datatype: "text".into(),
        }],
        optional: vec![
            lk::RequestedClaim {
                claim_type: "display_name".into(),
                datatype: "text".into(),
            },
            lk::RequestedClaim {
                claim_type: "email".into(),
                datatype: "text".into(),
            },
        ],
    }
}

pub struct Rp<'a> {
    pub caller: &'a dyn RpCaller,
}

impl Rp<'_> {
    pub fn sign_request(&self, callback_url: &str, nonce: &str) -> Result<String, Error> {
        let req = lk::RpSignRequest {
            callback_url: callback_url.to_string(),
            nonce: nonce.to_string(),
            requested_claims: Some(requested_claims()),
            authentication_requirements: None,
            flow_context: None,
        };
        let bytes = self
            .caller
            .call("sign-request", encode_rp_sign_request(&req))?;
        Ok(decode_rp_sign_response(&bytes)
            .map_err(|e| Error::internal(format!("sign-request decode: {e:?}")))?
            .signed_request)
    }

    pub fn decrypt_token(&self, encrypted_token: &str) -> Result<String, Error> {
        let req = lk::RpDecryptRequest {
            encrypted_token: encrypted_token.to_string(),
        };
        let bytes = self
            .caller
            .call("decrypt-token", encode_rp_decrypt_request(&req))?;
        Ok(decode_rp_decrypt_response(&bytes)
            .map_err(|e| Error::internal(format!("decrypt-token decode: {e:?}")))?
            .signed_assertion)
    }

    pub fn verify_assertion(
        &self,
        signed_assertion: &str,
        expected_domain: &str,
    ) -> Result<lk::RpVerifyResponse, Error> {
        let req = lk::RpVerifyRequest {
            signed_assertion: signed_assertion.to_string(),
            expected_domain: expected_domain.to_string(),
        };
        let bytes = self
            .caller
            .call("verify-assertion", encode_rp_verify_request(&req))?;
        decode_rp_verify_response(&bytes)
            .map_err(|e| Error::internal(format!("verify-assertion decode: {e:?}")))
    }

    pub fn userinfo_fetch(
        &self,
        signed_assertion: &str,
        api_base: &str,
        domain: &str,
    ) -> Result<lk::UserInfo, Error> {
        let req = lk::RpUserInfoRequest {
            token: signed_assertion.to_string(),
            api_base: api_base.to_string(),
            domain: domain.to_string(),
        };
        let bytes = self
            .caller
            .call("userinfo-fetch", encode_rp_user_info_request(&req))?;
        decode_user_info(&bytes).map_err(|e| Error::internal(format!("userinfo decode: {e:?}")))
    }

    /// The signing keys of `domain` as the RP resolved them. vcsstar verifies
    /// claim signatures against these itself. A `stale` result (the home
    /// domain was unreachable) is refused: it is never current trust.
    pub fn resolve_domain_keys(&self, domain: &str) -> Result<Vec<lk::DomainPublicKey>, Error> {
        use liblinkkeys::generated::{
            decode_rp_resolve_domain_keys_response, encode_rp_resolve_domain_keys_request,
        };
        let req = lk::RpResolveDomainKeysRequest {
            domain: domain.to_string(),
            max_cache_age_seconds: Some(300),
        };
        let bytes = self.caller.call(
            "resolve-domain-keys",
            encode_rp_resolve_domain_keys_request(&req),
        )?;
        let resp = decode_rp_resolve_domain_keys_response(&bytes)
            .map_err(|e| Error::internal(format!("resolve-domain-keys decode: {e:?}")))?;
        if resp.cache_status == "stale" {
            return Err(Error::new(
                Code::Unavailable,
                "home domain unreachable; its signing keys could not be refreshed",
            ));
        }
        Ok(resp.keys)
    }
}

/// Checks vcsstar performs itself after the RP verified the assertion.
pub fn check_assertion(
    a: &lk::IdentityAssertion,
    expected_domain: &str,
    rp_domain: &str,
    nonce: &str,
) -> Result<(), Error> {
    if !a.domain.eq_ignore_ascii_case(expected_domain) {
        return Err(Error::new(
            Code::Unauthenticated,
            "assertion domain does not match the login request",
        ));
    }
    if !a.audience.eq_ignore_ascii_case(rp_domain.trim()) {
        return Err(Error::new(
            Code::Unauthenticated,
            "assertion audience is not this RP",
        ));
    }
    if a.nonce != nonce {
        return Err(Error::new(
            Code::Unauthenticated,
            "assertion nonce does not match",
        ));
    }
    let now = chrono::Utc::now();
    let issued = chrono::DateTime::parse_from_rfc3339(&a.issued_at)
        .map_err(|_| Error::new(Code::Unauthenticated, "assertion issued_at is not RFC 3339"))?;
    let expires = chrono::DateTime::parse_from_rfc3339(&a.expires_at).map_err(|_| {
        Error::new(
            Code::Unauthenticated,
            "assertion expires_at is not RFC 3339",
        )
    })?;
    if issued > now + chrono::Duration::minutes(5) {
        return Err(Error::new(
            Code::Unauthenticated,
            "assertion is issued in the future",
        ));
    }
    if expires < now || expires < issued {
        return Err(Error::new(Code::Unauthenticated, "assertion has expired"));
    }
    Ok(())
}

/// Browser endpoints of a LinkKeys domain, from `_linkkeys_apis.<domain>`.
/// `https=` carries no scheme and may carry a path. Falls back to the domain.
pub fn browser_base(domain: &str) -> String {
    match resolve_apis_https(domain) {
        Some(base) => base.trim_end_matches('/').to_string(),
        None => format!("https://{domain}"),
    }
}

/// Resolve the `https_base` of `_linkkeys_apis.<domain>`. The async
/// resolver runs on its own thread with a private runtime, so this stays
/// synchronous and is safe to call from inside Rocket's runtime (a nested
/// `block_on` on a tokio worker panics).
fn resolve_apis_https(domain: &str) -> Option<String> {
    let name = format!("{}.", liblinkkeys::dns::linkkeys_apis_dns_name(domain));
    std::thread::Builder::new()
        .name("dns-txt".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            rt.block_on(async move {
                let resolver = hickory_resolver::TokioResolver::builder_tokio()
                    .ok()?
                    .build()
                    .ok()?;
                let lookup = tokio::time::timeout(
                    Duration::from_secs(5),
                    resolver.lookup(name, hickory_resolver::proto::rr::RecordType::TXT),
                )
                .await
                .ok()?
                .ok()?;
                for record in lookup.answers() {
                    if let hickory_resolver::proto::rr::RData::TXT(txt) = &record.data {
                        let joined: String = txt
                            .txt_data
                            .iter()
                            .map(|b| String::from_utf8_lossy(b).into_owned())
                            .collect();
                        if let Ok(rec) = liblinkkeys::dns::parse_linkkeys_apis_txt(&joined) {
                            if let Some(https) = rec.https_base {
                                return Some(https);
                            }
                        }
                    }
                }
                None
            })
        })
        .ok()?
        .join()
        .ok()
        .flatten()
}

/// The IDP authorize URL the browser is sent to.
pub fn authorize_url(base: &str, signed_request: &str, username: &str) -> String {
    let base = base.trim_end_matches('/');
    format!(
        "{base}/auth/authorize?signed_request={}&username={}",
        urlencode(signed_request),
        urlencode(username)
    )
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// One attested application key as the RP resolved it.
pub struct ResolvedKey {
    pub public_key: Vec<u8>,
    pub attestation_expires_at: chrono::DateTime<chrono::Utc>,
}

/// Resolve one application key of a user through the RP, verifying the
/// attestation set against the home domain's keys (liblinkkeys does the
/// cryptography; vcsstar keeps no RP state).
pub fn resolve_application_key(
    app: &crate::App,
    subject_user_id: &str,
    subject_domain: &str,
    application_id: &str,
    instance_id: &str,
    key_id: &str,
) -> Result<ResolvedKey, Error> {
    use liblinkkeys::application_keys::{verify_application_key_set, InstanceRef, KeyStatus};
    use liblinkkeys::generated::{
        decode_rp_resolve_application_keys_response, encode_rp_resolve_application_keys_request,
    };
    let rp = app.rp()?;
    let req = lk::RpResolveApplicationKeysRequest {
        subject_user_id: subject_user_id.to_string(),
        subject_domain: subject_domain.to_string(),
        application_id: application_id.to_string(),
        instance_id: instance_id.to_string(),
        max_cache_age_seconds: Some(300),
    };
    let raw = rp.caller.call(
        "resolve-application-keys",
        encode_rp_resolve_application_keys_request(&req),
    )?;
    let resp = decode_rp_resolve_application_keys_response(&raw)
        .map_err(|e| Error::new(Code::Unavailable, format!("rp response: {e}")))?;
    if resp.cache_status == "stale" {
        return Err(Error::new(
            Code::Unavailable,
            "home domain unreachable; attestation could not be refreshed",
        ));
    }
    let set = verify_application_key_set(
        &resp.application_keys,
        &resp.application_key_revocations,
        &resp.home_domain_keys,
        &InstanceRef {
            subject_user_id,
            subject_domain,
            application_id,
            instance_id,
        },
        chrono::Utc::now(),
        300,
    );
    let key = set
        .keys
        .iter()
        .find(|k| k.attestation.key_id == key_id)
        .ok_or_else(|| {
            Error::new(
                Code::NotFound,
                "application key not attested at the home domain",
            )
        })?;
    match &key.status {
        KeyStatus::Usable => {}
        other => {
            return Err(Error::new(
                Code::PreconditionFailed,
                format!("application key is not usable: {other:?}"),
            ))
        }
    }
    if key.attestation.key_usage != "sign" || key.attestation.algorithm != "ed25519" {
        return Err(Error::new(
            Code::PreconditionFailed,
            "key is not an Ed25519 signing key",
        ));
    }
    let until = chrono::DateTime::parse_from_rfc3339(&key.attestation.attestation_expires_at)
        .map_err(|e| Error::internal(format!("attestation time: {e}")))?
        .with_timezone(&chrono::Utc);
    Ok(ResolvedKey {
        public_key: key.attestation.public_key.clone(),
        attestation_expires_at: until,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assertion(nonce: &str) -> lk::IdentityAssertion {
        let now = chrono::Utc::now();
        lk::IdentityAssertion {
            user_id: "u".into(),
            domain: "Example.com".into(),
            audience: "rp.example".into(),
            nonce: nonce.into(),
            issued_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::minutes(4)).to_rfc3339(),
            authorized_claims: vec![],
            display_name: None,
        }
    }

    #[test]
    fn assertion_checks() {
        let a = assertion("n1");
        assert!(check_assertion(&a, "example.com", "rp.example", "n1").is_ok());
        assert!(check_assertion(&a, "other.com", "rp.example", "n1").is_err());
        assert!(check_assertion(&a, "example.com", "wrong", "n1").is_err());
        assert!(check_assertion(&a, "example.com", "rp.example", "n2").is_err());
        let mut expired = assertion("n1");
        expired.expires_at = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        assert!(check_assertion(&expired, "example.com", "rp.example", "n1").is_err());
    }

    #[test]
    fn authorize_url_shape() {
        let u = authorize_url("https://idp.example/x/", "a b", "alice");
        assert_eq!(
            u,
            "https://idp.example/x/auth/authorize?signed_request=a%20b&username=alice"
        );
    }
}
