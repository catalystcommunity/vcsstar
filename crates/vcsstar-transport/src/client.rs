//! Client-side carriers implementing the generated `vcsstar_types::Transport`.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

use csilgen_transport::carrier::StreamCarrier;
use csilgen_transport::rpc::{RpcClient, RpcRequest, RpcResponse};
use vcsstar_types::{ClientError, Transport};

use crate::MAX_FRAME;

/// The HTTP path every vcsstar instance serves the envelope-in-body carrier on.
pub const RPC_PATH: &str = "/csil/v1/rpc";
pub const CBOR_CONTENT_TYPE: &str = "application/cbor";

/// POST envelope-in-body over HTTP(S). One request per call; no connection state.
pub struct HttpTransport {
    url: String,
    auth: Option<String>,
    agent: ureq::Agent,
}

impl HttpTransport {
    /// `base_url` is the instance origin, e.g. `https://vcsstar.catalystsquad.com`.
    pub fn new(base_url: &str, auth: Option<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build();
        Self {
            url: format!("{}{}", base_url.trim_end_matches('/'), RPC_PATH),
            auth,
            agent,
        }
    }

    pub fn set_auth(&mut self, auth: Option<String>) {
        self.auth = auth;
    }

    /// Send one raw envelope and return the decoded response. The generic CLI
    /// layer uses this; typed clients go through `Transport::call`.
    pub fn call_envelope(
        &self,
        service: &str,
        op: &str,
        payload: Vec<u8>,
    ) -> Result<RpcResponse, ClientError> {
        let mut request = RpcRequest::new(service, op, payload).with_id(1);
        if let Some(auth) = &self.auth {
            request = request.with_auth(auth.clone());
        }
        let body = request
            .encode()
            .map_err(|e| ClientError::Transport(format!("encode envelope: {e}")))?;
        let response = self
            .agent
            .post(&self.url)
            .set("Content-Type", CBOR_CONTENT_TYPE)
            .set("Accept", CBOR_CONTENT_TYPE)
            .send_bytes(&body);
        let response = match response {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(ClientError::Transport(format!("http {code}: {text}")));
            }
            Err(e) => return Err(ClientError::Transport(format!("http: {e}"))),
        };
        let mut bytes = Vec::new();
        let reader = response.into_reader();
        Read::take(reader, MAX_FRAME as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| ClientError::Transport(format!("read: {e}")))?;
        if bytes.len() > MAX_FRAME {
            return Err(ClientError::Transport("response too large".into()));
        }
        RpcResponse::decode(&bytes)
            .map_err(|e| ClientError::Transport(format!("bad response envelope: {e}")))
    }
}

/// Map a decoded envelope to the generated client's expectations: a status-0
/// reply with the `ServiceError` variant becomes `ClientError::Service`.
pub fn response_to_payload(response: RpcResponse) -> Result<Vec<u8>, ClientError> {
    if response.status != csilgen_transport::conventions::Status::Ok {
        let message = response.error.clone().unwrap_or_default();
        return Err(ClientError::Transport(format!(
            "transport status {:?}: {message}",
            response.status
        )));
    }
    if response.variant.as_deref() == Some("ServiceError") {
        let err = vcsstar_types::decode_service_error(&response.payload)
            .map_err(|e| ClientError::Transport(format!("bad ServiceError payload: {e}")))?;
        return Err(ClientError::Service {
            code: err.code as i64,
            message: err.message,
        });
    }
    Ok(response.payload)
}

impl Transport for HttpTransport {
    fn call(&self, service: &str, op: &str, req: &[u8]) -> Result<Vec<u8>, ClientError> {
        response_to_payload(self.call_envelope(service, op, req.to_vec())?)
    }
}

/// Length-prefixed CSIL-RPC over a plain TCP stream. One connection, one call
/// at a time. TLS is added by the caller wrapping the stream (phase 2).
pub struct TcpTransport {
    client: Mutex<RpcClient<StreamCarrier<TcpStream>>>,
    auth: Option<String>,
}

impl TcpTransport {
    pub fn connect(addr: &str, auth: Option<String>) -> Result<Self, ClientError> {
        let sock = addr
            .parse::<std::net::SocketAddr>()
            .or_else(|_| {
                std::net::ToSocketAddrs::to_socket_addrs(addr)
                    .map_err(|e| e.to_string())
                    .and_then(|mut it| it.next().ok_or_else(|| "no address".to_string()))
            })
            .map_err(|e| ClientError::Transport(format!("resolve {addr}: {e}")))?;
        let stream = TcpStream::connect_timeout(&sock, Duration::from_secs(10))
            .map_err(|e| ClientError::Transport(format!("connect {addr}: {e}")))?;
        stream.set_read_timeout(Some(Duration::from_secs(120))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
        let carrier = StreamCarrier::with_max_frame(stream, MAX_FRAME)
            .map_err(|e| ClientError::Transport(format!("carrier: {e}")))?;
        Ok(Self {
            client: Mutex::new(RpcClient::new(carrier, false)),
            auth,
        })
    }

    pub fn call_envelope(
        &self,
        service: &str,
        op: &str,
        payload: Vec<u8>,
    ) -> Result<RpcResponse, ClientError> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| ClientError::Transport("poisoned".into()))?;
        client
            .call(service, op, payload, self.auth.clone())
            .map_err(|e| ClientError::Transport(format!("tcp: {e}")))
    }
}

impl Transport for TcpTransport {
    fn call(&self, service: &str, op: &str, req: &[u8]) -> Result<Vec<u8>, ClientError> {
        response_to_payload(self.call_envelope(service, op, req.to_vec())?)
    }
}
