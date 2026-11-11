//! Generated service traits from CSIL specification

use super::types::*;

/// Meta service trait
pub trait Meta {
    type Context;
    /// server-info (request/response).
    fn server_info(&self, ctx: &Self::Context, input: Empty) -> Result<ServerInfo, ServiceError>;
}

/// Wire-id ordinals for the Meta service (transport compact profiles).
pub mod meta_wire_ids {
    pub const SERVICE: u64 = 10;
    pub const OP_SERVER_INFO: u64 = 0;
}

/// Auth service trait
pub trait Auth {
    type Context;
    /// begin-login (request/response).
    fn begin_login(
        &self,
        ctx: &Self::Context,
        input: BeginLoginRequest,
    ) -> Result<BeginLoginResponse, ServiceError>;
    /// complete-login (request/response).
    fn complete_login(
        &self,
        ctx: &Self::Context,
        input: CompleteLoginRequest,
    ) -> Result<Session, ServiceError>;
    /// create-token (request/response).
    fn create_token(
        &self,
        ctx: &Self::Context,
        input: CreateTokenRequest,
    ) -> Result<ApiToken, ServiceError>;
    /// revoke-token (request/response).
    fn revoke_token(
        &self,
        ctx: &Self::Context,
        input: RevokeTokenRequest,
    ) -> Result<Ok, ServiceError>;
    /// whoami (request/response).
    fn whoami(&self, ctx: &Self::Context, input: Empty) -> Result<WhoAmI, ServiceError>;
    /// begin-device-login (request/response).
    fn begin_device_login(
        &self,
        ctx: &Self::Context,
        input: BeginDeviceRequest,
    ) -> Result<DeviceCode, ServiceError>;
    /// poll-device-login (request/response).
    fn poll_device_login(
        &self,
        ctx: &Self::Context,
        input: DevicePollRequest,
    ) -> Result<DevicePollResponse, ServiceError>;
    /// exchange-act-as-credential (request/response).
    fn exchange_act_as_credential(
        &self,
        ctx: &Self::Context,
        input: ActAsExchangeRequest,
    ) -> Result<ScopedBearer, ServiceError>;
    /// scope-set (request/response).
    fn scope_set(
        &self,
        ctx: &Self::Context,
        input: ScopeSetRequest,
    ) -> Result<SignedActAsScopeSet, ServiceError>;
    /// Browser side of the device flow. Needs a web session.
    fn approve_device_login(
        &self,
        ctx: &Self::Context,
        input: DeviceApproveRequest,
    ) -> Result<Ok, ServiceError>;
    /// register-key (request/response).
    fn register_key(
        &self,
        ctx: &Self::Context,
        input: RegisterKeyRequest,
    ) -> Result<SshKeyInfo, ServiceError>;
    /// list-keys (request/response).
    fn list_keys(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<ListKeysResponse, ServiceError>;
    /// revoke-key (request/response).
    fn revoke_key(&self, ctx: &Self::Context, input: KeyRef) -> Result<Ok, ServiceError>;
    /// list-tokens (request/response).
    fn list_tokens(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<ListTokensResponse, ServiceError>;
    /// logout (request/response).
    fn logout(&self, ctx: &Self::Context, input: Empty) -> Result<Ok, ServiceError>;
    /// GitHub account link of the caller (see GithubLink).
    fn github_link_status(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<GithubLink, ServiceError>;
    /// set-github-token (request/response).
    fn set_github_token(
        &self,
        ctx: &Self::Context,
        input: SetGithubTokenRequest,
    ) -> Result<GithubLink, ServiceError>;
    /// unlink-github (request/response).
    fn unlink_github(&self, ctx: &Self::Context, input: Empty) -> Result<Ok, ServiceError>;
}

/// Wire-id ordinals for the Auth service (transport compact profiles).
pub mod auth_wire_ids {
    pub const SERVICE: u64 = 1;
    pub const OP_BEGIN_LOGIN: u64 = 0;
    pub const OP_COMPLETE_LOGIN: u64 = 1;
    pub const OP_CREATE_TOKEN: u64 = 2;
    pub const OP_REVOKE_TOKEN: u64 = 3;
    pub const OP_WHOAMI: u64 = 4;
    pub const OP_BEGIN_DEVICE_LOGIN: u64 = 5;
    pub const OP_POLL_DEVICE_LOGIN: u64 = 6;
    pub const OP_EXCHANGE_ACT_AS_CREDENTIAL: u64 = 7;
    pub const OP_SCOPE_SET: u64 = 8;
    pub const OP_APPROVE_DEVICE_LOGIN: u64 = 9;
    pub const OP_REGISTER_KEY: u64 = 10;
    pub const OP_LIST_KEYS: u64 = 11;
    pub const OP_REVOKE_KEY: u64 = 12;
    pub const OP_LIST_TOKENS: u64 = 13;
    pub const OP_LOGOUT: u64 = 14;
    pub const OP_GITHUB_LINK_STATUS: u64 = 15;
    pub const OP_SET_GITHUB_TOKEN: u64 = 16;
    pub const OP_UNLINK_GITHUB: u64 = 17;
}

/// Orgs service trait
pub trait Orgs {
    type Context;
    /// create (request/response).
    fn create(&self, ctx: &Self::Context, input: CreateOrgRequest) -> Result<Org, ServiceError>;
    /// get (request/response).
    fn get(&self, ctx: &Self::Context, input: GetOrgRequest) -> Result<Org, ServiceError>;
    /// list (request/response).
    fn list(&self, ctx: &Self::Context, input: ListOrgsRequest) -> Result<OrgPage, ServiceError>;
    /// update (request/response).
    fn update(&self, ctx: &Self::Context, input: UpdateOrgRequest) -> Result<Org, ServiceError>;
    /// set-member (request/response).
    fn set_member(
        &self,
        ctx: &Self::Context,
        input: SetOrgMemberRequest,
    ) -> Result<OrgMember, ServiceError>;
    /// remove-member (request/response).
    fn remove_member(
        &self,
        ctx: &Self::Context,
        input: RemoveOrgMemberRequest,
    ) -> Result<Ok, ServiceError>;
    /// list-members (request/response).
    fn list_members(
        &self,
        ctx: &Self::Context,
        input: ListOrgMembersRequest,
    ) -> Result<OrgMemberPage, ServiceError>;
    /// create-team (request/response).
    fn create_team(
        &self,
        ctx: &Self::Context,
        input: CreateTeamRequest,
    ) -> Result<Team, ServiceError>;
    /// delete-team (request/response).
    fn delete_team(&self, ctx: &Self::Context, input: TeamRef) -> Result<Ok, ServiceError>;
    /// set-team-member (request/response).
    fn set_team_member(
        &self,
        ctx: &Self::Context,
        input: SetTeamMemberRequest,
    ) -> Result<Team, ServiceError>;
    /// list-teams (request/response).
    fn list_teams(
        &self,
        ctx: &Self::Context,
        input: GetOrgRequest,
    ) -> Result<ListTeamsResponse, ServiceError>;
}

/// Wire-id ordinals for the Orgs service (transport compact profiles).
pub mod orgs_wire_ids {
    pub const SERVICE: u64 = 2;
    pub const OP_CREATE: u64 = 0;
    pub const OP_GET: u64 = 1;
    pub const OP_LIST: u64 = 2;
    pub const OP_UPDATE: u64 = 3;
    pub const OP_SET_MEMBER: u64 = 4;
    pub const OP_REMOVE_MEMBER: u64 = 5;
    pub const OP_LIST_MEMBERS: u64 = 6;
    pub const OP_CREATE_TEAM: u64 = 7;
    pub const OP_DELETE_TEAM: u64 = 8;
    pub const OP_SET_TEAM_MEMBER: u64 = 9;
    pub const OP_LIST_TEAMS: u64 = 10;
}

/// Repos service trait
pub trait Repos {
    type Context;
    /// create (request/response).
    fn create(&self, ctx: &Self::Context, input: CreateRepoRequest) -> Result<Repo, ServiceError>;
    /// get (request/response).
    fn get(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Repo, ServiceError>;
    /// list (request/response).
    fn list(&self, ctx: &Self::Context, input: ListReposRequest) -> Result<RepoPage, ServiceError>;
    /// update (request/response).
    fn update(&self, ctx: &Self::Context, input: UpdateRepoRequest) -> Result<Repo, ServiceError>;
    /// fork (request/response).
    fn fork(&self, ctx: &Self::Context, input: ForkRequest) -> Result<Repo, ServiceError>;
    /// list-refs (request/response).
    fn list_refs(
        &self,
        ctx: &Self::Context,
        input: ListRefsRequest,
    ) -> Result<ListRefsResponse, ServiceError>;
    /// log (request/response).
    fn log(&self, ctx: &Self::Context, input: LogRequest) -> Result<CommitPage, ServiceError>;
    /// tree (request/response).
    fn tree(&self, ctx: &Self::Context, input: TreeRequest) -> Result<TreeResponse, ServiceError>;
    /// blob (request/response).
    fn blob(&self, ctx: &Self::Context, input: BlobRequest) -> Result<BlobResponse, ServiceError>;
    /// diff (request/response).
    fn diff(&self, ctx: &Self::Context, input: DiffRequest) -> Result<DiffResponse, ServiceError>;
    /// delete (request/response).
    fn delete(&self, ctx: &Self::Context, input: DeleteRepoRequest) -> Result<Ok, ServiceError>;
    /// update-ref (request/response).
    fn update_ref(
        &self,
        ctx: &Self::Context,
        input: UpdateRefRequest,
    ) -> Result<RefInfo, ServiceError>;
    /// get-commit (request/response).
    fn get_commit(
        &self,
        ctx: &Self::Context,
        input: GetCommitRequest,
    ) -> Result<Commit, ServiceError>;
    /// set-grant (request/response).
    fn set_grant(&self, ctx: &Self::Context, input: SetGrantRequest)
        -> Result<Grant, ServiceError>;
    /// remove-grant (request/response).
    fn remove_grant(
        &self,
        ctx: &Self::Context,
        input: RemoveGrantRequest,
    ) -> Result<Ok, ServiceError>;
    /// list-grants (request/response).
    fn list_grants(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ListGrantsResponse, ServiceError>;
    /// set-protection (request/response).
    fn set_protection(
        &self,
        ctx: &Self::Context,
        input: SetProtectionRequest,
    ) -> Result<BranchProtection, ServiceError>;
    /// remove-protection (request/response).
    fn remove_protection(
        &self,
        ctx: &Self::Context,
        input: RemoveProtectionRequest,
    ) -> Result<Ok, ServiceError>;
    /// list-protections (request/response).
    fn list_protections(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ListProtectionsResponse, ServiceError>;
    /// Pack every object reachable from the current refs that no live segment
    /// holds, and upload it as one new segment. Use it for history that was
    /// written before the object store existed. Repeatable; no-op when covered.
    fn capture_history(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<CaptureReport, ServiceError>;
}

/// Wire-id ordinals for the Repos service (transport compact profiles).
pub mod repos_wire_ids {
    pub const SERVICE: u64 = 3;
    pub const OP_CREATE: u64 = 0;
    pub const OP_GET: u64 = 1;
    pub const OP_LIST: u64 = 2;
    pub const OP_UPDATE: u64 = 3;
    pub const OP_FORK: u64 = 4;
    pub const OP_LIST_REFS: u64 = 5;
    pub const OP_LOG: u64 = 6;
    pub const OP_TREE: u64 = 7;
    pub const OP_BLOB: u64 = 8;
    pub const OP_DIFF: u64 = 9;
    pub const OP_DELETE: u64 = 10;
    pub const OP_UPDATE_REF: u64 = 11;
    pub const OP_GET_COMMIT: u64 = 12;
    pub const OP_SET_GRANT: u64 = 13;
    pub const OP_REMOVE_GRANT: u64 = 14;
    pub const OP_LIST_GRANTS: u64 = 15;
    pub const OP_SET_PROTECTION: u64 = 16;
    pub const OP_REMOVE_PROTECTION: u64 = 17;
    pub const OP_LIST_PROTECTIONS: u64 = 18;
    pub const OP_CAPTURE_HISTORY: u64 = 19;
}

/// PullRequests service trait
pub trait PullRequests {
    type Context;
    /// open (request/response).
    fn open(&self, ctx: &Self::Context, input: OpenPrRequest) -> Result<PullRequest, ServiceError>;
    /// get (request/response).
    fn get(&self, ctx: &Self::Context, input: PrRef) -> Result<PullRequest, ServiceError>;
    /// list (request/response).
    fn list(&self, ctx: &Self::Context, input: ListPrRequest) -> Result<PrPage, ServiceError>;
    /// review (request/response).
    fn review(&self, ctx: &Self::Context, input: ReviewRequest) -> Result<Review, ServiceError>;
    /// comment (request/response).
    fn comment(&self, ctx: &Self::Context, input: CommentRequest) -> Result<Comment, ServiceError>;
    /// merge (request/response).
    fn merge(&self, ctx: &Self::Context, input: MergeRequest) -> Result<MergeResult, ServiceError>;
    /// close (request/response).
    fn close(&self, ctx: &Self::Context, input: PrRef) -> Result<PullRequest, ServiceError>;
    /// reopen (request/response).
    fn reopen(&self, ctx: &Self::Context, input: PrRef) -> Result<PullRequest, ServiceError>;
    /// retarget (request/response).
    fn retarget(
        &self,
        ctx: &Self::Context,
        input: RetargetRequest,
    ) -> Result<PullRequest, ServiceError>;
    /// update (request/response).
    fn update(
        &self,
        ctx: &Self::Context,
        input: UpdatePrRequest,
    ) -> Result<PullRequest, ServiceError>;
    /// list-reviews (request/response).
    fn list_reviews(
        &self,
        ctx: &Self::Context,
        input: PrRef,
    ) -> Result<ListReviewsResponse, ServiceError>;
    /// list-comments (request/response).
    fn list_comments(
        &self,
        ctx: &Self::Context,
        input: PrRef,
    ) -> Result<ListCommentsResponse, ServiceError>;
}

/// Wire-id ordinals for the PullRequests service (transport compact profiles).
pub mod pull_requests_wire_ids {
    pub const SERVICE: u64 = 4;
    pub const OP_OPEN: u64 = 0;
    pub const OP_GET: u64 = 1;
    pub const OP_LIST: u64 = 2;
    pub const OP_REVIEW: u64 = 3;
    pub const OP_COMMENT: u64 = 4;
    pub const OP_MERGE: u64 = 5;
    pub const OP_CLOSE: u64 = 6;
    pub const OP_REOPEN: u64 = 7;
    pub const OP_RETARGET: u64 = 8;
    pub const OP_UPDATE: u64 = 9;
    pub const OP_LIST_REVIEWS: u64 = 10;
    pub const OP_LIST_COMMENTS: u64 = 11;
}

/// Statuses service trait
pub trait Statuses {
    type Context;
    /// set (request/response).
    fn set(
        &self,
        ctx: &Self::Context,
        input: SetStatusRequest,
    ) -> Result<CommitStatus, ServiceError>;
    /// list (request/response).
    fn list(
        &self,
        ctx: &Self::Context,
        input: ListStatusRequest,
    ) -> Result<ListStatusResponse, ServiceError>;
}

/// Wire-id ordinals for the Statuses service (transport compact profiles).
pub mod statuses_wire_ids {
    pub const SERVICE: u64 = 5;
    pub const OP_SET: u64 = 0;
    pub const OP_LIST: u64 = 1;
}

/// Hooks service trait
pub trait Hooks {
    type Context;
    /// create-endpoint (request/response).
    fn create_endpoint(
        &self,
        ctx: &Self::Context,
        input: CreateEndpointRequest,
    ) -> Result<Endpoint, ServiceError>;
    /// list-endpoints (request/response).
    fn list_endpoints(
        &self,
        ctx: &Self::Context,
        input: ListEndpointsRequest,
    ) -> Result<ListEndpointsResponse, ServiceError>;
    /// delete-endpoint (request/response).
    fn delete_endpoint(&self, ctx: &Self::Context, input: EndpointRef) -> Result<Ok, ServiceError>;
    /// list-deliveries (request/response).
    fn list_deliveries(
        &self,
        ctx: &Self::Context,
        input: DeliveryQuery,
    ) -> Result<DeliveryPage, ServiceError>;
    /// redeliver (request/response).
    fn redeliver(&self, ctx: &Self::Context, input: DeliveryRef) -> Result<Ok, ServiceError>;
    /// update-endpoint (request/response).
    fn update_endpoint(
        &self,
        ctx: &Self::Context,
        input: UpdateEndpointRequest,
    ) -> Result<Endpoint, ServiceError>;
}

/// Wire-id ordinals for the Hooks service (transport compact profiles).
pub mod hooks_wire_ids {
    pub const SERVICE: u64 = 6;
    pub const OP_CREATE_ENDPOINT: u64 = 0;
    pub const OP_LIST_ENDPOINTS: u64 = 1;
    pub const OP_DELETE_ENDPOINT: u64 = 2;
    pub const OP_LIST_DELIVERIES: u64 = 3;
    pub const OP_REDELIVER: u64 = 4;
    pub const OP_UPDATE_ENDPOINT: u64 = 5;
}

/// Mirror service trait
pub trait Mirror {
    type Context;
    /// status (request/response).
    fn status(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<MirrorStatus, ServiceError>;
    /// list-blocked (request/response).
    fn list_blocked(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ListBlockedResponse, ServiceError>;
    /// replay-blocked (request/response).
    fn replay_blocked(
        &self,
        ctx: &Self::Context,
        input: ReplayRequest,
    ) -> Result<ReplayResult, ServiceError>;
    /// set-credential (request/response).
    fn set_credential(
        &self,
        ctx: &Self::Context,
        input: SetCredentialRequest,
    ) -> Result<Credential, ServiceError>;
    /// reconcile (request/response).
    fn reconcile(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ReconcileReport, ServiceError>;
    /// pause (request/response).
    fn pause(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Ok, ServiceError>;
    /// resume (request/response).
    fn resume(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Ok, ServiceError>;
    /// import-repo (request/response).
    fn import_repo(&self, ctx: &Self::Context, input: ImportRequest) -> Result<Repo, ServiceError>;
    /// configure (request/response).
    fn configure(
        &self,
        ctx: &Self::Context,
        input: ConfigureMirrorRequest,
    ) -> Result<MirrorStatus, ServiceError>;
    /// list-credentials (request/response).
    fn list_credentials(
        &self,
        ctx: &Self::Context,
        input: ListCredentialsRequest,
    ) -> Result<ListCredentialsResponse, ServiceError>;
    /// remove-credential (request/response).
    fn remove_credential(
        &self,
        ctx: &Self::Context,
        input: CredentialRef,
    ) -> Result<Ok, ServiceError>;
    /// Repositories visible under the organization's `github_owner`, with
    /// the vcsstar repository that mirrors each one when there is one.
    /// Enable one with `import-repo`.
    fn list_github_repos(
        &self,
        ctx: &Self::Context,
        input: ListGithubReposRequest,
    ) -> Result<GithubRepoPage, ServiceError>;
    /// This node's view of GitHub availability and rate limits.
    fn health(&self, ctx: &Self::Context, input: Empty) -> Result<GithubHealth, ServiceError>;
}

/// Wire-id ordinals for the Mirror service (transport compact profiles).
pub mod mirror_wire_ids {
    pub const SERVICE: u64 = 7;
    pub const OP_STATUS: u64 = 0;
    pub const OP_LIST_BLOCKED: u64 = 1;
    pub const OP_REPLAY_BLOCKED: u64 = 2;
    pub const OP_SET_CREDENTIAL: u64 = 3;
    pub const OP_RECONCILE: u64 = 4;
    pub const OP_PAUSE: u64 = 5;
    pub const OP_RESUME: u64 = 6;
    pub const OP_IMPORT_REPO: u64 = 7;
    pub const OP_CONFIGURE: u64 = 8;
    pub const OP_LIST_CREDENTIALS: u64 = 9;
    pub const OP_REMOVE_CREDENTIAL: u64 = 10;
    pub const OP_LIST_GITHUB_REPOS: u64 = 11;
    pub const OP_HEALTH: u64 = 12;
}

/// ServiceAccounts service trait
pub trait ServiceAccounts {
    type Context;
    /// create (request/response).
    fn create(
        &self,
        ctx: &Self::Context,
        input: CreateServiceAccountRequest,
    ) -> Result<ServiceAccount, ServiceError>;
    /// list (request/response).
    fn list(
        &self,
        ctx: &Self::Context,
        input: GetOrgRequest,
    ) -> Result<ListServiceAccountsResponse, ServiceError>;
    /// Disabling refuses every token of the account; enabling lets the
    /// tokens that were not revoked work again.
    fn disable(&self, ctx: &Self::Context, input: ServiceAccountRef) -> Result<Ok, ServiceError>;
    /// enable (request/response).
    fn enable(&self, ctx: &Self::Context, input: ServiceAccountRef) -> Result<Ok, ServiceError>;
    /// create-token (request/response).
    fn create_token(
        &self,
        ctx: &Self::Context,
        input: CreateServiceAccountTokenRequest,
    ) -> Result<ApiToken, ServiceError>;
    /// list-tokens (request/response).
    fn list_tokens(
        &self,
        ctx: &Self::Context,
        input: ServiceAccountRef,
    ) -> Result<ListTokensResponse, ServiceError>;
    /// revoke-token (request/response).
    fn revoke_token(
        &self,
        ctx: &Self::Context,
        input: RevokeServiceAccountTokenRequest,
    ) -> Result<Ok, ServiceError>;
}

/// Wire-id ordinals for the ServiceAccounts service (transport compact profiles).
pub mod service_accounts_wire_ids {
    pub const SERVICE: u64 = 11;
    pub const OP_CREATE: u64 = 0;
    pub const OP_LIST: u64 = 1;
    pub const OP_DISABLE: u64 = 2;
    pub const OP_ENABLE: u64 = 3;
    pub const OP_CREATE_TOKEN: u64 = 4;
    pub const OP_LIST_TOKENS: u64 = 5;
    pub const OP_REVOKE_TOKEN: u64 = 6;
}

/// Federation service trait
pub trait Federation {
    type Context;
    /// handshake (request/response).
    fn handshake(&self, ctx: &Self::Context, input: FedHello) -> Result<FedHelloAck, ServiceError>;
    /// verify-head (request/response).
    fn verify_head(
        &self,
        ctx: &Self::Context,
        input: VerifyHeadRequest,
    ) -> Result<HeadInfo, ServiceError>;
    /// head-info (request/response).
    fn head_info(&self, ctx: &Self::Context, input: HeadQuery) -> Result<HeadInfo, ServiceError>;
    /// repo-info (request/response).
    fn repo_info(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Repo, ServiceError>;
}

/// Wire-id ordinals for the Federation service (transport compact profiles).
pub mod federation_wire_ids {
    pub const SERVICE: u64 = 8;
    pub const OP_HANDSHAKE: u64 = 0;
    pub const OP_VERIFY_HEAD: u64 = 1;
    pub const OP_HEAD_INFO: u64 = 2;
    pub const OP_REPO_INFO: u64 = 3;
}

/// EventStream service trait
pub trait EventStream {
    type Context;
    /// subscribe (channel inbound (bidirectional)).
    fn subscribe(&self, ctx: &Self::Context, msg: StreamControl) -> Result<(), ServiceError>;
}

/// Wire-id ordinals for the EventStream service (transport compact profiles).
pub mod event_stream_wire_ids {
    pub const SERVICE: u64 = 9;
    pub const OP_SUBSCRIBE: u64 = 0;
}

/// Decode one inbound channel frame for EventStream (with the generated
/// per-type codec) and dispatch to the matching trait method. The implementer
/// feeds raw bytes from its connection here; we never own the wire.
pub fn route_event_stream_channel<H>(
    _handlers: &H,
    _ctx: &H::Context,
    method: &str,
    _bytes: &[u8],
) -> Result<(), ServiceError>
where
    H: EventStream,
{
    Err(ServiceError {
        code: 404,
        message: format!("unknown channel {method}"),
    })
}

/// Decode one inbound channel frame for EventStream by its
/// `@wire-id` ordinal (compact transport profile) and dispatch to the
/// matching trait method. The verbose-profile twin is
/// `route_event_stream_channel`; the host calls whichever matches the profile
/// negotiated on the wire.
pub fn route_event_stream_channel_compact<H>(
    _handlers: &H,
    _ctx: &H::Context,
    op: u64,
    _bytes: &[u8],
) -> Result<(), ServiceError>
where
    H: EventStream,
{
    Err(ServiceError {
        code: 404,
        message: format!("unknown channel ordinal {op}"),
    })
}
