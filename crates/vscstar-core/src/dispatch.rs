//! The dispatch table: every `(service, op)` in `vscstar.csil` maps to a
//! handler here. csilgen generates shapes and routing helpers, not a router,
//! so this table is hand-written and tested for completeness against the
//! schema descriptor.

use vscstar_transport::{Dispatcher, HandlerOutcome, Peer, RpcRequest, Status};
use vscstar_types as t;

use crate::principal::{self, ReqCtx};
use crate::services::{auth, login, meta, orgs, repos};
use crate::{App, Error};

pub struct AppDispatcher {
    pub app: App,
}

impl AppDispatcher {
    pub fn new(app: App) -> Self {
        Self { app }
    }
}

/// Generic execution of one typed operation: decode, resolve the caller,
/// run the handler, encode. Application errors travel as a status-0 reply
/// with the `ServiceError` variant, exactly as the generated clients expect.
#[allow(clippy::too_many_arguments)]
fn run<Req, Resp>(
    app: &App,
    peer: &Peer,
    payload: &[u8],
    decode: fn(&[u8]) -> Result<Req, t::CsilCborError>,
    ctx_of: fn(&Req) -> &t::Ctx,
    encode: fn(&Resp) -> Vec<u8>,
    variant: &str,
    handler: fn(&App, &ReqCtx, Req) -> Result<Resp, Error>,
) -> HandlerOutcome {
    let req = match decode(payload) {
        Ok(r) => r,
        Err(e) => {
            return HandlerOutcome::Transport(
                Status::MalformedEnvelope,
                format!("bad request payload: {e}"),
            )
        }
    };
    let request_id = ctx_of(&req).request_id.clone();
    let principal = match principal::resolve(app, peer) {
        Ok(p) => p,
        Err(e) => return error_reply(e),
    };
    let ctx = ReqCtx {
        request_id,
        principal,
        peer: peer.clone(),
    };
    match handler(app, &ctx, req) {
        Ok(resp) => HandlerOutcome::Reply {
            variant: variant.to_string(),
            payload: encode(&resp),
        },
        Err(e) => error_reply(e),
    }
}

fn error_reply(e: Error) -> HandlerOutcome {
    let wire: t::ServiceError = e.into();
    HandlerOutcome::Reply {
        variant: "ServiceError".to_string(),
        payload: t::encode_service_error(&wire),
    }
}

fn not_implemented(op: &str) -> HandlerOutcome {
    error_reply(Error::not_implemented(op))
}

/// Operations declared in the contract that have no handler yet. The
/// completeness test accepts these; a release gate can refuse them later.
pub const NOT_IMPLEMENTED: &[(&str, &str)] = &[
    ("Auth", "exchange-act-as-credential"),
    ("Auth", "scope-set"),
    ("Auth", "register-key"),
    ("Auth", "list-keys"),
    ("Auth", "revoke-key"),
    ("Repos", "fork"),
    ("Repos", "list-refs"),
    ("Repos", "log"),
    ("Repos", "tree"),
    ("Repos", "blob"),
    ("Repos", "diff"),
    ("Repos", "update-ref"),
    ("Repos", "get-commit"),
    ("PullRequests", "open"),
    ("PullRequests", "get"),
    ("PullRequests", "list"),
    ("PullRequests", "review"),
    ("PullRequests", "comment"),
    ("PullRequests", "merge"),
    ("PullRequests", "close"),
    ("PullRequests", "reopen"),
    ("PullRequests", "retarget"),
    ("PullRequests", "update"),
    ("PullRequests", "list-reviews"),
    ("PullRequests", "list-comments"),
    ("Statuses", "set"),
    ("Statuses", "list"),
    ("Hooks", "create-endpoint"),
    ("Hooks", "list-endpoints"),
    ("Hooks", "delete-endpoint"),
    ("Hooks", "list-deliveries"),
    ("Hooks", "redeliver"),
    ("Hooks", "update-endpoint"),
    ("Mirror", "status"),
    ("Mirror", "list-blocked"),
    ("Mirror", "replay-blocked"),
    ("Mirror", "set-credential"),
    ("Mirror", "reconcile"),
    ("Mirror", "pause"),
    ("Mirror", "resume"),
    ("Mirror", "import-repo"),
    ("Mirror", "configure"),
    ("Mirror", "list-credentials"),
    ("Mirror", "remove-credential"),
    ("Federation", "handshake"),
    ("Federation", "verify-head"),
    ("Federation", "head-info"),
    ("Federation", "repo-info"),
];

