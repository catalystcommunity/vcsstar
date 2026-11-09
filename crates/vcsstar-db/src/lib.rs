//! Postgres access: connection pool, embedded migrations, schema, and models.
//! Every query is repo- or org-scoped; see plan/02-data-model.md.

pub mod models;
#[rustfmt::skip]
#[allow(clippy::all)]
pub mod schema;

use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub use diesel;
pub use uuid::Uuid;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../../migrations");

pub type DbPool = Pool<ConnectionManager<PgConnection>>;
pub type DbConn = PooledConnection<ConnectionManager<PgConnection>>;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("pool: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("query: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("migration: {0}")]
    Migration(String),
}

pub fn pool(database_url: &str, max_size: u32) -> Result<DbPool, DbError> {
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    Ok(Pool::builder().max_size(max_size).build(manager)?)
}

pub fn run_migrations(conn: &mut PgConnection) -> Result<Vec<String>, DbError> {
    conn.run_pending_migrations(MIGRATIONS)
        .map(|versions| versions.iter().map(|v| v.to_string()).collect())
        .map_err(|e| DbError::Migration(e.to_string()))
}

/// Allocate the next per-lane outbox sequence inside the caller's transaction.
pub fn next_lane_seq(conn: &mut PgConnection, lane: &str) -> Result<i64, DbError> {
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Text};
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        seq: i64,
    }
    let row: Row = diesel::sql_query("SELECT next_lane_seq($1) AS seq")
        .bind::<Text, _>(lane)
        .get_result(conn)?;
    Ok(row.seq)
}

pub fn new_id() -> Uuid {
    Uuid::now_v7()
}
