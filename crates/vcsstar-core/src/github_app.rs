//! GitHub App credentials for the mirror. The stored secret is the App id,
//! the installation id, and the App's RSA private key. From it vcsstar
//! mints a short App JWT (RS256, ten minutes) and exchanges it for an
//! installation access token (one hour), which then serves as the password
//! for `git push` and as the bearer for the REST API, exactly like a PAT.
//! Tokens are cached per credential until five minutes before expiry.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use serde::Deserialize;
use uuid::Uuid;

use crate::github::Failure;
use crate::{App, Error};

/// The secret value of a `github_app` credential.
#[derive(Debug, Clone, Deserialize)]
pub struct AppCredential {
    pub app_id: String,
    pub installation_id: String,
    pub private_key: String,
}

impl AppCredential {
    /// Parse and check the secret. The key must load, so a bad paste fails
    /// at `set-credential` and not inside a worker.
    pub fn parse(secret: &str) -> Result<Self, Error> {
        let c: AppCredential = serde_json::from_str(secret.trim()).map_err(|_| {
            Error::invalid(
                "a github_app secret is a JSON object with app_id, installation_id, and private_key",
            )
        })?;
        if c.app_id.trim().is_empty() || !c.app_id.trim().chars().all(|ch| ch.is_ascii_digit()) {
            return Err(Error::invalid("app_id must be the numeric GitHub App id"));
        }
        if c.installation_id.trim().is_empty()
            || !c
                .installation_id
                .trim()
                .chars()
                .all(|ch| ch.is_ascii_digit())
        {
            return Err(Error::invalid(
                "installation_id must be the numeric installation id",
            ));
        }
        c.key()?;
        Ok(c)
    }

    fn key(&self) -> Result<RsaPrivateKey, Error> {
        let pem = self.private_key.trim();
        RsaPrivateKey::from_pkcs1_pem(pem)
            .or_else(|_| RsaPrivateKey::from_pkcs8_pem(pem))
            .map_err(|_| Error::invalid("private_key is not a PEM RSA private key"))
    }

    /// The App JWT GitHub expects: `iss` is the App id, `iat` a minute in the
    /// past for clock skew, `exp` at most ten minutes ahead.
    pub fn jwt_at(&self, now: DateTime<Utc>) -> Result<String, Error> {
        let key = self.key()?;
        let header = serde_json::json!({ "alg": "RS256", "typ": "JWT" });
        let claims = serde_json::json!({
            "iat": now.timestamp() - 60,
            "exp": now.timestamp() + 9 * 60,
            "iss": self.app_id.trim(),
        });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let signing_input = format!(
            "{}.{}",
            b64.encode(header.to_string()),
            b64.encode(claims.to_string())
        );
        let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(key);
        let signature = signer.sign(signing_input.as_bytes());
        Ok(format!(
            "{signing_input}.{}",
            b64.encode(signature.to_bytes())
        ))
    }

    pub fn jwt(&self) -> Result<String, Error> {
        self.jwt_at(Utc::now())
    }
}

/// An App JWT for the instance's own App (no installation needed).
pub fn sign_jwt(app_id: &str, private_key: &str) -> Result<String, Error> {
    AppCredential {
        app_id: app_id.to_string(),
        installation_id: "0".into(),
        private_key: private_key.to_string(),
    }
    .jwt()
}

/// How long a "not installed" answer is believed before GitHub is asked
/// again.
const NOT_INSTALLED_TTL_MINUTES: i64 = 2;

