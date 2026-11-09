//! Client for the domain linkkeys server's `Rp` service over CSIL-RPC on
//! TCP with pinned TLS. vscstar holds no RP keys; every RP operation is
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
    fn call(&self, op: &str, payload: Vec<u8>) -> Result<Vec<u8>, Error>;
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
                "LinkKeys login is not configured (VSCSTAR__LINKKEYS_RP_ADDR)",
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
        let host = linkkeys_rpc_client::extract_hostname(&self.address).to_string();
        let name = rustls::pki_types::ServerName::try_from(host)
            .map_err(|e| Error::new(Code::Internal, format!("rp server name: {e}")))?;
        let conn = rustls::ClientConnection::new(Arc::clone(&self.tls), name)
            .map_err(|e| Error::new(Code::Internal, format!("rp tls: {e}")))?;
        let stream = rustls::StreamOwned::new(conn, self.connect()?);
        let carrier = StreamCarrier::with_max_frame(Pinned(stream), MAX_FRAME)
            .map_err(|e| Error::new(Code::Internal, format!("rp carrier: {e}")))?;
        let mut client = RpcClient::new(carrier, false);
        let reply = client
            .call("Rp", op, payload, Some(self.api_key.clone()))
            .map_err(|e| Error::new(Code::Unavailable, format!("rp {op}: {e}")))?;
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

/// Claims vscstar asks for. `handle` is REQUIRED; a login without it fails.
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
}

/// Checks vscstar performs itself after the RP verified the assertion.
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

/// Resolve the `https_base` of `_linkkeys_apis.<domain>`. Runs the async
/// resolver on a private current-thread runtime so callers stay synchronous.
fn resolve_apis_https(domain: &str) -> Option<String> {
    let name = format!("{}.", liblinkkeys::dns::linkkeys_apis_dns_name(domain));
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
