//! Refs in Postgres with compare-and-swap. These functions run inside the
//! caller's transaction so the outbox write lands with the ref change.

use diesel::prelude::*;
use uuid::Uuid;
use vcsstar_db::schema::{repo_refs, repos};
use vcsstar_vcs::RevId;

use crate::{target_bytes, target_from_bytes, Result, StoreError};

/// One ref change of a push or an API call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub name: String,
    /// Expected current target; `None` means the ref must not exist.
    pub old: Option<RevId>,
    /// New target; `None` means delete.
    pub new: Option<RevId>,
}

pub fn load(conn: &mut PgConnection, repo_id: Uuid) -> Result<Vec<(String, RevId)>> {
    let rows: Vec<(String, Vec<u8>)> = repo_refs::table
        .filter(repo_refs::repo_id.eq(repo_id))
        .select((repo_refs::name, repo_refs::target))
        .order(repo_refs::name.asc())
        .load(conn)?;
    Ok(rows
        .into_iter()
        .map(|(n, t)| (n, target_from_bytes(&t)))
        .collect())
}

pub fn get(conn: &mut PgConnection, repo_id: Uuid, name: &str) -> Result<Option<RevId>> {
    let row: Option<Vec<u8>> = repo_refs::table
        .filter(repo_refs::repo_id.eq(repo_id))
        .filter(repo_refs::name.eq(name))
        .select(repo_refs::target)
        .first(conn)
        .optional()?;
    Ok(row.map(|t| target_from_bytes(&t)))
}

/// Apply every update with compare-and-swap semantics, or fail on the first
/// mismatch. Bumps `repos.refs_version`. Must run inside a transaction.
pub fn apply(
    conn: &mut PgConnection,
    repo_id: Uuid,
    updates: &[RefUpdate],
    actor: Option<Uuid>,
) -> Result<()> {
    for u in updates {
        // Lock the row (or its absence) for the duration of the transaction.
        let current: Option<Vec<u8>> = repo_refs::table
            .filter(repo_refs::repo_id.eq(repo_id))
            .filter(repo_refs::name.eq(&u.name))
            .select(repo_refs::target)
            .for_update()
            .first(conn)
            .optional()?;
        let current = current.map(|t| target_from_bytes(&t));
        if current != u.old {
            return Err(StoreError::Cas(format!(
                "{}: expected {}, found {}",
                u.name,
                u.old.as_ref().map(|r| r.0.as_str()).unwrap_or("<absent>"),
                current.as_ref().map(|r| r.0.as_str()).unwrap_or("<absent>")
            )));
        }
        match (&current, &u.new) {
            (_, None) => {
                diesel::delete(
                    repo_refs::table
                        .filter(repo_refs::repo_id.eq(repo_id))
                        .filter(repo_refs::name.eq(&u.name)),
                )
                .execute(conn)?;
            }
            (None, Some(new)) => {
                diesel::insert_into(repo_refs::table)
                    .values((
                        repo_refs::repo_id.eq(repo_id),
                        repo_refs::name.eq(&u.name),
                        repo_refs::target.eq(target_bytes(new)),
                        repo_refs::updated_by.eq(actor),
                    ))
                    .execute(conn)?;
            }
            (Some(_), Some(new)) => {
                diesel::update(
                    repo_refs::table
                        .filter(repo_refs::repo_id.eq(repo_id))
                        .filter(repo_refs::name.eq(&u.name)),
                )
                .set((
                    repo_refs::target.eq(target_bytes(new)),
                    repo_refs::updated_at.eq(diesel::dsl::now),
                    repo_refs::updated_by.eq(actor),
                ))
                .execute(conn)?;
            }
        }
    }
    diesel::update(repos::table.find(repo_id))
        .set(repos::refs_version.eq(repos::refs_version + 1))
        .execute(conn)?;
    Ok(())
}

/// Copy every ref of `src` to `dst` (fork). Must run inside a transaction.
pub fn copy_all(conn: &mut PgConnection, src: Uuid, dst: Uuid) -> Result<usize> {
    let rows = load(conn, src)?;
    let mut n = 0;
    for (name, target) in rows {
        diesel::insert_into(repo_refs::table)
            .values((
                repo_refs::repo_id.eq(dst),
                repo_refs::name.eq(&name),
                repo_refs::target.eq(target_bytes(&target)),
            ))
            .execute(conn)?;
        n += 1;
    }
    Ok(n)
}
