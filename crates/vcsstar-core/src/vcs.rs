//! Repository hydration: make the node-local cache directory reflect the
//! truth in Postgres before any VCS operation, and small helpers shared by
//! the read ops, the push transaction, and the wire handlers.

use std::path::PathBuf;

use diesel::PgConnection;
use vcsstar_db::models::{Org, Repo};
use vcsstar_db::DbConn;
use vcsstar_store::refs;
use vcsstar_vcs::{RevId, RevisionInfo, VcsError};

use crate::{App, Code, Error};

impl From<VcsError> for Error {
    fn from(e: VcsError) -> Self {
        match e {
            VcsError::NotFound(m) => Error::new(Code::NotFound, m),
            VcsError::Invalid(m) => Error::invalid(m),
            VcsError::Unsupported(m) => Error::new(Code::PreconditionFailed, m),
            other => Error::internal(other),
        }
    }
}

impl From<vcsstar_store::StoreError> for Error {
    fn from(e: vcsstar_store::StoreError) -> Self {
        match e {
            vcsstar_store::StoreError::Vcs(v) => v.into(),
            vcsstar_store::StoreError::Cas(m) => Error::new(Code::Conflict, m),
            vcsstar_store::StoreError::Db(d) => d.into(),
            other => Error::internal(other),
        }
    }
}

/// Materialize refs from Postgres into the cache directory and return it.
pub fn hydrate(app: &App, conn: &mut DbConn, repo: &Repo) -> Result<PathBuf, Error> {
    let refs = refs::load(conn, repo.id)?;
    let manifest = if app.store.has_segment_store() {
        vcsstar_store::segments::manifest(conn, repo.id)?
    } else {
        Vec::new()
    };
    Ok(app
        .store
        .materialize(repo.id, &refs, &repo.default_branch, &manifest)?)
}

/// Record server-made objects (merges, imports) as a history segment.
pub fn capture(
    app: &App,
    conn: &mut PgConnection,
    repo_id: uuid::Uuid,
    dir: &std::path::Path,
    have: &[RevId],
    want: &[RevId],
) -> Result<(), Error> {
    if let Some((key, size, hash)) = app.store.capture_objects(repo_id, dir, have, want)? {
        vcsstar_store::segments::append(conn, repo_id, &key, size, &hash, "pack")?;
    }
    Ok(())
}

/// Upload one segment with every object the live segments do not hold.
/// Returns (object key, size, object count) when a segment was written.
pub fn capture_missing(
    app: &App,
    conn: &mut PgConnection,
    repo: &Repo,
    dir: &std::path::Path,
) -> Result<Option<(String, i64, usize)>, Error> {
    if !app.store.has_segment_store() {
        return Ok(None);
    }
    let manifest = vcsstar_store::segments::manifest(conn, repo.id)?;
    let want: Vec<RevId> = vcsstar_store::refs::load(conn, repo.id)?
        .into_iter()
        .map(|(_, target)| target)
        .collect();
    match app.store.capture_missing(repo.id, dir, &want, &manifest)? {
        Some((key, size, hash, objects)) => {
            vcsstar_store::segments::append(conn, repo.id, &key, size, &hash, "pack")?;
            Ok(Some((key, size, objects)))
        }
        None => Ok(None),
    }
}

pub fn repo_ref(org: &Org, repo: &Repo) -> vcsstar_types::RepoRef {
    vcsstar_types::RepoRef {
        instance: None,
        org: org.name.clone(),
        name: repo.name.clone(),
    }
}

pub fn ssh_url(app: &App, org: &Org, repo: &Repo) -> Option<String> {
    app.config.ssh_public_host.as_ref().map(|host| {
        if let Some((h, port)) = host.rsplit_once(':') {
            if port.chars().all(|c| c.is_ascii_digit()) && port != "22" {
                return format!("ssh://git@{h}:{port}/{}/{}.git", org.name, repo.name);
            }
        }
        format!("git@{host}:{}/{}.git", org.name, repo.name)
    })
}

pub fn commit_to_wire(
    r: &RevisionInfo,
    signature: vcsstar_types::Signature,
) -> vcsstar_types::Commit {
    vcsstar_types::Commit {
        id: r.id.0.clone(),
        parents: r.parents.iter().map(|p| p.0.clone()).collect(),
        tree: r.tree.clone(),
        author_name: r.author_name.clone(),
        author_email: r.author_email.clone(),
        authored_at: r.authored_at_ms,
        committer_name: r.committer_name.clone(),
        committer_email: r.committer_email.clone(),
        committed_at: r.committed_at_ms,
        message: r.message.clone(),
        signature,
    }
}

pub fn parse_rev(s: &str) -> Result<RevId, Error> {
    if s.is_empty() || s.len() > 200 {
        return Err(Error::invalid("rev is required"));
    }
    Ok(RevId(s.to_string()))
}
