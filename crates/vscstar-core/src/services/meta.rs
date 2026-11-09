use vscstar_types::{Empty, ServerInfo};

use crate::principal::ReqCtx;
use crate::{App, Error};

pub const CONTRACT_VERSION: &str = "v1alpha1";
pub const DESCRIPTOR: &[u8] = include_bytes!("../../../vscstar-types/vscstar.csil-schema.cbor");

/// The SHA-256 body digest stored inside the descriptor (32 bytes).
pub fn contract_digest() -> Vec<u8> {
    // The descriptor is a CBOR map; the `digest` key holds a 32-byte string.
    // Scan for the key rather than pull in a CBOR library for one field.
    let key = b"\x66digest"; // text(6) "digest"
    if let Some(pos) = DESCRIPTOR.windows(key.len()).position(|w| w == key) {
        let start = pos + key.len();
        // bytes(32) is encoded as 0x58 0x20 followed by 32 bytes.
        if DESCRIPTOR.len() >= start + 34
            && DESCRIPTOR[start] == 0x58
            && DESCRIPTOR[start + 1] == 0x20
        {
            return DESCRIPTOR[start + 2..start + 34].to_vec();
        }
    }
    Vec::new()
}

pub fn server_info(app: &App, _ctx: &ReqCtx, _req: Empty) -> Result<ServerInfo, Error> {
    Ok(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        contract_digest: contract_digest(),
        features: vec!["rpc-http".into(), "rpc-tcp".into()],
        instance_domain: app.config.instance_domain.clone(),
        public_url: app.config.public_url.clone(),
        instance_identity: app.config.instance_identity.clone(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn digest_is_32_bytes() {
        assert_eq!(super::contract_digest().len(), 32);
    }
}
