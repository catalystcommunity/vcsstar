//! Generated transport-agnostic service clients from CSIL specification

use super::codec::*;
use super::types::*;

/// Error from a generated client call: a structured error the service returned,
/// or a transport-level failure. The caller-supplied `Transport` decides how an
/// error response maps onto `Service`. `Encode` means the request has no valid
/// wire form, so the client did not call the transport.
#[derive(Debug, Clone)]
pub enum ClientError {
    Service { code: i64, message: String },
    Transport(String),
    Encode(String),
    ApplicationServiceError(Box<ServiceError>),
    Decode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Service { code, message } => write!(f, "service error {code}: {message}"),
            Self::Encode(msg) => write!(f, "invalid request: {msg}"),
            Self::ApplicationServiceError(e) => write!(f, "application error: {e:?}"),
            Self::Decode(msg) => write!(f, "invalid reply: {msg}"),
            ClientError::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A status-zero CSIL-RPC reply: `variant` names the declared output arm that
/// `payload` encodes. A transport returns it from `call_reply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireReply {
    pub variant: Option<String>,
    pub payload: Vec<u8>,
}

/// The caller-supplied byte carrier: it performs the call named by `(service, op)`
/// with the already-encoded request bytes and returns the response bytes, or an
/// error. The generated client owns (de)serialization via the codec; the carrier
/// only moves bytes, so it can be HTTP, a queue, or an in-process loop.
pub trait Transport {
    fn call(&self, service: &str, op: &str, req: &[u8]) -> Result<Vec<u8>, ClientError>;

    /// Perform a call whose operation declares several reply arms and return
    /// the status-zero reply `variant` with its payload, so the client can
    /// return the success arm or the declared error arm.
    fn call_reply(&self, service: &str, op: &str, req: &[u8]) -> Result<WireReply, ClientError>;
}
impl From<CsilCborError> for ClientError {
    fn from(error: CsilCborError) -> Self {
        Self::Decode(error.0)
    }
}

