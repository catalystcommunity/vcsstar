//! Bearer secrets: random 256-bit values, stored as SHA-256 digests.

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Prefix that lets logs and people recognize a vcsstar API token without
/// revealing it. The prefix is not part of the hashed material.
pub const API_TOKEN_PREFIX: &str = "vsk_";
pub const SESSION_PREFIX: &str = "vss_";
/// Short bearer issued to an act-as grantee.
pub const ACT_AS_PREFIX: &str = "vsa_";

pub fn new_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

pub fn hash(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_unique_and_prefixed() {
        let a = new_secret(API_TOKEN_PREFIX);
        let b = new_secret(API_TOKEN_PREFIX);
        assert_ne!(a, b);
        assert!(a.starts_with("vsk_"));
        assert_eq!(hash(&a).len(), 32);
    }
}
