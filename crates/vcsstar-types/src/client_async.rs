//! Generated transport-agnostic service clients from CSIL specification

#![allow(async_fn_in_trait)]

use super::client::ClientError;
use super::codec::*;
use super::types::*;

/// The caller-supplied byte carrier: it performs the call named by `(service, op)`
/// with the already-encoded request bytes and returns the response bytes, or an
/// error. The generated client owns (de)serialization via the codec; the carrier
/// only moves bytes, so it can be HTTP, a queue, or an in-process loop.
pub trait AsyncTransport {
    async fn call(&self, service: &str, op: &str, req: &[u8]) -> Result<Vec<u8>, ClientError>;
}

/// Typed client for the Meta service.
pub struct MetaAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> MetaAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// server-info (request/response).
    pub async fn server_info(&self, req: Empty) -> Result<ServerInfo, ClientError> {
        let csil_resp = self
            .transport
            .call("Meta", "server-info", &encode_empty(&req))
            .await?;
        decode_server_info(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Auth service.
pub struct AuthAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> AuthAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// begin-login (request/response).
    pub async fn begin_login(
        &self,
        req: BeginLoginRequest,
    ) -> Result<BeginLoginResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "begin-login", &encode_begin_login_request(&req))
            .await?;
        decode_begin_login_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// complete-login (request/response).
    pub async fn complete_login(&self, req: CompleteLoginRequest) -> Result<Session, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "complete-login",
                &encode_complete_login_request(&req),
            )
            .await?;
        decode_session(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// create-token (request/response).
    pub async fn create_token(&self, req: CreateTokenRequest) -> Result<ApiToken, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "create-token", &encode_create_token_request(&req))
            .await?;
        decode_api_token(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// revoke-token (request/response).
    pub async fn revoke_token(&self, req: RevokeTokenRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "revoke-token", &encode_revoke_token_request(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// whoami (request/response).
    pub async fn whoami(&self, req: Empty) -> Result<WhoAmI, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "whoami", &encode_empty(&req))
            .await?;
        decode_who_am_i(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// begin-device-login (request/response).
    pub async fn begin_device_login(
        &self,
        req: BeginDeviceRequest,
    ) -> Result<DeviceCode, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "begin-device-login",
                &encode_begin_device_request(&req),
            )
            .await?;
        decode_device_code(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// poll-device-login (request/response).
    pub async fn poll_device_login(
        &self,
        req: DevicePollRequest,
    ) -> Result<DevicePollResponse, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "poll-device-login",
                &encode_device_poll_request(&req),
            )
            .await?;
        decode_device_poll_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// exchange-act-as-credential (request/response).
    pub async fn exchange_act_as_credential(
        &self,
        req: ActAsExchangeRequest,
    ) -> Result<ScopedBearer, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "exchange-act-as-credential",
                &encode_act_as_exchange_request(&req),
            )
            .await?;
        decode_scoped_bearer(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// scope-set (request/response).
    pub async fn scope_set(
        &self,
        req: ScopeSetRequest,
    ) -> Result<SignedActAsScopeSet, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "scope-set", &encode_scope_set_request(&req))
            .await?;
        decode_signed_act_as_scope_set(&csil_resp)
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Browser side of the device flow. Needs a web session.
    pub async fn approve_device_login(&self, req: DeviceApproveRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "approve-device-login",
                &encode_device_approve_request(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// register-key (request/response).
    pub async fn register_key(&self, req: RegisterKeyRequest) -> Result<SshKeyInfo, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "register-key", &encode_register_key_request(&req))
            .await?;
        decode_ssh_key_info(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-keys (request/response).
    pub async fn list_keys(&self, req: Empty) -> Result<ListKeysResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "list-keys", &encode_empty(&req))
            .await?;
        decode_list_keys_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// revoke-key (request/response).
    pub async fn revoke_key(&self, req: KeyRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "revoke-key", &encode_key_ref(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-tokens (request/response).
    pub async fn list_tokens(&self, req: Empty) -> Result<ListTokensResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "list-tokens", &encode_empty(&req))
            .await?;
        decode_list_tokens_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// logout (request/response).
    pub async fn logout(&self, req: Empty) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "logout", &encode_empty(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// GitHub account link of the caller (see GithubLink).
    pub async fn github_link_status(&self, req: Empty) -> Result<GithubLink, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "github-link-status", &encode_empty(&req))
            .await?;
        decode_github_link(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-github-token (request/response).
    pub async fn set_github_token(
        &self,
        req: SetGithubTokenRequest,
    ) -> Result<GithubLink, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "set-github-token",
                &encode_set_github_token_request(&req),
            )
            .await?;
        decode_github_link(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// unlink-github (request/response).
    pub async fn unlink_github(&self, req: Empty) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "unlink-github", &encode_empty(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// The caller's display preferences (defaults when none are saved).
    pub async fn get_preferences(&self, req: Empty) -> Result<Preferences, ClientError> {
        let csil_resp = self
            .transport
            .call("Auth", "get-preferences", &encode_empty(&req))
            .await?;
        decode_preferences(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Save the given fields; absent fields keep their value.
    pub async fn set_preferences(
        &self,
        req: SetPreferencesRequest,
    ) -> Result<Preferences, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Auth",
                "set-preferences",
                &encode_set_preferences_request(&req),
            )
            .await?;
        decode_preferences(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Orgs service.
pub struct OrgsAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> OrgsAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create (request/response).
    pub async fn create(&self, req: CreateOrgRequest) -> Result<Org, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "create", &encode_create_org_request(&req))
            .await?;
        decode_org(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// get (request/response).
    pub async fn get(&self, req: GetOrgRequest) -> Result<Org, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "get", &encode_get_org_request(&req))
            .await?;
        decode_org(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list (request/response).
    pub async fn list(&self, req: ListOrgsRequest) -> Result<OrgPage, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "list", &encode_list_orgs_request(&req))
            .await?;
        decode_org_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// update (request/response).
    pub async fn update(&self, req: UpdateOrgRequest) -> Result<Org, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "update", &encode_update_org_request(&req))
            .await?;
        decode_org(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-member (request/response).
    pub async fn set_member(&self, req: SetOrgMemberRequest) -> Result<OrgMember, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "set-member", &encode_set_org_member_request(&req))
            .await?;
        decode_org_member(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// remove-member (request/response).
    pub async fn remove_member(&self, req: RemoveOrgMemberRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Orgs",
                "remove-member",
                &encode_remove_org_member_request(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-members (request/response).
    pub async fn list_members(
        &self,
        req: ListOrgMembersRequest,
    ) -> Result<OrgMemberPage, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Orgs",
                "list-members",
                &encode_list_org_members_request(&req),
            )
            .await?;
        decode_org_member_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// create-team (request/response).
    pub async fn create_team(&self, req: CreateTeamRequest) -> Result<Team, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "create-team", &encode_create_team_request(&req))
            .await?;
        decode_team(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// delete-team (request/response).
    pub async fn delete_team(&self, req: TeamRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "delete-team", &encode_team_ref(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-team-member (request/response).
    pub async fn set_team_member(&self, req: SetTeamMemberRequest) -> Result<Team, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Orgs",
                "set-team-member",
                &encode_set_team_member_request(&req),
            )
            .await?;
        decode_team(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-teams (request/response).
    pub async fn list_teams(&self, req: GetOrgRequest) -> Result<ListTeamsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Orgs", "list-teams", &encode_get_org_request(&req))
            .await?;
        decode_list_teams_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Repos service.
pub struct ReposAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> ReposAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create (request/response).
    pub async fn create(&self, req: CreateRepoRequest) -> Result<Repo, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "create", &encode_create_repo_request(&req))
            .await?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// get (request/response).
    pub async fn get(&self, req: GetRepoRequest) -> Result<Repo, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "get", &encode_get_repo_request(&req))
            .await?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list (request/response).
    pub async fn list(&self, req: ListReposRequest) -> Result<RepoPage, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "list", &encode_list_repos_request(&req))
            .await?;
        decode_repo_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// update (request/response).
    pub async fn update(&self, req: UpdateRepoRequest) -> Result<Repo, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "update", &encode_update_repo_request(&req))
            .await?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// fork (request/response).
    pub async fn fork(&self, req: ForkRequest) -> Result<Repo, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "fork", &encode_fork_request(&req))
            .await?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-refs (request/response).
    pub async fn list_refs(&self, req: ListRefsRequest) -> Result<ListRefsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "list-refs", &encode_list_refs_request(&req))
            .await?;
        decode_list_refs_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// log (request/response).
    pub async fn log(&self, req: LogRequest) -> Result<CommitPage, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "log", &encode_log_request(&req))
            .await?;
        decode_commit_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// tree (request/response).
    pub async fn tree(&self, req: TreeRequest) -> Result<TreeResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "tree", &encode_tree_request(&req))
            .await?;
        decode_tree_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// blob (request/response).
    pub async fn blob(&self, req: BlobRequest) -> Result<BlobResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "blob", &encode_blob_request(&req))
            .await?;
        decode_blob_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// diff (request/response).
    pub async fn diff(&self, req: DiffRequest) -> Result<DiffResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "diff", &encode_diff_request(&req))
            .await?;
        decode_diff_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// delete (request/response).
    pub async fn delete(&self, req: DeleteRepoRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "delete", &encode_delete_repo_request(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// update-ref (request/response).
    pub async fn update_ref(&self, req: UpdateRefRequest) -> Result<RefInfo, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "update-ref", &encode_update_ref_request(&req))
            .await?;
        decode_ref_info(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// get-commit (request/response).
    pub async fn get_commit(&self, req: GetCommitRequest) -> Result<Commit, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "get-commit", &encode_get_commit_request(&req))
            .await?;
        decode_commit(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-grant (request/response).
    pub async fn set_grant(&self, req: SetGrantRequest) -> Result<Grant, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "set-grant", &encode_set_grant_request(&req))
            .await?;
        decode_grant(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// remove-grant (request/response).
    pub async fn remove_grant(&self, req: RemoveGrantRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "remove-grant", &encode_remove_grant_request(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-grants (request/response).
    pub async fn list_grants(
        &self,
        req: GetRepoRequest,
    ) -> Result<ListGrantsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "list-grants", &encode_get_repo_request(&req))
            .await?;
        decode_list_grants_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-protection (request/response).
    pub async fn set_protection(
        &self,
        req: SetProtectionRequest,
    ) -> Result<BranchProtection, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Repos",
                "set-protection",
                &encode_set_protection_request(&req),
            )
            .await?;
        decode_branch_protection(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// remove-protection (request/response).
    pub async fn remove_protection(&self, req: RemoveProtectionRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Repos",
                "remove-protection",
                &encode_remove_protection_request(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-protections (request/response).
    pub async fn list_protections(
        &self,
        req: GetRepoRequest,
    ) -> Result<ListProtectionsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "list-protections", &encode_get_repo_request(&req))
            .await?;
        decode_list_protections_response(&csil_resp)
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Pack every object reachable from the current refs that no live segment
    /// holds, and upload it as one new segment. Use it for history that was
    /// written before the object store existed. Repeatable; no-op when covered.
    pub async fn capture_history(&self, req: GetRepoRequest) -> Result<CaptureReport, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "capture-history", &encode_get_repo_request(&req))
            .await?;
        decode_capture_report(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Direct forks of the repository that the caller can see, on this
    /// instance. A pull request into the repository may come from one.
    pub async fn list_forks(&self, req: GetRepoRequest) -> Result<RepoPage, ClientError> {
        let csil_resp = self
            .transport
            .call("Repos", "list-forks", &encode_get_repo_request(&req))
            .await?;
        decode_repo_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the PullRequests service.
pub struct PullRequestsAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> PullRequestsAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// open (request/response).
    pub async fn open(&self, req: OpenPrRequest) -> Result<PullRequest, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "open", &encode_open_pr_request(&req))
            .await?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// get (request/response).
    pub async fn get(&self, req: PrRef) -> Result<PullRequest, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "get", &encode_pr_ref(&req))
            .await?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list (request/response).
    pub async fn list(&self, req: ListPrRequest) -> Result<PrPage, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "list", &encode_list_pr_request(&req))
            .await?;
        decode_pr_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// review (request/response).
    pub async fn review(&self, req: ReviewRequest) -> Result<Review, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "review", &encode_review_request(&req))
            .await?;
        decode_review(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// comment (request/response).
    pub async fn comment(&self, req: CommentRequest) -> Result<Comment, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "comment", &encode_comment_request(&req))
            .await?;
        decode_comment(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// merge (request/response).
    pub async fn merge(&self, req: MergeRequest) -> Result<MergeResult, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "merge", &encode_merge_request(&req))
            .await?;
        decode_merge_result(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// close (request/response).
    pub async fn close(&self, req: PrRef) -> Result<PullRequest, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "close", &encode_pr_ref(&req))
            .await?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// reopen (request/response).
    pub async fn reopen(&self, req: PrRef) -> Result<PullRequest, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "reopen", &encode_pr_ref(&req))
            .await?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// retarget (request/response).
    pub async fn retarget(&self, req: RetargetRequest) -> Result<PullRequest, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "retarget", &encode_retarget_request(&req))
            .await?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// update (request/response).
    pub async fn update(&self, req: UpdatePrRequest) -> Result<PullRequest, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "update", &encode_update_pr_request(&req))
            .await?;
        decode_pull_request(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-reviews (request/response).
    pub async fn list_reviews(&self, req: PrRef) -> Result<ListReviewsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "list-reviews", &encode_pr_ref(&req))
            .await?;
        decode_list_reviews_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-comments (request/response).
    pub async fn list_comments(&self, req: PrRef) -> Result<ListCommentsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("PullRequests", "list-comments", &encode_pr_ref(&req))
            .await?;
        decode_list_comments_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Statuses service.