/// Operations with a handler. Kept as data so the completeness test can
/// compare against the descriptor without a database.
pub const IMPLEMENTED: &[(&str, &str)] = &[
    ("Meta", "server-info"),
    ("Auth", "begin-login"),
    ("Auth", "complete-login"),
    ("Auth", "begin-device-login"),
    ("Auth", "poll-device-login"),
    ("Auth", "approve-device-login"),
    ("Auth", "create-token"),
    ("Auth", "revoke-token"),
    ("Auth", "whoami"),
    ("Auth", "list-tokens"),
    ("Auth", "logout"),
    ("Orgs", "create"),
    ("Orgs", "get"),
    ("Orgs", "list"),
    ("Orgs", "update"),
    ("Orgs", "set-member"),
    ("Orgs", "remove-member"),
    ("Orgs", "list-members"),
    ("Orgs", "create-team"),
    ("Orgs", "delete-team"),
    ("Orgs", "set-team-member"),
    ("Orgs", "list-teams"),
    ("Repos", "create"),
    ("Repos", "get"),
    ("Repos", "list"),
    ("Repos", "update"),
    ("Repos", "delete"),
    ("Repos", "set-grant"),
    ("Repos", "remove-grant"),
    ("Repos", "list-grants"),
    ("Repos", "set-protection"),
    ("Repos", "remove-protection"),
    ("Repos", "list-protections"),
];

/// Streaming operations are served by the events carrier, not this table.
pub const STREAMING: &[(&str, &str)] = &[("EventStream", "subscribe")];

macro_rules! op {
    ($app:expr, $peer:expr, $payload:expr, $dec:ident, $enc:ident, $variant:literal, $handler:path) => {
        run(
            $app,
            $peer,
            $payload,
            t::$dec,
            |r| &r.ctx,
            t::$enc,
            $variant,
            $handler,
        )
    };
}

