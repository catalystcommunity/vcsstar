//! Server-side carriers: the envelope handler shared by HTTP and TCP, and a
//! thread-per-connection TCP listener.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use csilgen_transport::carrier::StreamCarrier;
use csilgen_transport::rpc::{HandlerOutcome, RpcRequest, RpcResponse, RpcServer};

use crate::{Carrier, CredentialSource, Dispatcher, Peer, Status, MAX_FRAME};

/// Decode one envelope, dispatch it, and encode the reply. This is the whole
/// HTTP carrier: the route hands in the body bytes and writes back the result.
/// The reply is always an envelope, so the HTTP status is always 200.
pub fn handle_envelope(dispatcher: &dyn Dispatcher, body: &[u8], mut peer: Peer) -> Vec<u8> {
    if body.len() > MAX_FRAME {
        return encode_response(RpcResponse::transport_error(
            Status::MalformedEnvelope,
            "envelope too large",
        ));
    }
    let mut request = match RpcRequest::decode(body) {
        Ok(r) => r,
        Err(e) => {
            return encode_response(RpcResponse::transport_error(
                Status::MalformedEnvelope,
                format!("bad envelope: {e}"),
            ))
        }
    };
    // Envelope auth wins over carrier auth when both are present.
    if let Some(auth) = request.auth.take() {
        peer.credential = Some(auth);
        peer.credential_source = CredentialSource::Envelope;
    }
    let id = request.id;
    let response = match dispatcher.dispatch(&request, &peer) {
        HandlerOutcome::Reply { variant, payload } => RpcResponse::ok(variant, payload),
        HandlerOutcome::Transport(status, message) => {
            RpcResponse::transport_error(status, &message)
        }
    };
    encode_response(response.with_id(id))
}

/// Encode a response envelope. Encoding a well-formed envelope cannot fail;
/// if it does, answer with a minimal transport error so the caller still gets
/// an envelope rather than a closed connection.
pub fn encode_response(response: RpcResponse) -> Vec<u8> {
    match response.encode() {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(error = %e, "failed to encode response envelope");
            RpcResponse::transport_error(Status::Internal, "response encoding failed")
                .encode()
                .unwrap_or_default()
        }
    }
}

/// Serve CSIL-RPC on a plain TCP listener until `shutdown` is set. Each
/// connection gets a thread; the dispatcher is shared.
pub fn serve_tcp(
    listener: TcpListener,
    dispatcher: Arc<dyn Dispatcher>,
    shutdown: Arc<AtomicBool>,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let dispatcher = Arc::clone(&dispatcher);
                let shutdown = Arc::clone(&shutdown);
                std::thread::Builder::new()
                    .name(format!("rpc-tcp-{addr}"))
                    .spawn(move || {
                        if let Err(e) =
                            serve_connection(stream, addr.to_string(), dispatcher, shutdown)
                        {
                            tracing::debug!(peer = %addr, error = %e, "tcp connection ended");
                        }
                    })?;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn serve_connection(
    stream: TcpStream,
    remote: String,
    dispatcher: Arc<dyn Dispatcher>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let carrier = StreamCarrier::with_max_frame(stream, MAX_FRAME)?;
    let mut server = RpcServer::new(carrier);
    let peer_base = Peer {
        remote,
        carrier: Carrier::Tcp,
        ..Peer::default()
    };
    let mut handler = |request: &RpcRequest| -> HandlerOutcome {
        let mut peer = peer_base.clone();
        if let Some(auth) = request.auth.clone() {
            peer.credential = Some(auth);
            peer.credential_source = CredentialSource::Envelope;
        }
        dispatcher.dispatch(request, &peer)
    };
    while !shutdown.load(Ordering::SeqCst) {
        match server.serve_one(&mut handler) {
            Ok(true) => continue,
            Ok(false) => break,
            Err(e) => return Err(Box::new(e)),
        }
    }
    Ok(())
}
