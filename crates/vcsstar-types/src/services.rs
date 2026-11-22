//! Generated service traits from CSIL specification

use super::codec::*;
use super::types::*;

#[derive(Debug)]
pub enum ChannelRouteError {
    UnknownMethod(String),
    UnknownOrdinal(u64),
    Decode(String),
    Handler(ServiceError),
}

impl std::fmt::Display for ChannelRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMethod(method) => write!(f, "unknown channel {method}"),
            Self::UnknownOrdinal(op) => write!(f, "unknown channel ordinal {op}"),
            Self::Decode(message) => write!(f, "invalid channel payload: {message}"),
            Self::Handler(error) => write!(f, "channel handler failed: {error:?}"),
        }
    }
}

impl std::error::Error for ChannelRouteError {}

impl From<CsilCborError> for ChannelRouteError {
    fn from(error: CsilCborError) -> Self {
        Self::Decode(error.0)
    }
}

/// Meta service trait
pub trait Meta {
    type Context;
    #[allow(clippy::result_large_err)]
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
    #[allow(clippy::result_large_err)]
    /// begin-login (request/response).
    fn begin_login(
        &self,
        ctx: &Self::Context,
        input: BeginLoginRequest,
    ) -> Result<BeginLoginResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// complete-login (request/response).
    fn complete_login(
        &self,
        ctx: &Self::Context,
        input: CompleteLoginRequest,
    ) -> Result<Session, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// create-token (request/response).
    fn create_token(
        &self,
        ctx: &Self::Context,
        input: CreateTokenRequest,
    ) -> Result<ApiToken, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// revoke-token (request/response).
    fn revoke_token(
        &self,
        ctx: &Self::Context,
        input: RevokeTokenRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// whoami (request/response).
    fn whoami(&self, ctx: &Self::Context, input: Empty) -> Result<WhoAmI, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// begin-device-login (request/response).
    fn begin_device_login(
        &self,
        ctx: &Self::Context,
        input: BeginDeviceRequest,
    ) -> Result<DeviceCode, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// poll-device-login (request/response).
    fn poll_device_login(
        &self,
        ctx: &Self::Context,
        input: DevicePollRequest,
    ) -> Result<DevicePollResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// exchange-act-as-credential (request/response).
    fn exchange_act_as_credential(
        &self,
        ctx: &Self::Context,
        input: ActAsExchangeRequest,
    ) -> Result<ScopedBearer, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// scope-set (request/response).
    fn scope_set(
        &self,
        ctx: &Self::Context,
        input: ScopeSetRequest,
    ) -> Result<SignedActAsScopeSet, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Browser side of the device flow. Needs a web session.
    fn approve_device_login(
        &self,
        ctx: &Self::Context,
        input: DeviceApproveRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// register-key (request/response).
    fn register_key(
        &self,
        ctx: &Self::Context,
        input: RegisterKeyRequest,
    ) -> Result<SshKeyInfo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-keys (request/response).
    fn list_keys(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<ListKeysResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// revoke-key (request/response).
    fn revoke_key(&self, ctx: &Self::Context, input: KeyRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-tokens (request/response).
    fn list_tokens(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<ListTokensResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// logout (request/response).
    fn logout(&self, ctx: &Self::Context, input: Empty) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// GitHub account link of the caller (see GithubLink).
    fn github_link_status(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<GithubLink, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-github-token (request/response).
    fn set_github_token(
        &self,
        ctx: &Self::Context,
        input: SetGithubTokenRequest,
    ) -> Result<GithubLink, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// unlink-github (request/response).
    fn unlink_github(&self, ctx: &Self::Context, input: Empty) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// The caller's display preferences (defaults when none are saved).
    fn get_preferences(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<Preferences, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Save the given fields; absent fields keep their value.
    fn set_preferences(
        &self,
        ctx: &Self::Context,
        input: SetPreferencesRequest,
    ) -> Result<Preferences, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// local-login (request/response).
    fn local_login(
        &self,
        ctx: &Self::Context,
        input: LocalLoginRequest,
    ) -> Result<Session, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// local-account (request/response).
    fn local_account(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<LocalAccount, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// change-local-password (request/response).
    fn change_local_password(
        &self,
        ctx: &Self::Context,
        input: ChangeLocalPasswordRequest,
    ) -> Result<Ok, ServiceError>;
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
    pub const OP_GET_PREFERENCES: u64 = 18;
    pub const OP_SET_PREFERENCES: u64 = 19;
    pub const OP_LOCAL_LOGIN: u64 = 20;
    pub const OP_LOCAL_ACCOUNT: u64 = 21;
    pub const OP_CHANGE_LOCAL_PASSWORD: u64 = 22;
}

/// Orgs service trait
pub trait Orgs {
    type Context;
    #[allow(clippy::result_large_err)]
    /// create (request/response).
    fn create(&self, ctx: &Self::Context, input: CreateOrgRequest) -> Result<Org, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// get (request/response).
    fn get(&self, ctx: &Self::Context, input: GetOrgRequest) -> Result<Org, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list (request/response).
    fn list(&self, ctx: &Self::Context, input: ListOrgsRequest) -> Result<OrgPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// update (request/response).
    fn update(&self, ctx: &Self::Context, input: UpdateOrgRequest) -> Result<Org, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-member (request/response).
    fn set_member(
        &self,
        ctx: &Self::Context,
        input: SetOrgMemberRequest,
    ) -> Result<OrgMember, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// remove-member (request/response).
    fn remove_member(
        &self,
        ctx: &Self::Context,
        input: RemoveOrgMemberRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-members (request/response).
    fn list_members(
        &self,
        ctx: &Self::Context,
        input: ListOrgMembersRequest,
    ) -> Result<OrgMemberPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// create-team (request/response).
    fn create_team(
        &self,
        ctx: &Self::Context,
        input: CreateTeamRequest,
    ) -> Result<Team, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// delete-team (request/response).
    fn delete_team(&self, ctx: &Self::Context, input: TeamRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-team-member (request/response).
    fn set_team_member(
        &self,
        ctx: &Self::Context,
        input: SetTeamMemberRequest,
    ) -> Result<Team, ServiceError>;
    #[allow(clippy::result_large_err)]
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
    #[allow(clippy::result_large_err)]
    /// create (request/response).
    fn create(&self, ctx: &Self::Context, input: CreateRepoRequest) -> Result<Repo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// get (request/response).
    fn get(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Repo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list (request/response).
    fn list(&self, ctx: &Self::Context, input: ListReposRequest) -> Result<RepoPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// update (request/response).
    fn update(&self, ctx: &Self::Context, input: UpdateRepoRequest) -> Result<Repo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// fork (request/response).
    fn fork(&self, ctx: &Self::Context, input: ForkRequest) -> Result<Repo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-refs (request/response).
    fn list_refs(
        &self,
        ctx: &Self::Context,
        input: ListRefsRequest,
    ) -> Result<ListRefsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// log (request/response).
    fn log(&self, ctx: &Self::Context, input: LogRequest) -> Result<CommitPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// tree (request/response).
    fn tree(&self, ctx: &Self::Context, input: TreeRequest) -> Result<TreeResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// blob (request/response).
    fn blob(&self, ctx: &Self::Context, input: BlobRequest) -> Result<BlobResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// diff (request/response).
    fn diff(&self, ctx: &Self::Context, input: DiffRequest) -> Result<DiffResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// delete (request/response).
    fn delete(&self, ctx: &Self::Context, input: DeleteRepoRequest) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// update-ref (request/response).
    fn update_ref(
        &self,
        ctx: &Self::Context,
        input: UpdateRefRequest,
    ) -> Result<RefInfo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// get-commit (request/response).
    fn get_commit(
        &self,
        ctx: &Self::Context,
        input: GetCommitRequest,
    ) -> Result<Commit, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-grant (request/response).
    fn set_grant(&self, ctx: &Self::Context, input: SetGrantRequest)
        -> Result<Grant, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// remove-grant (request/response).
    fn remove_grant(
        &self,
        ctx: &Self::Context,
        input: RemoveGrantRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-grants (request/response).
    fn list_grants(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ListGrantsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-protection (request/response).
    fn set_protection(
        &self,
        ctx: &Self::Context,
        input: SetProtectionRequest,
    ) -> Result<BranchProtection, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// remove-protection (request/response).
    fn remove_protection(
        &self,
        ctx: &Self::Context,
        input: RemoveProtectionRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-protections (request/response).
    fn list_protections(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ListProtectionsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Pack every object reachable from the current refs that no live segment
    /// holds, and upload it as one new segment. Use it for history that was
    /// written before the object store existed. Repeatable; no-op when covered.
    fn capture_history(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<CaptureReport, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Direct forks of the repository that the caller can see, on this
    /// instance. A pull request into the repository may come from one.
    fn list_forks(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<RepoPage, ServiceError>;
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
    pub const OP_LIST_FORKS: u64 = 20;
}

/// PullRequests service trait
pub trait PullRequests {
    type Context;
    #[allow(clippy::result_large_err)]
    /// open (request/response).
    fn open(&self, ctx: &Self::Context, input: OpenPrRequest) -> Result<PullRequest, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// get (request/response).
    fn get(&self, ctx: &Self::Context, input: PrRef) -> Result<PullRequest, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list (request/response).
    fn list(&self, ctx: &Self::Context, input: ListPrRequest) -> Result<PrPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// review (request/response).
    fn review(&self, ctx: &Self::Context, input: ReviewRequest) -> Result<Review, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// comment (request/response).
    fn comment(&self, ctx: &Self::Context, input: CommentRequest) -> Result<Comment, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// merge (request/response).
    fn merge(&self, ctx: &Self::Context, input: MergeRequest) -> Result<MergeResult, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// close (request/response).
    fn close(&self, ctx: &Self::Context, input: PrRef) -> Result<PullRequest, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// reopen (request/response).
    fn reopen(&self, ctx: &Self::Context, input: PrRef) -> Result<PullRequest, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// retarget (request/response).
    fn retarget(
        &self,
        ctx: &Self::Context,
        input: RetargetRequest,
    ) -> Result<PullRequest, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// update (request/response).
    fn update(
        &self,
        ctx: &Self::Context,
        input: UpdatePrRequest,
    ) -> Result<PullRequest, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-reviews (request/response).
    fn list_reviews(
        &self,
        ctx: &Self::Context,
        input: PrRef,
    ) -> Result<ListReviewsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-comments (request/response).
    fn list_comments(
        &self,
        ctx: &Self::Context,
        input: PrRef,
    ) -> Result<ListCommentsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// edit-comment (request/response).
    fn edit_comment(
        &self,
        ctx: &Self::Context,
        input: EditCommentRequest,
    ) -> Result<Comment, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// delete-comment (request/response).
    fn delete_comment(
        &self,
        ctx: &Self::Context,
        input: DeleteCommentRequest,
    ) -> Result<Ok, ServiceError>;
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
    pub const OP_EDIT_COMMENT: u64 = 12;
    pub const OP_DELETE_COMMENT: u64 = 13;
}

/// Statuses service trait
pub trait Statuses {
    type Context;
    #[allow(clippy::result_large_err)]
    /// set (request/response).
    fn set(
        &self,
        ctx: &Self::Context,
        input: SetStatusRequest,
    ) -> Result<CommitStatus, ServiceError>;
    #[allow(clippy::result_large_err)]
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
    #[allow(clippy::result_large_err)]
    /// create-endpoint (request/response).
    fn create_endpoint(
        &self,
        ctx: &Self::Context,
        input: CreateEndpointRequest,
    ) -> Result<Endpoint, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-endpoints (request/response).
    fn list_endpoints(
        &self,
        ctx: &Self::Context,
        input: ListEndpointsRequest,
    ) -> Result<ListEndpointsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// delete-endpoint (request/response).
    fn delete_endpoint(&self, ctx: &Self::Context, input: EndpointRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-deliveries (request/response).
    fn list_deliveries(
        &self,
        ctx: &Self::Context,
        input: DeliveryQuery,
    ) -> Result<DeliveryPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// redeliver (request/response).
    fn redeliver(&self, ctx: &Self::Context, input: DeliveryRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
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
    #[allow(clippy::result_large_err)]
    /// status (request/response).
    fn status(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<MirrorStatus, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-blocked (request/response).
    fn list_blocked(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ListBlockedResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// replay-blocked (request/response).
    fn replay_blocked(
        &self,
        ctx: &Self::Context,
        input: ReplayRequest,
    ) -> Result<ReplayResult, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-credential (request/response).
    fn set_credential(
        &self,
        ctx: &Self::Context,
        input: SetCredentialRequest,
    ) -> Result<Credential, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// reconcile (request/response).
    fn reconcile(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<ReconcileReport, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// pause (request/response).
    fn pause(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// resume (request/response).
    fn resume(&self, ctx: &Self::Context, input: GetRepoRequest) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Make the repository and queue its import. The answer comes before
    /// the branches are fetched; `status` reports `import_state`.
    fn import_repo(&self, ctx: &Self::Context, input: ImportRequest) -> Result<Repo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// configure (request/response).
    fn configure(
        &self,
        ctx: &Self::Context,
        input: ConfigureMirrorRequest,
    ) -> Result<MirrorStatus, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-credentials (request/response).
    fn list_credentials(
        &self,
        ctx: &Self::Context,
        input: ListCredentialsRequest,
    ) -> Result<ListCredentialsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// remove-credential (request/response).
    fn remove_credential(
        &self,
        ctx: &Self::Context,
        input: CredentialRef,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Repositories visible under the organization's `github_owner`, with
    /// the vcsstar repository that mirrors each one when there is one.
    /// Enable one with `import-repo`.
    fn list_github_repos(
        &self,
        ctx: &Self::Context,
        input: ListGithubReposRequest,
    ) -> Result<GithubRepoPage, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// This node's view of GitHub availability and rate limits.
    fn health(&self, ctx: &Self::Context, input: Empty) -> Result<GithubHealth, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Start the history import of the repository's GitHub pull requests
    /// again from the first one. A mirrored repository imports its history
    /// by itself; this is for a repository whose import must run again.
    fn backfill(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<MirrorStatus, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Where the instance's GitHub Apps are installed, with the vcsstar
    /// organizations whose `github_owner` is each owner.
    fn list_github_installations(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<GithubInstallationList, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// import-all (request/response).
    fn import_all(
        &self,
        ctx: &Self::Context,
        input: ImportAllRequest,
    ) -> Result<ImportAllResult, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-schedule (request/response).
    fn set_schedule(
        &self,
        ctx: &Self::Context,
        input: SetMirrorScheduleRequest,
    ) -> Result<MirrorStatus, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-connections (request/response).
    fn list_connections(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<SyncConnections, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// connect (request/response).
    fn connect(
        &self,
        ctx: &Self::Context,
        input: ConnectRemoteRequest,
    ) -> Result<SyncConnection, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// update-connection (request/response).
    fn update_connection(
        &self,
        ctx: &Self::Context,
        input: UpdateConnectionRequest,
    ) -> Result<SyncConnection, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// sync-now (request/response).
    fn sync_now(&self, ctx: &Self::Context, input: SyncConnectionRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// webhook-secret (request/response).
    fn webhook_secret(
        &self,
        ctx: &Self::Context,
        input: SyncConnectionRef,
    ) -> Result<SyncWebhookSecret, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// get-policy (request/response).
    fn get_policy(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<SyncPolicy, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-policy (request/response).
    fn set_policy(
        &self,
        ctx: &Self::Context,
        input: SetSyncPolicyRequest,
    ) -> Result<SyncPolicy, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-conflicts (request/response).
    fn list_conflicts(
        &self,
        ctx: &Self::Context,
        input: GetRepoRequest,
    ) -> Result<SyncConflicts, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// resolve-conflict (request/response).
    fn resolve_conflict(
        &self,
        ctx: &Self::Context,
        input: ResolveSyncConflictRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// link-provider (request/response).
    fn link_provider(
        &self,
        ctx: &Self::Context,
        input: LinkProviderRequest,
    ) -> Result<ProviderLink, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-provider-links (request/response).
    fn list_provider_links(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<ProviderLinks, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// unlink-provider (request/response).
    fn unlink_provider(
        &self,
        ctx: &Self::Context,
        input: UnlinkProviderRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-sync-records (request/response).
    fn list_sync_records(
        &self,
        ctx: &Self::Context,
        input: SyncRecordsRequest,
    ) -> Result<SyncRecords, ServiceError>;
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
    pub const OP_BACKFILL: u64 = 13;
    pub const OP_LIST_GITHUB_INSTALLATIONS: u64 = 14;
    pub const OP_IMPORT_ALL: u64 = 15;
    pub const OP_SET_SCHEDULE: u64 = 16;
    pub const OP_LIST_CONNECTIONS: u64 = 17;
    pub const OP_CONNECT: u64 = 18;
    pub const OP_UPDATE_CONNECTION: u64 = 19;
    pub const OP_SYNC_NOW: u64 = 20;
    pub const OP_WEBHOOK_SECRET: u64 = 21;
    pub const OP_GET_POLICY: u64 = 22;
    pub const OP_SET_POLICY: u64 = 23;
    pub const OP_LIST_CONFLICTS: u64 = 24;
    pub const OP_RESOLVE_CONFLICT: u64 = 25;
    pub const OP_LINK_PROVIDER: u64 = 26;
    pub const OP_LIST_PROVIDER_LINKS: u64 = 27;
    pub const OP_UNLINK_PROVIDER: u64 = 28;
    pub const OP_LIST_SYNC_RECORDS: u64 = 29;
}

/// Instance service trait
pub trait Instance {
    type Context;
    #[allow(clippy::result_large_err)]
    /// Every instance role. Instance admins only.
    fn list_roles(
        &self,
        ctx: &Self::Context,
        input: Empty,
    ) -> Result<InstanceRoleList, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// set-role (request/response).
    fn set_role(
        &self,
        ctx: &Self::Context,
        input: SetInstanceRoleRequest,
    ) -> Result<InstanceRoleGrant, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// remove-role (request/response).
    fn remove_role(
        &self,
        ctx: &Self::Context,
        input: RemoveInstanceRoleRequest,
    ) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// create-local-account (request/response).
    fn create_local_account(
        &self,
        ctx: &Self::Context,
        input: CreateLocalAccountRequest,
    ) -> Result<LocalAccount, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// reset-local-password (request/response).
    fn reset_local_password(
        &self,
        ctx: &Self::Context,
        input: ResetLocalPasswordRequest,
    ) -> Result<Ok, ServiceError>;
}

/// Wire-id ordinals for the Instance service (transport compact profiles).
pub mod instance_wire_ids {
    pub const SERVICE: u64 = 12;
    pub const OP_LIST_ROLES: u64 = 0;
    pub const OP_SET_ROLE: u64 = 1;
    pub const OP_REMOVE_ROLE: u64 = 2;
    pub const OP_CREATE_LOCAL_ACCOUNT: u64 = 3;
    pub const OP_RESET_LOCAL_PASSWORD: u64 = 4;
}

/// ServiceAccounts service trait
pub trait ServiceAccounts {
    type Context;
    #[allow(clippy::result_large_err)]
    /// create (request/response).
    fn create(
        &self,
        ctx: &Self::Context,
        input: CreateServiceAccountRequest,
    ) -> Result<ServiceAccount, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list (request/response).
    fn list(
        &self,
        ctx: &Self::Context,
        input: GetOrgRequest,
    ) -> Result<ListServiceAccountsResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// Disabling refuses every token of the account; enabling lets the
    /// tokens that were not revoked work again.
    fn disable(&self, ctx: &Self::Context, input: ServiceAccountRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// enable (request/response).
    fn enable(&self, ctx: &Self::Context, input: ServiceAccountRef) -> Result<Ok, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// create-token (request/response).
    fn create_token(
        &self,
        ctx: &Self::Context,
        input: CreateServiceAccountTokenRequest,
    ) -> Result<ApiToken, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// list-tokens (request/response).
    fn list_tokens(
        &self,
        ctx: &Self::Context,
        input: ServiceAccountRef,
    ) -> Result<ListTokensResponse, ServiceError>;
    #[allow(clippy::result_large_err)]
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
    #[allow(clippy::result_large_err)]
    /// handshake (request/response).
    fn handshake(&self, ctx: &Self::Context, input: FedHello) -> Result<FedHelloAck, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// verify-head (request/response).
    fn verify_head(
        &self,
        ctx: &Self::Context,
        input: VerifyHeadRequest,
    ) -> Result<HeadInfo, ServiceError>;
    #[allow(clippy::result_large_err)]
    /// head-info (request/response).
    fn head_info(&self, ctx: &Self::Context, input: HeadQuery) -> Result<HeadInfo, ServiceError>;
    #[allow(clippy::result_large_err)]
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
) -> Result<(), ChannelRouteError>
where
    H: EventStream,
{
    Err(ChannelRouteError::UnknownMethod(method.to_owned()))
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
) -> Result<(), ChannelRouteError>
where
    H: EventStream,
{
    Err(ChannelRouteError::UnknownOrdinal(op))
}
