//! Branch policy shared by Git pushes, RPC ref updates, merges, and sync.
use crate::{
    principal::Principal,
    services::repos::{protections_for, ProtectionRow},
    Error,
};
use diesel::prelude::*;
use vcsstar_db::{
    models::Repo,
    schema::{repo_refs, repos},
};
use vcsstar_store::refs::RefUpdate;

pub enum Source<'a> {
    Direct,
    Merge,
    Connection(&'a str),
}
pub fn matches(rule: &ProtectionRow, name: &str, default: &str) -> bool {
    if rule.ref_pattern == "@default" {
        name == format!("refs/heads/{default}")
    } else {
        vcsstar_vcs::ref_pattern_matches(&rule.ref_pattern, name)
    }
}
pub fn override_allowed(rule: &ProtectionRow, actor: Option<&Principal>) -> bool {
    actor.is_some_and(|p| {
        rule.override_accounts
            .iter()
            .flatten()
            .any(|k| k == &p.key())
    })
}
pub fn lock(conn: &mut PgConnection, repo: uuid::Uuid) -> Result<Repo, Error> {
    Ok(repos::table
        .find(repo)
        .for_update()
        .select(Repo::as_select())
        .first(conn)?)
}
pub fn check_change(
    conn: &mut PgConnection,
    repo: &Repo,
    actor: Option<&Principal>,
    update: &RefUpdate,
    forced: bool,
    source: Source<'_>,
) -> Result<(), Error> {
    let repo = lock(conn, repo.id)?;
    if repo.archived {
        return Err(Error::forbidden("repository is archived"));
    }
    let rules = protections_for(conn, repo.id)?;
    let empty: bool = !diesel::select(diesel::dsl::exists(
        repo_refs::table.filter(repo_refs::repo_id.eq(repo.id)),
    ))
    .get_result::<bool>(conn)?;
    for rule in rules
        .iter()
        .filter(|r| matches(r, &update.name, &repo.default_branch))
    {
        let authority = match source {
            Source::Connection(id) => id,
            _ => "local",
        };
        if authority != rule.merge_authority {
            return Err(Error::forbidden(
                "only the configured merge authority can update this branch",
            ));
        }
        if override_allowed(rule, actor) {
            continue;
        }
        if update.new.is_none() && !rule.allow_delete {
            return Err(Error::forbidden("branch protection prevents deletion"));
        }
        if forced && !rule.allow_force_push {
            return Err(Error::forbidden("branch protection prevents force updates"));
        }
        // An empty repository needs one initial branch before a PR can exist.
        let initial = empty
            && update.old.is_none()
            && update.new.is_some()
            && update.name == format!("refs/heads/{}", repo.default_branch);
        if matches!(source, Source::Direct)
            && !initial
            && update.new.is_some()
            && (rule.require_pull_request
                || rule.required_approvals > 0
                || !rule.required_checks.is_empty())
        {
            return Err(Error::forbidden(
                "branch protection requires a pull request",
            ));
        }
    }
    Ok(())
}
