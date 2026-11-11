//! Row structs for the tables the server reads and writes directly.

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use uuid::Uuid;

use crate::schema::*;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = identities)]
pub struct Identity {
    pub id: Uuid,
    pub lk_user_id: Uuid,
    pub lk_domain: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub email_verified: bool,
    pub first_seen: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub disabled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = identities)]
pub struct NewIdentity<'a> {
    pub id: Uuid,
    pub lk_user_id: Uuid,
    pub lk_domain: &'a str,
    pub handle: &'a str,
    pub display_name: Option<&'a str>,
    pub email: Option<&'a str>,
    pub email_verified: bool,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = api_tokens)]
pub struct ApiToken {
    pub id: Uuid,
    pub identity_id: Uuid,
    pub token_hash: Vec<u8>,
    pub name: String,
    pub scopes: Vec<Option<String>>,
    pub client_label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = api_tokens)]
pub struct NewApiToken<'a> {
    pub id: Uuid,
    pub identity_id: Uuid,
    pub token_hash: Vec<u8>,
    pub name: &'a str,
    pub scopes: Vec<Option<String>>,
    pub client_label: Option<&'a str>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = sessions)]
pub struct Session {
    pub id: Uuid,
    pub identity_id: Uuid,
    pub token_hash: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = orgs)]
pub struct Org {
    pub id: Uuid,
    pub name: String,
    pub display_name: Option<String>,
    pub github_owner: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = orgs)]
pub struct NewOrg<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub display_name: Option<&'a str>,
    pub github_owner: Option<&'a str>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = org_members)]
pub struct OrgMember {
    pub org_id: Uuid,
    pub identity_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = repos)]
pub struct Repo {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub visibility: String,
    pub vcs_type: String,
    pub default_branch: String,
    pub description: Option<String>,
    pub fork_of_repo_id: Option<Uuid>,
    pub fork_of_remote: Option<String>,
    pub archived: bool,
    pub mirror_enabled: bool,
    pub refs_version: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = repos)]
pub struct NewRepo<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: &'a str,
    pub visibility: &'a str,
    pub vcs_type: &'a str,
    pub default_branch: &'a str,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = repo_grants)]
pub struct RepoGrant {
    pub repo_id: Uuid,
    pub grantee_kind: String,
    pub grantee_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = events)]
pub struct NewEvent<'a> {
    pub lane: &'a str,
    pub seq: i64,
    pub repo_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub kind: &'a str,
    pub actor: Option<Uuid>,
    pub subject: Option<&'a str>,
    pub payload_cbor: Vec<u8>,
    pub origin: Option<&'a str>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = audit_log)]
pub struct NewAudit<'a> {
    pub id: Uuid,
    pub actor: Option<Uuid>,
    pub acting_via: Option<&'a str>,
    pub action: &'a str,
    pub resource: &'a str,
    pub detail_cbor: Option<Vec<u8>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = sessions)]
pub struct NewSession<'a> {
    pub id: Uuid,
    pub identity_id: Uuid,
    pub token_hash: Vec<u8>,
    pub expires_at: DateTime<Utc>,
    pub user_agent: Option<&'a str>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = pending_logins)]
pub struct PendingLogin {
    pub id: Uuid,
    pub attempt_hash: Vec<u8>,
    pub nonce: String,
    pub identity_input: String,
    pub idp_domain: String,
    pub return_to: Option<String>,
    pub device_login_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = pending_logins)]
pub struct NewPendingLogin<'a> {
    pub id: Uuid,
    pub attempt_hash: Vec<u8>,
    pub nonce: &'a str,
    pub identity_input: &'a str,
    pub idp_domain: &'a str,
    pub return_to: Option<&'a str>,
    pub device_login_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = device_logins)]
pub struct DeviceLogin {
    pub id: Uuid,
    pub code_hash: Vec<u8>,
    pub user_code_hash: Vec<u8>,
    pub client_label: String,
    pub scopes: Vec<Option<String>>,
    pub state: String,
    pub identity_id: Option<Uuid>,
    pub api_token_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = device_logins)]
pub struct NewDeviceLogin<'a> {
    pub id: Uuid,
    pub code_hash: Vec<u8>,
    pub user_code_hash: Vec<u8>,
    pub client_label: &'a str,
    pub scopes: Vec<Option<String>>,
    pub state: &'a str,
    pub expires_at: DateTime<Utc>,
}
