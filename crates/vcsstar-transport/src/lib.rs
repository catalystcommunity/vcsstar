//! Carriers for the vcsstar CSIL-RPC contract.
//!
//! csilgen generates shapes and routing only. This crate supplies the wire:
//! one shared envelope handler used by the HTTP route and the TCP listener,
//! and client-side carriers that implement the generated `Transport` trait.

pub mod client;
pub mod server;

pub use csilgen_transport::conventions::Status;
pub use csilgen_transport::rpc::{HandlerOutcome, RpcRequest, RpcResponse};

/// Largest envelope accepted on any carrier. Git data never travels in an
/// envelope, so 4 MiB covers every contract payload with room to spare.
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// Carrier-level facts about the caller that the dispatcher needs.
#[derive(Debug, Clone, Default)]
pub struct Peer {
    /// Credential presented on the carrier: envelope `auth`, an
    /// `Authorization: Bearer` header, or a session cookie value.
    pub credential: Option<String>,
    /// How the credential arrived; cookies need same-origin checks.
    pub credential_source: CredentialSource,
    pub remote: String,
    pub carrier: Carrier,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CredentialSource {
    #[default]
    None,
    Envelope,
    Header,
    Cookie,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Carrier {
    #[default]
    Http,
    Tcp,
}

/// The application's dispatch seam. One implementation serves every carrier.
pub trait Dispatcher: Send + Sync + 'static {
    fn dispatch(&self, request: &RpcRequest, peer: &Peer) -> HandlerOutcome;
}

impl<F> Dispatcher for F
where
    F: Fn(&RpcRequest, &Peer) -> HandlerOutcome + Send + Sync + 'static,
{
    fn dispatch(&self, request: &RpcRequest, peer: &Peer) -> HandlerOutcome {
        self(request, peer)
    }
}