impl Dispatcher for AppDispatcher {
    fn dispatch(&self, request: &RpcRequest, peer: &Peer) -> HandlerOutcome {
        let app = &self.app;
        let payload = request.payload.as_slice();
        match (request.service.as_str(), request.op.as_str()) {
            ("Meta", "server-info") => op!(
                app,
                peer,
                payload,
                decode_empty,
                encode_server_info,
                "ServerInfo",
                meta::server_info
            ),

            ("Auth", "begin-login") => op!(
                app,
                peer,
                payload,
                decode_begin_login_request,
                encode_begin_login_response,
                "BeginLoginResponse",
                login::begin
            ),
            ("Auth", "complete-login") => op!(
                app,
                peer,
                payload,
                decode_complete_login_request,
                encode_session,
                "Session",
                login::complete
            ),
            ("Auth", "begin-device-login") => op!(
                app,
                peer,
                payload,
                decode_begin_device_request,
                encode_device_code,
                "DeviceCode",
                login::begin_device
            ),
            ("Auth", "poll-device-login") => op!(
                app,
                peer,
                payload,
                decode_device_poll_request,
                encode_device_poll_response,
                "DevicePollResponse",
                login::poll_device
            ),
            ("Auth", "approve-device-login") => op!(
                app,
                peer,
                payload,
                decode_device_approve_request,
                encode_ok,
                "Ok",
                login::approve_device
            ),
            ("Auth", "create-token") => op!(
                app,
                peer,
                payload,
                decode_create_token_request,
                encode_api_token,
                "ApiToken",
                auth::create_token
            ),
            ("Auth", "revoke-token") => op!(
                app,
                peer,
                payload,
                decode_revoke_token_request,
                encode_ok,
                "Ok",
                auth::revoke_token
            ),
            ("Auth", "whoami") => op!(
                app,
                peer,
                payload,
                decode_empty,
                encode_who_am_i,
                "WhoAmI",
                auth::whoami
            ),
            ("Auth", "list-tokens") => op!(
                app,
                peer,
                payload,
                decode_empty,
                encode_list_tokens_response,
                "ListTokensResponse",
                auth::list_tokens
            ),
            ("Auth", "logout") => op!(
                app,
                peer,
                payload,
                decode_empty,
                encode_ok,
                "Ok",
                auth::logout
            ),

            ("Orgs", "create") => op!(
                app,
                peer,
                payload,
                decode_create_org_request,
                encode_org,
                "Org",
                orgs::create
            ),
            ("Orgs", "get") => op!(
                app,
                peer,
                payload,
                decode_get_org_request,
                encode_org,
                "Org",
                orgs::get
            ),
            ("Orgs", "list") => op!(
                app,
                peer,
                payload,
                decode_list_orgs_request,
                encode_org_page,
                "OrgPage",
                orgs::list
            ),
            ("Orgs", "update") => op!(
                app,
                peer,
                payload,
                decode_update_org_request,
                encode_org,
                "Org",
                orgs::update
            ),
            ("Orgs", "set-member") => op!(
                app,
                peer,
                payload,
                decode_set_org_member_request,
                encode_org_member,
                "OrgMember",
                orgs::set_member
            ),
            ("Orgs", "remove-member") => op!(
                app,
                peer,
                payload,
                decode_remove_org_member_request,
                encode_ok,
                "Ok",
                orgs::remove_member
            ),
            ("Orgs", "list-members") => op!(
                app,
                peer,
                payload,
                decode_list_org_members_request,
                encode_org_member_page,
                "OrgMemberPage",
                orgs::list_members
            ),
            ("Orgs", "create-team") => op!(
                app,
                peer,
                payload,
                decode_create_team_request,
                encode_team,
                "Team",
                orgs::create_team
            ),
            ("Orgs", "delete-team") => op!(
                app,
                peer,
                payload,
                decode_team_ref,
                encode_ok,
                "Ok",
                orgs::delete_team
            ),
            ("Orgs", "set-team-member") => op!(
                app,
                peer,
                payload,
                decode_set_team_member_request,
                encode_team,
                "Team",
                orgs::set_team_member
            ),
            ("Orgs", "list-teams") => op!(
                app,
                peer,
                payload,
                decode_get_org_request,
                encode_list_teams_response,
                "ListTeamsResponse",
                orgs::list_teams
            ),

            ("Repos", "create") => op!(
                app,
                peer,
                payload,
                decode_create_repo_request,
                encode_repo,
                "Repo",
                repos::create
            ),
            ("Repos", "get") => op!(
                app,
                peer,
                payload,
                decode_get_repo_request,
                encode_repo,
                "Repo",
                repos::get
            ),
            ("Repos", "list") => op!(
                app,
                peer,
                payload,
                decode_list_repos_request,
                encode_repo_page,
                "RepoPage",
                repos::list
            ),
            ("Repos", "update") => op!(
                app,
                peer,
                payload,
                decode_update_repo_request,
                encode_repo,
                "Repo",
                repos::update
            ),
            ("Repos", "delete") => op!(
                app,
                peer,
                payload,
                decode_delete_repo_request,
                encode_ok,
                "Ok",
                repos::delete
            ),
            ("Repos", "set-grant") => op!(
                app,
                peer,
                payload,
                decode_set_grant_request,
                encode_grant,
                "Grant",
                repos::set_grant
            ),
            ("Repos", "remove-grant") => op!(
                app,
                peer,
                payload,
                decode_remove_grant_request,
                encode_ok,
                "Ok",
                repos::remove_grant
            ),
            ("Repos", "list-grants") => op!(
                app,
                peer,
                payload,
                decode_get_repo_request,
                encode_list_grants_response,
                "ListGrantsResponse",
                repos::list_grants
            ),
            ("Repos", "set-protection") => op!(
                app,
                peer,
                payload,
                decode_set_protection_request,
                encode_branch_protection,
                "BranchProtection",
                repos::set_protection
            ),
            ("Repos", "remove-protection") => op!(
                app,
                peer,
                payload,
                decode_remove_protection_request,
                encode_ok,
                "Ok",
                repos::remove_protection
            ),
            ("Repos", "list-protections") => op!(
                app,
                peer,
                payload,
                decode_get_repo_request,
                encode_list_protections_response,
                "ListProtectionsResponse",
                repos::list_protections
            ),

            (service, op) if NOT_IMPLEMENTED.contains(&(service, op)) => {
                not_implemented(&format!("{service}/{op}"))
            }
            (service, op) => HandlerOutcome::Transport(
                Status::UnknownServiceOrOp,
                format!("unknown operation {service}/{op}"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every operation in the schema descriptor is either implemented,
    /// explicitly listed as not implemented, or a streaming op. Nothing in
    /// the contract is silently unreachable.
    #[test]
    fn dispatch_table_covers_the_contract() {
        let desc: ciborium::Value =
            ciborium::de::from_reader(meta::DESCRIPTOR).expect("descriptor decodes");
        let body = map_get(&desc, "body").expect("body");
        let services = map_get(body, "services").expect("services");
        let mut in_contract = BTreeSet::new();
        for svc in services.as_array().expect("services array") {
            let name = map_get(svc, "name")
                .and_then(|v| v.as_text())
                .expect("service name")
                .to_string();
            for op in map_get(svc, "operations")
                .and_then(|v| v.as_array())
                .expect("ops")
            {
                let op_name = map_get(op, "name")
                    .and_then(|v| v.as_text())
                    .expect("op name")
                    .to_string();
                in_contract.insert((name.clone(), op_name));
            }
        }
        let mut in_table = BTreeSet::new();
        for (s, o) in IMPLEMENTED.iter().chain(NOT_IMPLEMENTED).chain(STREAMING) {
            assert!(
                in_table.insert((s.to_string(), o.to_string())),
                "duplicate table entry {s}/{o}"
            );
        }
        let missing: Vec<_> = in_contract.difference(&in_table).collect();
        let extra: Vec<_> = in_table.difference(&in_contract).collect();
        assert!(
            missing.is_empty(),
            "contract ops without a dispatch entry: {missing:?}"
        );
        assert!(
            extra.is_empty(),
            "dispatch entries not in the contract: {extra:?}"
        );
        assert!(
            in_contract.len() > 80,
            "descriptor looks truncated: {} ops",
            in_contract.len()
        );
    }

    fn map_get<'a>(v: &'a ciborium::Value, key: &str) -> Option<&'a ciborium::Value> {
        v.as_map()?
            .iter()
            .find(|(k, _)| k.as_text() == Some(key))
            .map(|(_, v)| v)
    }
}