/// Installation tokens in hand, keyed by credential id (or, for the
/// instance App, by a key derived from the repository name), plus the
/// repositories the instance App turned out not to be installed for.
#[derive(Default)]
pub struct TokenCache {
    tokens: Mutex<HashMap<Uuid, (String, DateTime<Utc>)>>,
    not_installed: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl TokenCache {
    pub fn not_installed(&self, full_name: &str) -> bool {
        let map = self.not_installed.lock().unwrap_or_else(|p| p.into_inner());
        map.get(full_name).is_some_and(|until| *until > Utc::now())
    }

    pub fn remember_not_installed(&self, full_name: &str) {
        let mut map = self.not_installed.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(
            full_name.to_string(),
            Utc::now() + Duration::minutes(NOT_INSTALLED_TTL_MINUTES),
        );
    }

    pub fn forget_not_installed(&self, full_name: &str) {
        let mut map = self.not_installed.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(full_name);
    }

    pub(crate) fn get(&self, id: Uuid) -> Option<String> {
        let map = self.tokens.lock().unwrap_or_else(|p| p.into_inner());
        map.get(&id)
            .filter(|(_, exp)| *exp - Duration::minutes(5) > Utc::now())
            .map(|(t, _)| t.clone())
    }

    pub(crate) fn put(&self, id: Uuid, token: String, expires_at: DateTime<Utc>) {
        let mut map = self.tokens.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(id, (token, expires_at));
    }

    /// Drop a token the API refused so the next call mints a fresh one.
    pub fn forget(&self, id: Uuid) {
        let mut map = self.tokens.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&id);
    }
}

/// An installation access token for credential `id`, minted from `secret`
/// when none is cached. Failures are classified like other GitHub calls:
/// a refused JWT or an unknown installation is terminal (the admin must
/// fix the credential), the rest is transient.
pub fn installation_token(app: &App, id: Uuid, secret: &str) -> Result<String, Failure> {
    if let Some(t) = app.github_app_tokens.get(id) {
        return Ok(t);
    }
    let cred = AppCredential::parse(secret).map_err(|e| Failure::Terminal(e.message))?;
    let jwt = cred.jwt().map_err(|e| Failure::Terminal(e.message))?;
    let (token, expires_at) = crate::github::installation_token(
        &app.config.github_api_base,
        &jwt,
        cred.installation_id.trim(),
    )?;
    app.github_app_tokens.put(id, token.clone(), expires_at);
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::VerifyingKey;
    use rsa::signature::Verifier;

    /// A throwaway 2048-bit key generated for this test; it guards nothing.
    const TEST_KEY: &str = include_str!("../tests/fixtures/github-app-test-key.pem");

    fn secret(key: &str) -> String {
        serde_json::json!({
            "app_id": "123456",
            "installation_id": "7890123",
            "private_key": key,
        })
        .to_string()
    }

    #[test]
    fn parses_and_signs_a_verifiable_jwt() {
        let cred = AppCredential::parse(&secret(TEST_KEY)).expect("parse");
        let now = Utc::now();
        let jwt = cred.jwt_at(now).expect("jwt");
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header: serde_json::Value =
            serde_json::from_slice(&b64.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "RS256");
        let claims: serde_json::Value =
            serde_json::from_slice(&b64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["iss"], "123456");
        assert_eq!(claims["iat"], now.timestamp() - 60);
        assert_eq!(claims["exp"], now.timestamp() + 540);

        let public = RsaPrivateKey::from_pkcs1_pem(TEST_KEY)
            .unwrap()
            .to_public_key();
        let verifier = VerifyingKey::<sha2::Sha256>::new(public);
        let sig = rsa::pkcs1v15::Signature::try_from(b64.decode(parts[2]).unwrap().as_slice())
            .expect("signature bytes");
        verifier
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .expect("signature verifies with the App's public key");
    }

    #[test]
    fn rejects_bad_secrets() {
        assert!(AppCredential::parse("ghp_notjson").is_err());
        assert!(AppCredential::parse(&secret(
            "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----"
        ))
        .is_err());
        let mut j: serde_json::Value = serde_json::from_str(&secret(TEST_KEY)).unwrap();
        j["app_id"] = "my-app".into();
        assert!(AppCredential::parse(&j.to_string()).is_err());
    }

    #[test]
    fn cache_serves_until_five_minutes_before_expiry() {
        let c = TokenCache::default();
        let id = Uuid::now_v7();
        assert!(c.get(id).is_none());
        c.put(id, "t1".into(), Utc::now() + Duration::minutes(4));
        assert!(c.get(id).is_none(), "too close to expiry");
        c.put(id, "t2".into(), Utc::now() + Duration::minutes(50));
        assert_eq!(c.get(id).as_deref(), Some("t2"));
        c.forget(id);
        assert!(c.get(id).is_none());
    }
}
