//! Durable connection, replica, and history records.
use super::Item;
use crate::{App, Error};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Jsonb, Nullable, Text, Timestamptz, Uuid as SqlUuid};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, QueryableByName)]
pub struct Connection {
    #[diesel(sql_type=SqlUuid)]
    pub id: Uuid,
    #[diesel(sql_type=SqlUuid)]
    pub repo_id: Uuid,
    #[diesel(sql_type=Text)]
    pub provider: String,
    #[diesel(sql_type=Text)]
    pub host: String,
    #[diesel(sql_type=Text)]
    pub project: String,
    #[diesel(sql_type=Bool)]
    pub paused: bool,
    #[diesel(sql_type=Bool)]
    pub inbound_enabled: bool,
    #[diesel(sql_type=Bool)]
    pub outbound_enabled: bool,
    #[diesel(sql_type=Bool)]
    pub create_remote: bool,
    #[diesel(sql_type=Bool)]
    pub automatic_webhook: bool,
    #[diesel(sql_type=Bool)]
    pub setup_done: bool,
    #[diesel(sql_type=diesel::sql_types::Binary)]
    pub webhook_secret: Vec<u8>,
    #[diesel(sql_type=BigInt)]
    pub requested: i64,
    #[diesel(sql_type=BigInt)]
    pub completed: i64,
    #[diesel(sql_type=Timestamptz)]
    pub poll_after: OffsetDateTime,
    #[diesel(sql_type=Nullable<Timestamptz>)]
    pub last_success_at: Option<OffsetDateTime>,
    #[diesel(sql_type=Nullable<Text>)]
    pub last_error: Option<String>,
}
impl Connection {
    pub fn git_url(&self, app: &App) -> String {
        if self.provider == "github" {
            crate::mirror::github_url(app, &self.project)
        } else {
            format!("https://{}/{}.git", self.host, self.project)
        }
    }
    pub fn webhook_url(&self, app: &App) -> String {
        format!(
            "{}/hooks/sync/{}",
            app.config.public_url.trim_end_matches('/'),
            self.id
        )
    }
}
#[derive(Debug, Clone, QueryableByName)]
pub struct Replica {
    #[diesel(sql_type=Text)]
    pub object_key: String,
    #[diesel(sql_type=Nullable<Text>)]
    pub remote_id: Option<String>,
    #[diesel(sql_type=Nullable<Jsonb>)]
    pub local_base: Option<Value>,
    #[diesel(sql_type=Nullable<Jsonb>)]
    pub remote_base: Option<Value>,
    #[diesel(sql_type=Text)]
    pub outcome: String,
    #[diesel(sql_type=Nullable<Text>)]
    pub last_error: Option<String>,
}
#[derive(Debug, Clone, QueryableByName)]
pub struct Object {
    #[diesel(sql_type=Text)]
    pub object_key: String,
    #[diesel(sql_type=Jsonb)]
    pub payload: Value,
    #[diesel(sql_type=Text)]
    pub source: String,
    #[diesel(sql_type=Bool)]
    pub deleted: bool,
}
#[derive(Debug, Clone, QueryableByName)]
pub struct Conflict {
    #[diesel(sql_type=SqlUuid)]
    pub id: Uuid,
    #[diesel(sql_type=SqlUuid)]
    pub repo_id: Uuid,
    #[diesel(sql_type=SqlUuid)]
    pub connection_id: Uuid,
    #[diesel(sql_type=Text)]
    pub object_key: String,
    #[diesel(sql_type=Nullable<Jsonb>)]
    pub local_value: Option<Value>,
    #[diesel(sql_type=Nullable<Jsonb>)]
    pub remote_value: Option<Value>,
    #[diesel(sql_type=Timestamptz)]
    pub created_at: OffsetDateTime,
    #[diesel(sql_type=Nullable<Text>)]
    pub resolution: Option<String>,
}
#[derive(Debug, Clone, QueryableByName)]
pub struct Policy {
    #[diesel(sql_type=Nullable<Text>)]
    pub preferred_source: Option<String>,
    #[diesel(sql_type=Nullable<SqlUuid>)]
    pub resolver_identity: Option<Uuid>,
    #[diesel(sql_type=Nullable<SqlUuid>)]
    pub resolver_team: Option<Uuid>,
}
pub fn connections(conn: &mut PgConnection, repo: Uuid) -> Result<Vec<Connection>, Error> {
    Ok(
        diesel::sql_query("SELECT * FROM sync_connections WHERE repo_id=$1 ORDER BY created_at,id")
            .bind::<SqlUuid, _>(repo)
            .load(conn)?,
    )
}
pub fn connection(conn: &mut PgConnection, id: Uuid) -> Result<Connection, Error> {
    diesel::sql_query("SELECT * FROM sync_connections WHERE id=$1")
        .bind::<SqlUuid, _>(id)
        .get_result(conn)
        .map_err(Error::from)
}
pub fn managed(conn: &mut PgConnection, repo: Uuid) -> Result<bool, Error> {
    Ok(!connections(conn, repo)?.is_empty())
}
pub fn replicas(conn: &mut PgConnection, id: Uuid) -> Result<Vec<Replica>, Error> {
    Ok(
        diesel::sql_query("SELECT * FROM sync_replicas WHERE connection_id=$1 ORDER BY object_key")
            .bind::<SqlUuid, _>(id)
            .load(conn)?,
    )
}
pub fn objects(conn: &mut PgConnection, repo: Uuid) -> Result<Vec<Object>, Error> {
    Ok(
        diesel::sql_query("SELECT * FROM sync_objects WHERE repo_id=$1 ORDER BY object_key")
            .bind::<SqlUuid, _>(repo)
            .load(conn)?,
    )
}
pub fn policy(conn: &mut PgConnection, repo: Uuid) -> Result<Option<Policy>, Error> {
    Ok(
        diesel::sql_query("SELECT * FROM sync_policy WHERE repo_id=$1")
            .bind::<SqlUuid, _>(repo)
            .get_result(conn)
            .optional()?,
    )
}
pub fn conflicts(conn: &mut PgConnection, repo: Uuid) -> Result<Vec<Conflict>, Error> {
    Ok(diesel::sql_query("SELECT * FROM sync_conflicts WHERE repo_id=$1 AND resolved_at IS NULL ORDER BY created_at,id").bind::<SqlUuid,_>(repo).load(conn)?)
}
pub fn save_object(
    conn: &mut PgConnection,
    repo: Uuid,
    key: &str,
    item: Option<&Item>,
    source: &str,
) -> Result<(), Error> {
    if let Some(item) = item {
        diesel::sql_query("INSERT INTO sync_objects(repo_id,object_key,payload,source) VALUES($1,$2,$3,$4) ON CONFLICT(repo_id,object_key) DO UPDATE SET payload=$3,source=$4,deleted=false,revision=sync_objects.revision+1 WHERE sync_objects.payload IS DISTINCT FROM $3 OR sync_objects.deleted OR sync_objects.source IS DISTINCT FROM $4")
            .bind::<SqlUuid,_>(repo).bind::<Text,_>(key).bind::<Jsonb,_>(serde_json::to_value(item).map_err(Error::internal)?).bind::<Text,_>(source).execute(conn)?;
    } else {
        diesel::sql_query("UPDATE sync_objects SET deleted=true,source=$3,revision=revision+1 WHERE repo_id=$1 AND object_key=$2 AND NOT deleted").bind::<SqlUuid,_>(repo).bind::<Text,_>(key).bind::<Text,_>(source).execute(conn)?;
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub fn save_replica(
    conn: &mut PgConnection,
    id: Uuid,
    key: &str,
    remote_id: Option<&str>,
    local: Option<&Item>,
    remote: Option<&Item>,
    outcome: &str,
    error: Option<&str>,
) -> Result<(), Error> {
    diesel::sql_query("INSERT INTO sync_replicas(connection_id,object_key,remote_id,local_base,remote_base,outcome,last_error) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(connection_id,object_key) DO UPDATE SET remote_id=COALESCE($3,sync_replicas.remote_id),local_base=$4,remote_base=$5,outcome=$6,last_error=$7,updated_at=now()")
        .bind::<SqlUuid,_>(id).bind::<Text,_>(key).bind::<Nullable<Text>,_>(remote_id)
        .bind::<Nullable<Jsonb>,_>(local.map(serde_json::to_value).transpose().map_err(Error::internal)?)
        .bind::<Nullable<Jsonb>,_>(remote.map(serde_json::to_value).transpose().map_err(Error::internal)?)
        .bind::<Text,_>(outcome).bind::<Nullable<Text>,_>(error).execute(conn)?;
    Ok(())
}
pub fn record_conflict(
    conn: &mut PgConnection,
    c: &Connection,
    key: &str,
    local: Option<&Item>,
    remote: Option<&Item>,
) -> Result<(), Error> {
    diesel::sql_query("INSERT INTO sync_conflicts(id,repo_id,connection_id,object_key,local_value,remote_value) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(repo_id,object_key,connection_id) WHERE resolved_at IS NULL DO UPDATE SET local_value=$5,remote_value=$6")
        .bind::<SqlUuid,_>(Uuid::now_v7()).bind::<SqlUuid,_>(c.repo_id).bind::<SqlUuid,_>(c.id).bind::<Text,_>(key)
        .bind::<Nullable<Jsonb>,_>(local.map(serde_json::to_value).transpose().map_err(Error::internal)?)
        .bind::<Nullable<Jsonb>,_>(remote.map(serde_json::to_value).transpose().map_err(Error::internal)?).execute(conn)?;
    Ok(())
}
pub fn archive(
    conn: &mut PgConnection,
    id: Uuid,
    remote_id: &str,
    value: &Value,
) -> Result<(), Error> {
    diesel::sql_query("INSERT INTO sync_history(connection_id,remote_id,digest,payload) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
        .bind::<SqlUuid,_>(id).bind::<Text,_>(remote_id).bind::<Text,_>(super::digest(value)).bind::<Jsonb,_>(value).execute(conn)?;
    Ok(())
}
pub fn wake(conn: &mut PgConnection, repo: Uuid) -> Result<(), Error> {
    diesel::sql_query(
        "UPDATE sync_connections SET requested=requested+1,poll_after=now() WHERE repo_id=$1",
    )
    .bind::<SqlUuid, _>(repo)
    .execute(conn)?;
    Ok(())
}
pub fn item(v: &Option<Value>) -> Result<Option<Item>, Error> {
    v.clone()
        .map(serde_json::from_value)
        .transpose()
        .map_err(Error::internal)
}
