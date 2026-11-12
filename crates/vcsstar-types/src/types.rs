//! Generated types from CSIL specification

#![allow(non_camel_case_types, clippy::large_enum_variant)]

pub type Timestamp = i64;

#[derive(Debug, Clone, PartialEq)]
pub struct Ctx {
    pub request_id: String,
    pub deadline_ms: Option<u64>,
    pub contract_version: Option<String>,
    pub contract_digest: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceError {
    pub code: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ok {
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Empty {
    pub ctx: Ctx,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub domain: String,
    pub handle: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepoRef {
    pub instance: Option<String>,
    pub org: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrgRef {
    pub org: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrgRepoRef {
    pub org: String,
    pub repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub cursor: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerInfo {
    pub version: String,
    pub contract_version: String,
    pub contract_digest: Vec<u8>,
    pub features: Vec<String>,
    pub instance_domain: String,
    pub public_url: String,
    pub instance_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeginLoginRequest {
    pub ctx: Ctx,
    pub identity_input: String,
    pub return_to: Option<String>,
    pub device_user_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeginLoginResponse {
    pub redirect_url: String,
    pub attempt_token: String,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompleteLoginRequest {
    pub ctx: Ctx,
    pub attempt_token: String,
    pub encrypted_token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub session_token: String,
    pub identity: Identity,
    pub expires_at: Timestamp,
    pub return_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTokenRequest {
    pub ctx: Ctx,
    pub name: String,
    pub scopes: Vec<String>,
    pub client_label: Option<String>,
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub client_label: Option<String>,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevokeTokenRequest {
    pub ctx: Ctx,
    pub token_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListTokensResponse {
    pub tokens: Vec<ApiToken>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    pub diff_layout: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetPreferencesRequest {
    pub ctx: Ctx,
    pub diff_layout: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhoAmI {
    pub identity: Identity,
    pub scopes: Vec<String>,
    pub credential_kind: String,
    pub acting_via: Option<String>,
    pub expires_at: Option<Timestamp>,
    pub instance_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeginDeviceRequest {
    pub ctx: Ctx,
    pub client_label: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at: Timestamp,
    pub poll_interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevicePollRequest {
    pub ctx: Ctx,
    pub device_code: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevicePollResponse {
    pub pending: bool,
    pub token: Option<ApiToken>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceApproveRequest {
    pub ctx: Ctx,
    pub user_code: String,
    pub approve: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActAsExchangeRequest {
    pub ctx: Ctx,
    pub credential_cbor: Vec<u8>,
    pub requested_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopedBearer {
    pub bearer: String,
    pub scopes: Vec<String>,
    pub expires_at: Timestamp,
    pub subject: Identity,
    pub acting_via: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeSetRequest {
    pub ctx: Ctx,
    pub grantee_cbor: Vec<u8>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignedActAsScopeSet {
    pub scope_set_cbor: Vec<u8>,
    pub scopes: Vec<String>,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SshKeyInfo {
    pub key_id: String,
    pub public_key: String,
    pub usage: String,
    pub status: String,
    pub application_id: Option<String>,
    pub instance_id: Option<String>,
    pub attested_until: Option<Timestamp>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegisterKeyRequest {
    pub ctx: Ctx,
    pub public_key: String,
    pub usage: String,
    pub application_id: Option<String>,
    pub instance_id: Option<String>,
    pub lk_key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyRef {
    pub ctx: Ctx,
    pub key_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListKeysResponse {
    pub keys: Vec<SshKeyInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubLink {
    pub linked: bool,
    pub kind: Option<String>,
    pub login: Option<String>,
    pub github_id: Option<u64>,
    pub linked_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub broken_reason: Option<String>,
    pub start_url: Option<String>,
    pub app_slug: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetGithubTokenRequest {
    pub ctx: Ctx,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Org {
    pub name: String,
    pub display_name: Option<String>,
    pub github_owner: Option<String>,
    pub created_at: Timestamp,
    pub viewer_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateOrgRequest {
    pub ctx: Ctx,
    pub name: String,
    pub display_name: Option<String>,
    pub github_owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetOrgRequest {
    pub ctx: Ctx,
    pub org: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListOrgsRequest {
    pub ctx: Ctx,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrgPage {
    pub orgs: Vec<Org>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateOrgRequest {
    pub ctx: Ctx,
    pub org: String,
    pub display_name: Option<String>,
    pub github_owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrgMember {
    pub identity: Identity,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetOrgMemberRequest {
    pub ctx: Ctx,
    pub org: String,
    pub identity: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoveOrgMemberRequest {
    pub ctx: Ctx,
    pub org: String,
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListOrgMembersRequest {
    pub ctx: Ctx,
    pub org: String,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrgMemberPage {
    pub members: Vec<OrgMember>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub org: String,
    pub name: String,
    pub members: Vec<Identity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTeamRequest {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeamRef {
    pub ctx: Ctx,
    pub org: String,
    pub team: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetTeamMemberRequest {
    pub ctx: Ctx,
    pub org: String,
    pub team: String,
    pub identity: String,
    pub member: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListTeamsResponse {
    pub teams: Vec<Team>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub org: String,
    pub name: String,
    pub vcs_type: String,
    pub visibility: String,
    pub default_branch: String,
    pub description: Option<String>,
    pub fork_of: Option<RepoRef>,
    pub mirror_enabled: bool,
    pub archived: bool,
    pub created_at: Timestamp,
    pub pushed_at: Option<Timestamp>,
    pub https_url: String,
    pub ssh_url: Option<String>,
    pub viewer_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateRepoRequest {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
    pub vcs_type: Option<String>,
    pub visibility: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetRepoRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListReposRequest {
    pub ctx: Ctx,
    pub org: Option<String>,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepoPage {
    pub repos: Vec<Repo>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateRepoRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub description: Option<String>,
    pub default_branch: Option<String>,
    pub visibility: Option<String>,
    pub archived: Option<bool>,
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteRepoRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub confirm_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForkRequest {
    pub ctx: Ctx,
    pub source: RepoRef,
    pub org: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefInfo {
    pub name: String,
    pub target: String,
    pub peeled: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListRefsRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureReport {
    pub repo: RepoRef,
    pub uploaded: bool,
    pub objects: u64,
    pub bytes: u64,
    pub segment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListRefsResponse {
    pub refs: Vec<RefInfo>,
    pub refs_version: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateRefRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub name: String,
    pub expected: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub status: String,
    pub signer: Option<Identity>,
    pub key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub tree: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: Timestamp,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at: Timestamp,
    pub message: String,
    pub signature: Signature,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub rev: String,
    pub path: Option<String>,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitPage {
    pub commits: Vec<Commit>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetCommitRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub rev: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeEntry {
    pub name: String,
    pub kind: String,
    pub id: String,
    pub mode: u64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub rev: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeResponse {
    pub entries: Vec<TreeEntry>,
    pub rev_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlobRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub rev: String,
    pub path: String,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlobResponse {
    pub id: String,
    pub size: u64,
    pub offset: u64,
    pub content: Vec<u8>,
    pub truncated: bool,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub base: String,
    pub head: String,
    pub path: Option<String>,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    pub status: String,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffResponse {
    pub files: Vec<FileDiff>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub grantee_kind: String,
    pub grantee: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetGrantRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub grant: Grant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoveGrantRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub grantee_kind: String,
    pub grantee: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListGrantsResponse {
    pub grants: Vec<Grant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BranchProtection {
    pub ref_pattern: String,
    pub required_approvals: u64,
    pub dismiss_stale: bool,
    pub required_signature_level: String,
    pub required_checks: Vec<String>,
    pub allowed_merge_kinds: Vec<String>,
    pub allow_force_push: bool,
    pub allow_delete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetProtectionRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub protection: BranchProtection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoveProtectionRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub ref_pattern: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListProtectionsResponse {
    pub protections: Vec<BranchProtection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrHead {
    pub repo: RepoRef,
    pub r#ref: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullRequest {
    pub repo: RepoRef,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub author: Identity,
    pub state: String,
    pub base_ref: String,
    pub head: PrHead,
    pub base_sha: String,
    pub head_sha: String,
    pub merge_base_sha: Option<String>,
    pub mergeable: String,
    pub merge_kind: Option<String>,
    pub merged_commit: Option<String>,
    pub merged_by: Option<Identity>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub closed_at: Option<Timestamp>,
    pub github_number: Option<u64>,
    pub github_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrRef {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenPrRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub title: String,
    pub body: Option<String>,
    pub base_ref: String,
    pub head: PrHead,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListPrRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub state: Option<String>,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrPage {
    pub pull_requests: Vec<PullRequest>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub number: u64,
    pub reviewer: Identity,
    pub state: String,
    pub head_sha: String,
    pub body: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub number: u64,
    pub state: String,
    pub body: Option<String>,
    pub head_sha: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub id: String,
    pub number: u64,
    pub author: Identity,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub commit_sha: Option<String>,
    pub created_at: Timestamp,
    pub edited_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommentRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub number: u64,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListCommentsResponse {
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListReviewsResponse {
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub number: u64,
    pub expected_head_sha: String,
    pub merge_kind: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    pub pull_request: PullRequest,
    pub merged_commit: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetargetRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub number: u64,
    pub base_ref: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdatePrRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub number: u64,
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitStatus {
    pub sha: String,
    pub context: String,
    pub state: String,
    pub target_url: Option<String>,
    pub description: Option<String>,
    pub reporter: Identity,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetStatusRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub sha: String,
    pub context: String,
    pub state: String,
    pub target_url: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListStatusRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListStatusResponse {
    pub statuses: Vec<CommitStatus>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub id: String,
    pub org: String,
    pub repo: Option<String>,
    pub kind: String,
    pub url: Option<String>,
    pub corndogs_queue: Option<String>,
    pub event_filter: Vec<String>,
    pub content_type: String,
    pub active: bool,
    pub unordered: bool,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateEndpointRequest {
    pub ctx: Ctx,
    pub org: String,
    pub repo: Option<String>,
    pub kind: String,
    pub url: Option<String>,
    pub corndogs_queue: Option<String>,
    pub event_filter: Vec<String>,
    pub content_type: Option<String>,
    pub secret: Option<String>,
    pub unordered: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListEndpointsRequest {
    pub ctx: Ctx,
    pub target: OrgRepoRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListEndpointsResponse {
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EndpointRef {
    pub ctx: Ctx,
    pub endpoint_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateEndpointRequest {
    pub ctx: Ctx,
    pub endpoint_id: String,
    pub url: Option<String>,
    pub event_filter: Option<Vec<String>>,
    pub active: Option<bool>,
    pub secret: Option<String>,
    pub unordered: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub id: String,
    pub endpoint_id: String,
    pub event_lane: String,
    pub event_seq: u64,
    pub event_kind: String,
    pub state: String,
    pub attempts: u64,
    pub next_attempt_at: Option<Timestamp>,
    pub last_status: Option<u64>,
    pub last_error: Option<String>,
    pub delivered_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryQuery {
    pub ctx: Ctx,
    pub endpoint_id: String,
    pub state: Option<String>,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryPage {
    pub deliveries: Vec<Delivery>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRef {
    pub ctx: Ctx,
    pub delivery_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirrorStatus {
    pub repo: RepoRef,
    pub github_repo: Option<String>,
    pub mode: String,
    pub paused: bool,
    pub cursor_seq: u64,
    pub pending: u64,
    pub blocked: u64,
    pub last_success_at: Option<Timestamp>,
    pub last_error: Option<String>,
    pub last_full_reconcile_at: Option<Timestamp>,
    pub last_pull_sync_at: Option<Timestamp>,
    pub backfill_state: Option<String>,
    pub backfill_pulls: Option<u64>,
    pub backfill_started_at: Option<Timestamp>,
    pub backfill_done_at: Option<Timestamp>,
    pub import_state: Option<String>,
    pub pull_sync_interval_s: u64,
    pub ref_reconcile_interval_s: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetMirrorScheduleRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub pull_sync_interval_s: Option<u64>,
    pub ref_reconcile_interval_s: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportAllRequest {
    pub ctx: Ctx,
    pub org: String,
    pub mode: Option<String>,
    pub skip_archived: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportAllResult {
    pub queued: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockedAction {
    pub id: String,
    pub event_seq: u64,
    pub action_kind: String,
    pub object_key: String,
    pub error: String,
    pub blocked_at: Timestamp,
    pub credential_purpose: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListBlockedResponse {
    pub actions: Vec<BlockedAction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub action_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayResult {
    pub requeued: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Credential {
    pub id: String,
    pub org: String,
    pub repo: Option<String>,
    pub provider: String,
    pub purpose: String,
    pub auth_method: String,
    pub host: String,
    pub secret_ref: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetCredentialRequest {
    pub ctx: Ctx,
    pub org: String,
    pub repo: Option<String>,
    pub provider: String,
    pub purpose: String,
    pub auth_method: String,
    pub host: String,
    pub secret_ref: String,
    pub secret: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListCredentialsRequest {
    pub ctx: Ctx,
    pub target: OrgRepoRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListCredentialsResponse {
    pub credentials: Vec<Credential>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialRef {
    pub ctx: Ctx,
    pub credential_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigureMirrorRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub github_repo: String,
    pub mode: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileReport {
    pub repo: RepoRef,
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub refs_checked: u64,
    pub refs_pushed: u64,
    pub drift_refs: Vec<String>,
    pub error: Option<String>,
    pub pulls_checked: Option<u64>,
    pub pulls_changed: Option<u64>,
    pub pulls_complete: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRequest {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
    pub github_repo: String,
    pub mode: Option<String>,
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListGithubReposRequest {
    pub ctx: Ctx,
    pub org: String,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubRepoInfo {
    pub full_name: String,
    pub description: Option<String>,
    pub private: bool,
    pub archived: bool,
    pub fork: bool,
    pub default_branch: String,
    pub html_url: String,
    pub pushed_at: Option<Timestamp>,
    pub mirrored_as: Option<String>,
    pub import_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubInstallation {
    pub app_id: String,
    pub app_slug: Option<String>,
    pub installation_id: u64,
    pub owner: String,
    pub owner_type: String,
    pub repository_selection: String,
    pub vcsstar_orgs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubInstallationList {
    pub installations: Vec<GithubInstallation>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubRepoPage {
    pub repos: Vec<GithubRepoInfo>,
    pub next_cursor: Option<String>,
    pub source: String,
    pub installation_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaneHealth {
    pub lane: String,
    pub state: String,
    pub consecutive_failures: u64,
    pub open_until: Option<Timestamp>,
    pub last_error: Option<String>,
    pub last_failure_at: Option<Timestamp>,
    pub last_success_at: Option<Timestamp>,
    pub recovered_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitInfo {
    pub token: String,
    pub remaining: u64,
    pub limit: u64,
    pub reset_at: Timestamp,
    pub held: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubHealth {
    pub lanes: Vec<LaneHealth>,
    pub rate_limits: Vec<RateLimitInfo>,
    pub pending_actions: u64,
    pub blocked_actions: u64,
    pub oldest_pending_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceAccount {
    pub identity: Identity,
    pub org: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub disabled_at: Option<Timestamp>,
    pub active_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateServiceAccountRequest {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceAccountRef {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListServiceAccountsResponse {
    pub accounts: Vec<ServiceAccount>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateServiceAccountTokenRequest {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
    pub token_name: String,
    pub scopes: Vec<String>,
    pub client_label: Option<String>,
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevokeServiceAccountTokenRequest {
    pub ctx: Ctx,
    pub org: String,
    pub name: String,
    pub token_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRoleGrant {
    pub identity: Identity,
    pub role: String,
    pub source: String,
    pub granted_by: Option<Identity>,
    pub granted_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRoleList {
    pub grants: Vec<InstanceRoleGrant>,
    pub configured_admins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetInstanceRoleRequest {
    pub ctx: Ctx,
    pub identity: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoveInstanceRoleRequest {
    pub ctx: Ctx,
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FedHello {
    pub ctx: Ctx,
    pub instance: String,
    pub instance_identity: String,
    pub instance_id: String,
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub timestamp: Timestamp,
    pub signature: Vec<u8>,
    pub capabilities: FedCapabilities,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FedCapabilities {
    pub protocol_versions: Vec<String>,
    pub vcs_types: Vec<String>,
    pub event_kinds: Vec<String>,
    pub max_frame_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FedHelloAck {
    pub instance: String,
    pub instance_identity: String,
    pub instance_id: String,
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub timestamp: Timestamp,
    pub signature: Vec<u8>,
    pub capabilities: FedCapabilities,
    pub peer_session_token: String,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifyHeadRequest {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub r#ref: String,
    pub sha: String,
    pub on_behalf_of: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeadQuery {
    pub ctx: Ctx,
    pub repo: RepoRef,
    pub r#ref: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeadInfo {
    pub repo: RepoRef,
    pub r#ref: String,
    pub sha: String,
    pub exists: bool,
    pub readable: bool,
    pub as_of: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitSummary {
    pub id: String,
    pub author_name: String,
    pub author_email: String,
    pub message_head: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PushPayload {
    pub r#ref: String,
    pub before: String,
    pub after: String,
    pub forced: bool,
    pub commits: Vec<CommitSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefPayload {
    pub r#ref: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrPayload {
    pub number: u64,
    pub head_sha: Option<String>,
    pub base_ref: Option<String>,
    pub state: Option<String>,
    pub merge_kind: Option<String>,
    pub comment_id: Option<String>,
    pub review_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleasePayload {
    pub tag: String,
    pub name: String,
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirrorPayload {
    pub object_key: String,
    pub error: Option<String>,
    pub action_kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsPayload {
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusPayload {
    pub sha: String,
    pub context: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MembershipPayload {
    pub identity: String,
    pub role: Option<String>,
    pub team: Option<String>,
}

/// EventPayload variants
#[derive(Debug, Clone, PartialEq)]
pub enum EventPayload {
    Variant0(PushPayload),
    Variant1(RefPayload),
    Variant2(PrPayload),
    Variant3(ReleasePayload),
    Variant4(MirrorPayload),
    Variant5(SettingsPayload),
    Variant6(StatusPayload),
    Variant7(MembershipPayload),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub lane: String,
    pub seq: u64,
    pub kind: String,
    pub actor: Option<Identity>,
    pub repo: Option<RepoRef>,
    pub org: Option<String>,
    pub subject: Option<String>,
    pub at: Timestamp,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeRequest {
    pub lanes: Vec<String>,
    pub kinds: Option<Vec<String>>,
    pub after_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnsubscribeRequest {
    pub lanes: Vec<String>,
}

/// StreamControl variants
#[derive(Debug, Clone, PartialEq)]
pub enum StreamControl {
    Variant0(SubscribeRequest),
    Variant1(UnsribeMarker),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnsribeMarker {
    pub unsubscribe: UnsubscribeRequest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamAck {
    pub subscribed: Vec<String>,
}

/// StreamMessage variants
#[derive(Debug, Clone, PartialEq)]
pub enum StreamMessage {
    Variant0(Event),
    Variant1(StreamAck),
}