pub struct StatusesAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> StatusesAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// set (request/response).
    pub async fn set(&self, req: SetStatusRequest) -> Result<CommitStatus, ClientError> {
        let csil_resp = self
            .transport
            .call("Statuses", "set", &encode_set_status_request(&req))
            .await?;
        decode_commit_status(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list (request/response).
    pub async fn list(&self, req: ListStatusRequest) -> Result<ListStatusResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Statuses", "list", &encode_list_status_request(&req))
            .await?;
        decode_list_status_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Hooks service.
pub struct HooksAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> HooksAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create-endpoint (request/response).
    pub async fn create_endpoint(
        &self,
        req: CreateEndpointRequest,
    ) -> Result<Endpoint, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Hooks",
                "create-endpoint",
                &encode_create_endpoint_request(&req),
            )
            .await?;
        decode_endpoint(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-endpoints (request/response).
    pub async fn list_endpoints(
        &self,
        req: ListEndpointsRequest,
    ) -> Result<ListEndpointsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Hooks",
                "list-endpoints",
                &encode_list_endpoints_request(&req),
            )
            .await?;
        decode_list_endpoints_response(&csil_resp)
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// delete-endpoint (request/response).
    pub async fn delete_endpoint(&self, req: EndpointRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Hooks", "delete-endpoint", &encode_endpoint_ref(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-deliveries (request/response).
    pub async fn list_deliveries(&self, req: DeliveryQuery) -> Result<DeliveryPage, ClientError> {
        let csil_resp = self
            .transport
            .call("Hooks", "list-deliveries", &encode_delivery_query(&req))
            .await?;
        decode_delivery_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// redeliver (request/response).
    pub async fn redeliver(&self, req: DeliveryRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Hooks", "redeliver", &encode_delivery_ref(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// update-endpoint (request/response).
    pub async fn update_endpoint(
        &self,
        req: UpdateEndpointRequest,
    ) -> Result<Endpoint, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Hooks",
                "update-endpoint",
                &encode_update_endpoint_request(&req),
            )
            .await?;
        decode_endpoint(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Mirror service.
pub struct MirrorAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> MirrorAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// status (request/response).
    pub async fn status(&self, req: GetRepoRequest) -> Result<MirrorStatus, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "status", &encode_get_repo_request(&req))
            .await?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-blocked (request/response).
    pub async fn list_blocked(
        &self,
        req: GetRepoRequest,
    ) -> Result<ListBlockedResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "list-blocked", &encode_get_repo_request(&req))
            .await?;
        decode_list_blocked_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// replay-blocked (request/response).
    pub async fn replay_blocked(&self, req: ReplayRequest) -> Result<ReplayResult, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "replay-blocked", &encode_replay_request(&req))
            .await?;
        decode_replay_result(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-credential (request/response).
    pub async fn set_credential(
        &self,
        req: SetCredentialRequest,
    ) -> Result<Credential, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Mirror",
                "set-credential",
                &encode_set_credential_request(&req),
            )
            .await?;
        decode_credential(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// reconcile (request/response).
    pub async fn reconcile(&self, req: GetRepoRequest) -> Result<ReconcileReport, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "reconcile", &encode_get_repo_request(&req))
            .await?;
        decode_reconcile_report(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// pause (request/response).
    pub async fn pause(&self, req: GetRepoRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "pause", &encode_get_repo_request(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// resume (request/response).
    pub async fn resume(&self, req: GetRepoRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "resume", &encode_get_repo_request(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Make the repository and queue its import. The answer comes before
    /// the branches are fetched; `status` reports `import_state`.
    pub async fn import_repo(&self, req: ImportRequest) -> Result<Repo, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "import-repo", &encode_import_request(&req))
            .await?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// configure (request/response).
    pub async fn configure(
        &self,
        req: ConfigureMirrorRequest,
    ) -> Result<MirrorStatus, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Mirror",
                "configure",
                &encode_configure_mirror_request(&req),
            )
            .await?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-credentials (request/response).
    pub async fn list_credentials(
        &self,
        req: ListCredentialsRequest,
    ) -> Result<ListCredentialsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Mirror",
                "list-credentials",
                &encode_list_credentials_request(&req),
            )
            .await?;
        decode_list_credentials_response(&csil_resp)
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// remove-credential (request/response).
    pub async fn remove_credential(&self, req: CredentialRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "remove-credential", &encode_credential_ref(&req))
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Repositories visible under the organization's `github_owner`, with
    /// the vcsstar repository that mirrors each one when there is one.
    /// Enable one with `import-repo`.
    pub async fn list_github_repos(
        &self,
        req: ListGithubReposRequest,
    ) -> Result<GithubRepoPage, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Mirror",
                "list-github-repos",
                &encode_list_github_repos_request(&req),
            )
            .await?;
        decode_github_repo_page(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// This node's view of GitHub availability and rate limits.
    pub async fn health(&self, req: Empty) -> Result<GithubHealth, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "health", &encode_empty(&req))
            .await?;
        decode_github_health(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Start the history import of the repository's GitHub pull requests
    /// again from the first one. A mirrored repository imports its history
    /// by itself; this is for a repository whose import must run again.
    pub async fn backfill(&self, req: GetRepoRequest) -> Result<MirrorStatus, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "backfill", &encode_get_repo_request(&req))
            .await?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Where the instance's GitHub Apps are installed, with the vcsstar
    /// organizations whose `github_owner` is each owner.
    pub async fn list_github_installations(
        &self,
        req: Empty,
    ) -> Result<GithubInstallationList, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "list-github-installations", &encode_empty(&req))
            .await?;
        decode_github_installation_list(&csil_resp)
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// import-all (request/response).
    pub async fn import_all(&self, req: ImportAllRequest) -> Result<ImportAllResult, ClientError> {
        let csil_resp = self
            .transport
            .call("Mirror", "import-all", &encode_import_all_request(&req))
            .await?;
        decode_import_all_result(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-schedule (request/response).
    pub async fn set_schedule(
        &self,
        req: SetMirrorScheduleRequest,
    ) -> Result<MirrorStatus, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Mirror",
                "set-schedule",
                &encode_set_mirror_schedule_request(&req),
            )
            .await?;
        decode_mirror_status(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Instance service.
pub struct InstanceAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> InstanceAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Every instance role. Instance admins only.
    pub async fn list_roles(&self, req: Empty) -> Result<InstanceRoleList, ClientError> {
        let csil_resp = self
            .transport
            .call("Instance", "list-roles", &encode_empty(&req))
            .await?;
        decode_instance_role_list(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// set-role (request/response).
    pub async fn set_role(
        &self,
        req: SetInstanceRoleRequest,
    ) -> Result<InstanceRoleGrant, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Instance",
                "set-role",
                &encode_set_instance_role_request(&req),
            )
            .await?;
        decode_instance_role_grant(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// remove-role (request/response).
    pub async fn remove_role(&self, req: RemoveInstanceRoleRequest) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Instance",
                "remove-role",
                &encode_remove_instance_role_request(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the ServiceAccounts service.
pub struct ServiceAccountsAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> ServiceAccountsAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// create (request/response).
    pub async fn create(
        &self,
        req: CreateServiceAccountRequest,
    ) -> Result<ServiceAccount, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "ServiceAccounts",
                "create",
                &encode_create_service_account_request(&req),
            )
            .await?;
        decode_service_account(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list (request/response).
    pub async fn list(
        &self,
        req: GetOrgRequest,
    ) -> Result<ListServiceAccountsResponse, ClientError> {
        let csil_resp = self
            .transport
            .call("ServiceAccounts", "list", &encode_get_org_request(&req))
            .await?;
        decode_list_service_accounts_response(&csil_resp)
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Disabling refuses every token of the account; enabling lets the
    /// tokens that were not revoked work again.
    pub async fn disable(&self, req: ServiceAccountRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "ServiceAccounts",
                "disable",
                &encode_service_account_ref(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// enable (request/response).
    pub async fn enable(&self, req: ServiceAccountRef) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "ServiceAccounts",
                "enable",
                &encode_service_account_ref(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// create-token (request/response).
    pub async fn create_token(
        &self,
        req: CreateServiceAccountTokenRequest,
    ) -> Result<ApiToken, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "ServiceAccounts",
                "create-token",
                &encode_create_service_account_token_request(&req),
            )
            .await?;
        decode_api_token(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// list-tokens (request/response).
    pub async fn list_tokens(
        &self,
        req: ServiceAccountRef,
    ) -> Result<ListTokensResponse, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "ServiceAccounts",
                "list-tokens",
                &encode_service_account_ref(&req),
            )
            .await?;
        decode_list_tokens_response(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// revoke-token (request/response).
    pub async fn revoke_token(
        &self,
        req: RevokeServiceAccountTokenRequest,
    ) -> Result<Ok, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "ServiceAccounts",
                "revoke-token",
                &encode_revoke_service_account_token_request(&req),
            )
            .await?;
        decode_ok(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the Federation service.
pub struct FederationAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> FederationAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// handshake (request/response).
    pub async fn handshake(&self, req: FedHello) -> Result<FedHelloAck, ClientError> {
        let csil_resp = self
            .transport
            .call("Federation", "handshake", &encode_fed_hello(&req))
            .await?;
        decode_fed_hello_ack(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// verify-head (request/response).
    pub async fn verify_head(&self, req: VerifyHeadRequest) -> Result<HeadInfo, ClientError> {
        let csil_resp = self
            .transport
            .call(
                "Federation",
                "verify-head",
                &encode_verify_head_request(&req),
            )
            .await?;
        decode_head_info(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// head-info (request/response).
    pub async fn head_info(&self, req: HeadQuery) -> Result<HeadInfo, ClientError> {
        let csil_resp = self
            .transport
            .call("Federation", "head-info", &encode_head_query(&req))
            .await?;
        decode_head_info(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// repo-info (request/response).
    pub async fn repo_info(&self, req: GetRepoRequest) -> Result<Repo, ClientError> {
        let csil_resp = self
            .transport
            .call("Federation", "repo-info", &encode_get_repo_request(&req))
            .await?;
        decode_repo(&csil_resp).map_err(|e| ClientError::Transport(e.to_string()))
    }
}

/// Typed client for the EventStream service.
pub struct EventStreamAsyncClient<T: AsyncTransport> {
    #[allow(dead_code)]
    transport: T,
}

impl<T: AsyncTransport> EventStreamAsyncClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    // channel operation `subscribe` is not part of the RPC client
}