/// Resolve a status-zero RPC reply into its success payload or its declared error
/// arm. Generated clients call it with the reply from `Transport::call_reply`.
pub fn resolve_rpc_reply(
    service: &str,
    op: &str,
    variant: Option<&str>,
    payload: Vec<u8>,
) -> Result<Vec<u8>, ClientError> {
    match (service, op) {
        ("Meta", "server-info") => match variant {
            Some("ServerInfo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "begin-login") => match variant {
            Some("BeginLoginResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "complete-login") => match variant {
            Some("Session") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "create-token") => match variant {
            Some("ApiToken") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "revoke-token") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "whoami") => match variant {
            Some("WhoAmI") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "begin-device-login") => match variant {
            Some("DeviceCode") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "poll-device-login") => match variant {
            Some("DevicePollResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "exchange-act-as-credential") => match variant {
            Some("ScopedBearer") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "scope-set") => match variant {
            Some("SignedActAsScopeSet") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "approve-device-login") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "register-key") => match variant {
            Some("SshKeyInfo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "list-keys") => match variant {
            Some("ListKeysResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "revoke-key") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "list-tokens") => match variant {
            Some("ListTokensResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "logout") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "github-link-status") => match variant {
            Some("GithubLink") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "set-github-token") => match variant {
            Some("GithubLink") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "unlink-github") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "get-preferences") => match variant {
            Some("Preferences") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "set-preferences") => match variant {
            Some("Preferences") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "local-login") => match variant {
            Some("Session") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "local-account") => match variant {
            Some("LocalAccount") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Auth", "change-local-password") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "create") => match variant {
            Some("Org") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "get") => match variant {
            Some("Org") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "list") => match variant {
            Some("OrgPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "update") => match variant {
            Some("Org") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "set-member") => match variant {
            Some("OrgMember") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "remove-member") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "list-members") => match variant {
            Some("OrgMemberPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "create-team") => match variant {
            Some("Team") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "delete-team") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "set-team-member") => match variant {
            Some("Team") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Orgs", "list-teams") => match variant {
            Some("ListTeamsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "create") => match variant {
            Some("Repo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "get") => match variant {
            Some("Repo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "list") => match variant {
            Some("RepoPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "update") => match variant {
            Some("Repo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "fork") => match variant {
            Some("Repo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "list-refs") => match variant {
            Some("ListRefsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "log") => match variant {
            Some("CommitPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "tree") => match variant {
            Some("TreeResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "blob") => match variant {
            Some("BlobResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "diff") => match variant {
            Some("DiffResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "delete") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "update-ref") => match variant {
            Some("RefInfo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "get-commit") => match variant {
            Some("Commit") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "set-grant") => match variant {
            Some("Grant") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "remove-grant") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "list-grants") => match variant {
            Some("ListGrantsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "set-protection") => match variant {
            Some("BranchProtection") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "remove-protection") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "list-protections") => match variant {
            Some("ListProtectionsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "capture-history") => match variant {
            Some("CaptureReport") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Repos", "list-forks") => match variant {
            Some("RepoPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "open") => match variant {
            Some("PullRequest") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "get") => match variant {
            Some("PullRequest") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "list") => match variant {
            Some("PrPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "review") => match variant {
            Some("Review") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "comment") => match variant {
            Some("Comment") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "merge") => match variant {
            Some("MergeResult") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "close") => match variant {
            Some("PullRequest") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "reopen") => match variant {
            Some("PullRequest") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "retarget") => match variant {
            Some("PullRequest") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "update") => match variant {
            Some("PullRequest") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "list-reviews") => match variant {
            Some("ListReviewsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "list-comments") => match variant {
            Some("ListCommentsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "edit-comment") => match variant {
            Some("Comment") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("PullRequests", "delete-comment") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Statuses", "set") => match variant {
            Some("CommitStatus") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Statuses", "list") => match variant {
            Some("ListStatusResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Hooks", "create-endpoint") => match variant {
            Some("Endpoint") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Hooks", "list-endpoints") => match variant {
            Some("ListEndpointsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Hooks", "delete-endpoint") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Hooks", "list-deliveries") => match variant {
            Some("DeliveryPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Hooks", "redeliver") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Hooks", "update-endpoint") => match variant {
            Some("Endpoint") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "status") => match variant {
            Some("MirrorStatus") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-blocked") => match variant {
            Some("ListBlockedResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "replay-blocked") => match variant {
            Some("ReplayResult") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "set-credential") => match variant {
            Some("Credential") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "reconcile") => match variant {
            Some("ReconcileReport") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "pause") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "resume") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "import-repo") => match variant {
            Some("Repo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "configure") => match variant {
            Some("MirrorStatus") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-credentials") => match variant {
            Some("ListCredentialsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "remove-credential") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-github-repos") => match variant {
            Some("GithubRepoPage") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "health") => match variant {
            Some("GithubHealth") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "backfill") => match variant {
            Some("MirrorStatus") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-github-installations") => match variant {
            Some("GithubInstallationList") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "import-all") => match variant {
            Some("ImportAllResult") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "set-schedule") => match variant {
            Some("MirrorStatus") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-connections") => match variant {
            Some("SyncConnections") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "connect") => match variant {
            Some("SyncConnection") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "update-connection") => match variant {
            Some("SyncConnection") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "sync-now") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "webhook-secret") => match variant {
            Some("SyncWebhookSecret") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "get-policy") => match variant {
            Some("SyncPolicy") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "set-policy") => match variant {
            Some("SyncPolicy") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-conflicts") => match variant {
            Some("SyncConflicts") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "resolve-conflict") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "link-provider") => match variant {
            Some("ProviderLink") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-provider-links") => match variant {
            Some("ProviderLinks") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "unlink-provider") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Mirror", "list-sync-records") => match variant {
            Some("SyncRecords") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Instance", "list-roles") => match variant {
            Some("InstanceRoleList") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Instance", "set-role") => match variant {
            Some("InstanceRoleGrant") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Instance", "remove-role") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Instance", "create-local-account") => match variant {
            Some("LocalAccount") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Instance", "reset-local-password") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "create") => match variant {
            Some("ServiceAccount") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "list") => match variant {
            Some("ListServiceAccountsResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "disable") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "enable") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "create-token") => match variant {
            Some("ApiToken") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "list-tokens") => match variant {
            Some("ListTokensResponse") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("ServiceAccounts", "revoke-token") => match variant {
            Some("Ok") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Federation", "handshake") => match variant {
            Some("FedHelloAck") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Federation", "verify-head") => match variant {
            Some("HeadInfo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Federation", "head-info") => match variant {
            Some("HeadInfo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        ("Federation", "repo-info") => match variant {
            Some("Repo") => Ok(payload),
            Some("ServiceError") => {
                let value = decode_service_error(&payload)?;
                let value = Box::new(value);
                let error = ClientError::ApplicationServiceError(value);
                Err(error)
            }
            _ => Err(ClientError::Decode(format!(
                "invalid reply variant {variant:?}"
            ))),
        },
        _ => Ok(payload),
    }
}

/// Typed client for the Meta service.
pub struct MetaClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> MetaClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// server-info (request/response).
    pub fn server_info(&self, req: Empty) -> Result<ServerInfo, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Meta", "server-info", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Meta",
            "server-info",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_server_info(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Auth service.
pub struct AuthClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> AuthClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// begin-login (request/response).
    pub fn begin_login(&self, req: BeginLoginRequest) -> Result<BeginLoginResponse, ClientError> {
        let csil_req = match encode_begin_login_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "begin-login", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "begin-login",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_begin_login_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// complete-login (request/response).
    pub fn complete_login(&self, req: CompleteLoginRequest) -> Result<Session, ClientError> {
        let csil_req = match encode_complete_login_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "complete-login", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "complete-login",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_session(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// create-token (request/response).
    pub fn create_token(&self, req: CreateTokenRequest) -> Result<ApiToken, ClientError> {
        let csil_req = match encode_create_token_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "create-token", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "create-token",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_api_token(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// revoke-token (request/response).
    pub fn revoke_token(&self, req: RevokeTokenRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_revoke_token_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "revoke-token", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "revoke-token",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// whoami (request/response).
    pub fn whoami(&self, req: Empty) -> Result<WhoAmI, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Auth", "whoami", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "whoami",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_who_am_i(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// begin-device-login (request/response).
    pub fn begin_device_login(&self, req: BeginDeviceRequest) -> Result<DeviceCode, ClientError> {
        let csil_req = match encode_begin_device_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "begin-device-login", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "begin-device-login",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_device_code(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// poll-device-login (request/response).
    pub fn poll_device_login(
        &self,
        req: DevicePollRequest,
    ) -> Result<DevicePollResponse, ClientError> {
        let csil_req = match encode_device_poll_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "poll-device-login", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "poll-device-login",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_device_poll_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// exchange-act-as-credential (request/response).
    pub fn exchange_act_as_credential(
        &self,
        req: ActAsExchangeRequest,
    ) -> Result<ScopedBearer, ClientError> {
        let csil_req = match encode_act_as_exchange_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply =
            self.transport
                .call_reply("Auth", "exchange-act-as-credential", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "exchange-act-as-credential",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_scoped_bearer(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// scope-set (request/response).
    pub fn scope_set(&self, req: ScopeSetRequest) -> Result<SignedActAsScopeSet, ClientError> {
        let csil_req = match encode_scope_set_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Auth", "scope-set", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "scope-set",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_signed_act_as_scope_set(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Browser side of the device flow. Needs a web session.
    pub fn approve_device_login(&self, req: DeviceApproveRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_device_approve_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "approve-device-login", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "approve-device-login",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// register-key (request/response).
    pub fn register_key(&self, req: RegisterKeyRequest) -> Result<SshKeyInfo, ClientError> {
        let csil_req = match encode_register_key_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "register-key", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "register-key",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ssh_key_info(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-keys (request/response).
    pub fn list_keys(&self, req: Empty) -> Result<ListKeysResponse, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Auth", "list-keys", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "list-keys",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_keys_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// revoke-key (request/response).
    pub fn revoke_key(&self, req: KeyRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_key_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Auth", "revoke-key", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "revoke-key",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-tokens (request/response).
    pub fn list_tokens(&self, req: Empty) -> Result<ListTokensResponse, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "list-tokens", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "list-tokens",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_tokens_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// logout (request/response).
    pub fn logout(&self, req: Empty) -> Result<Ok, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Auth", "logout", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "logout",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// GitHub account link of the caller (see GithubLink).
    pub fn github_link_status(&self, req: Empty) -> Result<GithubLink, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "github-link-status", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "github-link-status",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_github_link(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-github-token (request/response).
    pub fn set_github_token(&self, req: SetGithubTokenRequest) -> Result<GithubLink, ClientError> {
        let csil_req = match encode_set_github_token_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "set-github-token", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "set-github-token",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_github_link(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// unlink-github (request/response).
    pub fn unlink_github(&self, req: Empty) -> Result<Ok, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "unlink-github", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "unlink-github",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// The caller's display preferences (defaults when none are saved).
    pub fn get_preferences(&self, req: Empty) -> Result<Preferences, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "get-preferences", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "get-preferences",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_preferences(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Save the given fields; absent fields keep their value.
    pub fn set_preferences(&self, req: SetPreferencesRequest) -> Result<Preferences, ClientError> {
        let csil_req = match encode_set_preferences_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "set-preferences", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "set-preferences",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_preferences(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// local-login (request/response).
    pub fn local_login(&self, req: LocalLoginRequest) -> Result<Session, ClientError> {
        let csil_req = match encode_local_login_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "local-login", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "local-login",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_session(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// local-account (request/response).
    pub fn local_account(&self, req: Empty) -> Result<LocalAccount, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "local-account", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "local-account",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_local_account(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// change-local-password (request/response).
    pub fn change_local_password(
        &self,
        req: ChangeLocalPasswordRequest,
    ) -> Result<Ok, ClientError> {
        let csil_req = match encode_change_local_password_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Auth", "change-local-password", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Auth",
            "change-local-password",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Orgs service.
pub struct OrgsClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> OrgsClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create (request/response).
    pub fn create(&self, req: CreateOrgRequest) -> Result<Org, ClientError> {
        let csil_req = match encode_create_org_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Orgs", "create", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "create",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_org(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// get (request/response).
    pub fn get(&self, req: GetOrgRequest) -> Result<Org, ClientError> {
        let csil_req = match encode_get_org_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Orgs", "get", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "get",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_org(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list (request/response).
    pub fn list(&self, req: ListOrgsRequest) -> Result<OrgPage, ClientError> {
        let csil_req = match encode_list_orgs_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Orgs", "list", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "list",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_org_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// update (request/response).
    pub fn update(&self, req: UpdateOrgRequest) -> Result<Org, ClientError> {
        let csil_req = match encode_update_org_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Orgs", "update", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "update",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_org(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-member (request/response).
    pub fn set_member(&self, req: SetOrgMemberRequest) -> Result<OrgMember, ClientError> {
        let csil_req = match encode_set_org_member_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Orgs", "set-member", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "set-member",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_org_member(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// remove-member (request/response).
    pub fn remove_member(&self, req: RemoveOrgMemberRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_remove_org_member_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Orgs", "remove-member", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "remove-member",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-members (request/response).
    pub fn list_members(&self, req: ListOrgMembersRequest) -> Result<OrgMemberPage, ClientError> {
        let csil_req = match encode_list_org_members_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Orgs", "list-members", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "list-members",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_org_member_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// create-team (request/response).
    pub fn create_team(&self, req: CreateTeamRequest) -> Result<Team, ClientError> {
        let csil_req = match encode_create_team_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Orgs", "create-team", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "create-team",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_team(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// delete-team (request/response).
    pub fn delete_team(&self, req: TeamRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_team_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Orgs", "delete-team", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "delete-team",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-team-member (request/response).
    pub fn set_team_member(&self, req: SetTeamMemberRequest) -> Result<Team, ClientError> {
        let csil_req = match encode_set_team_member_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Orgs", "set-team-member", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "set-team-member",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_team(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-teams (request/response).
    pub fn list_teams(&self, req: GetOrgRequest) -> Result<ListTeamsResponse, ClientError> {
        let csil_req = match encode_get_org_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Orgs", "list-teams", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Orgs",
            "list-teams",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_teams_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Repos service.
pub struct ReposClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> ReposClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create (request/response).
    pub fn create(&self, req: CreateRepoRequest) -> Result<Repo, ClientError> {
        let csil_req = match encode_create_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "create", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "create",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// get (request/response).
    pub fn get(&self, req: GetRepoRequest) -> Result<Repo, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "get", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "get",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list (request/response).
    pub fn list(&self, req: ListReposRequest) -> Result<RepoPage, ClientError> {
        let csil_req = match encode_list_repos_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "list", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "list",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// update (request/response).
    pub fn update(&self, req: UpdateRepoRequest) -> Result<Repo, ClientError> {
        let csil_req = match encode_update_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "update", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "update",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// fork (request/response).
    pub fn fork(&self, req: ForkRequest) -> Result<Repo, ClientError> {
        let csil_req = match encode_fork_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "fork", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "fork",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-refs (request/response).
    pub fn list_refs(&self, req: ListRefsRequest) -> Result<ListRefsResponse, ClientError> {
        let csil_req = match encode_list_refs_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "list-refs", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "list-refs",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_refs_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// log (request/response).
    pub fn log(&self, req: LogRequest) -> Result<CommitPage, ClientError> {
        let csil_req = match encode_log_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "log", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "log",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_commit_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// tree (request/response).
    pub fn tree(&self, req: TreeRequest) -> Result<TreeResponse, ClientError> {
        let csil_req = match encode_tree_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "tree", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "tree",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_tree_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// blob (request/response).
    pub fn blob(&self, req: BlobRequest) -> Result<BlobResponse, ClientError> {
        let csil_req = match encode_blob_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "blob", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "blob",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_blob_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// diff (request/response).
    pub fn diff(&self, req: DiffRequest) -> Result<DiffResponse, ClientError> {
        let csil_req = match encode_diff_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "diff", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "diff",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_diff_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// delete (request/response).
    pub fn delete(&self, req: DeleteRepoRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_delete_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "delete", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "delete",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// update-ref (request/response).
    pub fn update_ref(&self, req: UpdateRefRequest) -> Result<RefInfo, ClientError> {
        let csil_req = match encode_update_ref_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "update-ref", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "update-ref",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ref_info(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// get-commit (request/response).
    pub fn get_commit(&self, req: GetCommitRequest) -> Result<Commit, ClientError> {
        let csil_req = match encode_get_commit_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "get-commit", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "get-commit",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_commit(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-grant (request/response).
    pub fn set_grant(&self, req: SetGrantRequest) -> Result<Grant, ClientError> {
        let csil_req = match encode_set_grant_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Repos", "set-grant", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "set-grant",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_grant(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// remove-grant (request/response).
    pub fn remove_grant(&self, req: RemoveGrantRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_remove_grant_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "remove-grant", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "remove-grant",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-grants (request/response).
    pub fn list_grants(&self, req: GetRepoRequest) -> Result<ListGrantsResponse, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "list-grants", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "list-grants",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_grants_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-protection (request/response).
    pub fn set_protection(
        &self,
        req: SetProtectionRequest,
    ) -> Result<BranchProtection, ClientError> {
        let csil_req = match encode_set_protection_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "set-protection", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "set-protection",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_branch_protection(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// remove-protection (request/response).
    pub fn remove_protection(&self, req: RemoveProtectionRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_remove_protection_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "remove-protection", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "remove-protection",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-protections (request/response).
    pub fn list_protections(
        &self,
        req: GetRepoRequest,
    ) -> Result<ListProtectionsResponse, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "list-protections", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "list-protections",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_protections_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Pack every object reachable from the current refs that no live segment
    /// holds, and upload it as one new segment. Use it for history that was
    /// written before the object store existed. Repeatable; no-op when covered.
    pub fn capture_history(&self, req: GetRepoRequest) -> Result<CaptureReport, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "capture-history", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "capture-history",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_capture_report(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Direct forks of the repository that the caller can see, on this
    /// instance. A pull request into the repository may come from one.
    pub fn list_forks(&self, req: GetRepoRequest) -> Result<RepoPage, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Repos", "list-forks", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Repos",
            "list-forks",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the PullRequests service.
pub struct PullRequestsClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> PullRequestsClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// open (request/response).
    pub fn open(&self, req: OpenPrRequest) -> Result<PullRequest, ClientError> {
        let csil_req = match encode_open_pr_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "open", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "open",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// get (request/response).
    pub fn get(&self, req: PrRef) -> Result<PullRequest, ClientError> {
        let csil_req = match encode_pr_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "get", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "get",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list (request/response).
    pub fn list(&self, req: ListPrRequest) -> Result<PrPage, ClientError> {
        let csil_req = match encode_list_pr_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "list", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "list",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pr_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// review (request/response).
    pub fn review(&self, req: ReviewRequest) -> Result<Review, ClientError> {
        let csil_req = match encode_review_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "review", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "review",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_review(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// comment (request/response).
    pub fn comment(&self, req: CommentRequest) -> Result<Comment, ClientError> {
        let csil_req = match encode_comment_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "comment", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "comment",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_comment(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// merge (request/response).
    pub fn merge(&self, req: MergeRequest) -> Result<MergeResult, ClientError> {
        let csil_req = match encode_merge_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "merge", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "merge",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_merge_result(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// close (request/response).
    pub fn close(&self, req: PrRef) -> Result<PullRequest, ClientError> {
        let csil_req = match encode_pr_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "close", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "close",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// reopen (request/response).
    pub fn reopen(&self, req: PrRef) -> Result<PullRequest, ClientError> {
        let csil_req = match encode_pr_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "reopen", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "reopen",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// retarget (request/response).
    pub fn retarget(&self, req: RetargetRequest) -> Result<PullRequest, ClientError> {
        let csil_req = match encode_retarget_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "retarget", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "retarget",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// update (request/response).
    pub fn update(&self, req: UpdatePrRequest) -> Result<PullRequest, ClientError> {
        let csil_req = match encode_update_pr_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "update", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "update",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-reviews (request/response).
    pub fn list_reviews(&self, req: PrRef) -> Result<ListReviewsResponse, ClientError> {
        let csil_req = match encode_pr_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "list-reviews", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "list-reviews",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_reviews_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-comments (request/response).
    pub fn list_comments(&self, req: PrRef) -> Result<ListCommentsResponse, ClientError> {
        let csil_req = match encode_pr_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "list-comments", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "list-comments",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_comments_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// edit-comment (request/response).
    pub fn edit_comment(&self, req: EditCommentRequest) -> Result<Comment, ClientError> {
        let csil_req = match encode_edit_comment_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "edit-comment", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "edit-comment",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_comment(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// delete-comment (request/response).
    pub fn delete_comment(&self, req: DeleteCommentRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_delete_comment_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("PullRequests", "delete-comment", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "PullRequests",
            "delete-comment",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Statuses service.
pub struct StatusesClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> StatusesClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// set (request/response).
    pub fn set(&self, req: SetStatusRequest) -> Result<CommitStatus, ClientError> {
        let csil_req = match encode_set_status_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Statuses", "set", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Statuses",
            "set",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_commit_status(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list (request/response).
    pub fn list(&self, req: ListStatusRequest) -> Result<ListStatusResponse, ClientError> {
        let csil_req = match encode_list_status_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Statuses", "list", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Statuses",
            "list",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_status_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Hooks service.
pub struct HooksClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> HooksClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create-endpoint (request/response).
    pub fn create_endpoint(&self, req: CreateEndpointRequest) -> Result<Endpoint, ClientError> {
        let csil_req = match encode_create_endpoint_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Hooks", "create-endpoint", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Hooks",
            "create-endpoint",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_endpoint(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-endpoints (request/response).
    pub fn list_endpoints(
        &self,
        req: ListEndpointsRequest,
    ) -> Result<ListEndpointsResponse, ClientError> {
        let csil_req = match encode_list_endpoints_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Hooks", "list-endpoints", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Hooks",
            "list-endpoints",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_endpoints_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// delete-endpoint (request/response).
    pub fn delete_endpoint(&self, req: EndpointRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_endpoint_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Hooks", "delete-endpoint", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Hooks",
            "delete-endpoint",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-deliveries (request/response).
    pub fn list_deliveries(&self, req: DeliveryQuery) -> Result<DeliveryPage, ClientError> {
        let csil_req = match encode_delivery_query(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Hooks", "list-deliveries", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Hooks",
            "list-deliveries",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_delivery_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// redeliver (request/response).
    pub fn redeliver(&self, req: DeliveryRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_delivery_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Hooks", "redeliver", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Hooks",
            "redeliver",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// update-endpoint (request/response).
    pub fn update_endpoint(&self, req: UpdateEndpointRequest) -> Result<Endpoint, ClientError> {
        let csil_req = match encode_update_endpoint_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Hooks", "update-endpoint", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Hooks",
            "update-endpoint",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_endpoint(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Mirror service.
pub struct MirrorClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> MirrorClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// status (request/response).
    pub fn status(&self, req: GetRepoRequest) -> Result<MirrorStatus, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "status", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "status",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-blocked (request/response).
    pub fn list_blocked(&self, req: GetRepoRequest) -> Result<ListBlockedResponse, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-blocked", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-blocked",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_blocked_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// replay-blocked (request/response).
    pub fn replay_blocked(&self, req: ReplayRequest) -> Result<ReplayResult, ClientError> {
        let csil_req = match encode_replay_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "replay-blocked", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "replay-blocked",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_replay_result(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-credential (request/response).
    pub fn set_credential(&self, req: SetCredentialRequest) -> Result<Credential, ClientError> {
        let csil_req = match encode_set_credential_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "set-credential", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "set-credential",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_credential(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// reconcile (request/response).
    pub fn reconcile(&self, req: GetRepoRequest) -> Result<ReconcileReport, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "reconcile", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "reconcile",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_reconcile_report(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// pause (request/response).
    pub fn pause(&self, req: GetRepoRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "pause", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "pause",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// resume (request/response).
    pub fn resume(&self, req: GetRepoRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "resume", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "resume",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Make the repository and queue its import. The answer comes before
    /// the branches are fetched; `status` reports `import_state`.
    pub fn import_repo(&self, req: ImportRequest) -> Result<Repo, ClientError> {
        let csil_req = match encode_import_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "import-repo", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "import-repo",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// configure (request/response).
    pub fn configure(&self, req: ConfigureMirrorRequest) -> Result<MirrorStatus, ClientError> {
        let csil_req = match encode_configure_mirror_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "configure", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "configure",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-credentials (request/response).
    pub fn list_credentials(
        &self,
        req: ListCredentialsRequest,
    ) -> Result<ListCredentialsResponse, ClientError> {
        let csil_req = match encode_list_credentials_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-credentials", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-credentials",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_credentials_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// remove-credential (request/response).
    pub fn remove_credential(&self, req: CredentialRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_credential_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "remove-credential", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "remove-credential",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Repositories visible under the organization's `github_owner`, with
    /// the vcsstar repository that mirrors each one when there is one.
    /// Enable one with `import-repo`.
    pub fn list_github_repos(
        &self,
        req: ListGithubReposRequest,
    ) -> Result<GithubRepoPage, ClientError> {
        let csil_req = match encode_list_github_repos_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-github-repos", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-github-repos",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_github_repo_page(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// This node's view of GitHub availability and rate limits.
    pub fn health(&self, req: Empty) -> Result<GithubHealth, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "health", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "health",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_github_health(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Start the history import of the repository's GitHub pull requests
    /// again from the first one. A mirrored repository imports its history
    /// by itself; this is for a repository whose import must run again.
    pub fn backfill(&self, req: GetRepoRequest) -> Result<MirrorStatus, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "backfill", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "backfill",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Where the instance's GitHub Apps are installed, with the vcsstar
    /// organizations whose `github_owner` is each owner.
    pub fn list_github_installations(
        &self,
        req: Empty,
    ) -> Result<GithubInstallationList, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply =
            self.transport
                .call_reply("Mirror", "list-github-installations", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-github-installations",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_github_installation_list(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// import-all (request/response).
    pub fn import_all(&self, req: ImportAllRequest) -> Result<ImportAllResult, ClientError> {
        let csil_req = match encode_import_all_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "import-all", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "import-all",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_import_all_result(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-schedule (request/response).
    pub fn set_schedule(&self, req: SetMirrorScheduleRequest) -> Result<MirrorStatus, ClientError> {
        let csil_req = match encode_set_mirror_schedule_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "set-schedule", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "set-schedule",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-connections (request/response).
    pub fn list_connections(&self, req: GetRepoRequest) -> Result<SyncConnections, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-connections", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-connections",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_connections(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// connect (request/response).
    pub fn connect(&self, req: ConnectRemoteRequest) -> Result<SyncConnection, ClientError> {
        let csil_req = match encode_connect_remote_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "connect", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "connect",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_connection(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// update-connection (request/response).
    pub fn update_connection(
        &self,
        req: UpdateConnectionRequest,
    ) -> Result<SyncConnection, ClientError> {
        let csil_req = match encode_update_connection_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "update-connection", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "update-connection",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_connection(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// sync-now (request/response).
    pub fn sync_now(&self, req: SyncConnectionRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_sync_connection_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self.transport.call_reply("Mirror", "sync-now", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "sync-now",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// webhook-secret (request/response).
    pub fn webhook_secret(&self, req: SyncConnectionRef) -> Result<SyncWebhookSecret, ClientError> {
        let csil_req = match encode_sync_connection_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "webhook-secret", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "webhook-secret",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_webhook_secret(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// get-policy (request/response).
    pub fn get_policy(&self, req: GetRepoRequest) -> Result<SyncPolicy, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "get-policy", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "get-policy",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_policy(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-policy (request/response).
    pub fn set_policy(&self, req: SetSyncPolicyRequest) -> Result<SyncPolicy, ClientError> {
        let csil_req = match encode_set_sync_policy_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "set-policy", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "set-policy",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_policy(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-conflicts (request/response).
    pub fn list_conflicts(&self, req: GetRepoRequest) -> Result<SyncConflicts, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-conflicts", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-conflicts",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_conflicts(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// resolve-conflict (request/response).
    pub fn resolve_conflict(&self, req: ResolveSyncConflictRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_resolve_sync_conflict_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "resolve-conflict", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "resolve-conflict",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// link-provider (request/response).
    pub fn link_provider(&self, req: LinkProviderRequest) -> Result<ProviderLink, ClientError> {
        let csil_req = match encode_link_provider_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "link-provider", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "link-provider",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_provider_link(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-provider-links (request/response).
    pub fn list_provider_links(&self, req: Empty) -> Result<ProviderLinks, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-provider-links", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-provider-links",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_provider_links(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// unlink-provider (request/response).
    pub fn unlink_provider(&self, req: UnlinkProviderRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_unlink_provider_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "unlink-provider", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "unlink-provider",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-sync-records (request/response).
    pub fn list_sync_records(&self, req: SyncRecordsRequest) -> Result<SyncRecords, ClientError> {
        let csil_req = match encode_sync_records_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Mirror", "list-sync-records", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Mirror",
            "list-sync-records",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_sync_records(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Instance service.
pub struct InstanceClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> InstanceClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Every instance role. Instance admins only.
    pub fn list_roles(&self, req: Empty) -> Result<InstanceRoleList, ClientError> {
        let csil_req = match encode_empty(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Instance", "list-roles", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Instance",
            "list-roles",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_instance_role_list(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// set-role (request/response).
    pub fn set_role(&self, req: SetInstanceRoleRequest) -> Result<InstanceRoleGrant, ClientError> {
        let csil_req = match encode_set_instance_role_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Instance", "set-role", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Instance",
            "set-role",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_instance_role_grant(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// remove-role (request/response).
    pub fn remove_role(&self, req: RemoveInstanceRoleRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_remove_instance_role_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Instance", "remove-role", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Instance",
            "remove-role",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// create-local-account (request/response).
    pub fn create_local_account(
        &self,
        req: CreateLocalAccountRequest,
    ) -> Result<LocalAccount, ClientError> {
        let csil_req = match encode_create_local_account_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply =
            self.transport
                .call_reply("Instance", "create-local-account", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Instance",
            "create-local-account",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_local_account(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// reset-local-password (request/response).
    pub fn reset_local_password(&self, req: ResetLocalPasswordRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_reset_local_password_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply =
            self.transport
                .call_reply("Instance", "reset-local-password", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Instance",
            "reset-local-password",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the ServiceAccounts service.
pub struct ServiceAccountsClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> ServiceAccountsClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create (request/response).
    pub fn create(&self, req: CreateServiceAccountRequest) -> Result<ServiceAccount, ClientError> {
        let csil_req = match encode_create_service_account_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "create", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "create",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_service_account(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list (request/response).
    pub fn list(&self, req: GetOrgRequest) -> Result<ListServiceAccountsResponse, ClientError> {
        let csil_req = match encode_get_org_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "list", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "list",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_service_accounts_response(&csil_resp)
            .map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// Disabling refuses every token of the account; enabling lets the
    /// tokens that were not revoked work again.
    pub fn disable(&self, req: ServiceAccountRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_service_account_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "disable", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "disable",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// enable (request/response).
    pub fn enable(&self, req: ServiceAccountRef) -> Result<Ok, ClientError> {
        let csil_req = match encode_service_account_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "enable", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "enable",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// create-token (request/response).
    pub fn create_token(
        &self,
        req: CreateServiceAccountTokenRequest,
    ) -> Result<ApiToken, ClientError> {
        let csil_req = match encode_create_service_account_token_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "create-token", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "create-token",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_api_token(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// list-tokens (request/response).
    pub fn list_tokens(&self, req: ServiceAccountRef) -> Result<ListTokensResponse, ClientError> {
        let csil_req = match encode_service_account_ref(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "list-tokens", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "list-tokens",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_list_tokens_response(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// revoke-token (request/response).
    pub fn revoke_token(&self, req: RevokeServiceAccountTokenRequest) -> Result<Ok, ClientError> {
        let csil_req = match encode_revoke_service_account_token_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("ServiceAccounts", "revoke-token", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "ServiceAccounts",
            "revoke-token",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the Federation service.
pub struct FederationClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> FederationClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// handshake (request/response).
    pub fn handshake(&self, req: FedHello) -> Result<FedHelloAck, ClientError> {
        let csil_req = match encode_fed_hello(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Federation", "handshake", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Federation",
            "handshake",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_fed_hello_ack(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// verify-head (request/response).
    pub fn verify_head(&self, req: VerifyHeadRequest) -> Result<HeadInfo, ClientError> {
        let csil_req = match encode_verify_head_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Federation", "verify-head", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Federation",
            "verify-head",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_head_info(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// head-info (request/response).
    pub fn head_info(&self, req: HeadQuery) -> Result<HeadInfo, ClientError> {
        let csil_req = match encode_head_query(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Federation", "head-info", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Federation",
            "head-info",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_head_info(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// repo-info (request/response).
    pub fn repo_info(&self, req: GetRepoRequest) -> Result<Repo, ClientError> {
        let csil_req = match encode_get_repo_request(&req) {
            Ok(csil_bytes) => csil_bytes,
            Err(e) => return Err(ClientError::Encode(e.to_string())),
        };
        let csil_reply = self
            .transport
            .call_reply("Federation", "repo-info", &csil_req)?;
        let csil_resp = resolve_rpc_reply(
            "Federation",
            "repo-info",
            csil_reply.variant.as_deref(),
            csil_reply.payload,
        )?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Decode(e.to_string()))
    }
}

/// Typed client for the EventStream service.
pub struct EventStreamClient<T: Transport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: Transport> EventStreamClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    // channel operation `subscribe` is not part of the RPC client
}
