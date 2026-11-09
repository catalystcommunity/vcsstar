//! The VCS layer: one trait scoped to the DVCS tier (DAG of revisions with
//! opaque ids, named refs with compare-and-set, transferable history,
//! ancestry merges, a pre-transaction hook) and the git backend.
//!
//! Every `dir` a backend receives is a local cache directory, never the
//! durable store. Refs are materialized into it from Postgres before an
//! operation and recorded back through the pre-receive transaction.

pub mod git;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;

pub use git::GitBackend;

/// Opaque revision id in the backend's canonical text form (hex for git).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RevId(pub String);

impl RevId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RevId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{tool} failed ({status}): {stderr}")]
    Tool {
        tool: String,
        status: i32,
        stderr: String,
    },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, VcsError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcsCaps {
    pub merge_commit: bool,
    pub squash: bool,
    pub rebase: bool,
    pub signed_revs: bool,
    pub shallow: bool,
    pub shared_objects: bool,
}

/// Full revision metadata: enough for attribution and verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionInfo {
    pub id: RevId,
    pub parents: Vec<RevId>,
    pub tree: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at_ms: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at_ms: i64,
    pub message: String,
    /// Raw signature block (`gpgsig` header for git), when present.
    pub signature: Option<String>,
}

impl RevisionInfo {
    pub fn message_head(&self) -> String {
        self.message.lines().next().unwrap_or("").to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: String,
    /// "blob" | "tree" | "symlink" | "submodule"
    pub kind: String,
    pub id: String,
    pub mode: u32,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiffInfo {
    /// "added" | "modified" | "deleted" | "renamed" | "binary"
    pub status: String,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    Merge,
    Squash,
    Rebase,
}

impl MergeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "merge" => Some(MergeKind::Merge),
            "squash" => Some(MergeKind::Squash),
            "rebase" => Some(MergeKind::Rebase),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            MergeKind::Merge => "merge",
            MergeKind::Squash => "squash",
            MergeKind::Rebase => "rebase",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Clean(RevId),
    Conflicts(Vec<String>),
}

/// Identity and message for a server-made commit.
#[derive(Debug, Clone)]
pub struct CommitMsg {
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
}

/// SSH signing key for server-made commits. The private key file has no
/// passphrase; it is the instance's application key (see plan/01).
#[derive(Debug, Clone)]
pub struct Signer {
    pub private_key_path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigStatus {
    /// Good signature by the named principal (from the allowed-signers file).
    Good {
        principal: String,
        fingerprint: String,
    },
    /// A signature exists but does not verify against the allowed signers.
    Bad { reason: String },
    /// No signature on the revision.
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireService {
    UploadPack,
    ReceivePack,
}

impl WireService {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "git-upload-pack" => Some(WireService::UploadPack),
            "git-receive-pack" => Some(WireService::ReceivePack),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            WireService::UploadPack => "git-upload-pack",
            WireService::ReceivePack => "git-receive-pack",
        }
    }
}

/// Extra process environment for operations that must see objects a hook
/// is inspecting (git quarantine) or that carry the push context.
pub type Env = Vec<(String, String)>;

pub trait VcsBackend: Send + Sync {
    fn vcs_type(&self) -> &'static str;
    fn caps(&self) -> VcsCaps;

    // repository lifecycle (operates on a local cache dir)
    fn init(&self, dir: &Path) -> Result<()>;
    fn fork(&self, src: &Path, dst: &Path) -> Result<()>;

    // refs
    fn list_refs(&self, dir: &Path) -> Result<Vec<(String, RevId)>>;
    /// Make the directory's refs exactly `refs` and point HEAD at `head`.
    fn write_refs(&self, dir: &Path, refs: &[(String, RevId)], head: &str) -> Result<()>;
    fn resolve(&self, dir: &Path, rev: &str, env: &Env) -> Result<RevId>;
    fn object_exists(&self, dir: &Path, id: &RevId, env: &Env) -> bool;

    // history
    fn revision(&self, dir: &Path, id: &RevId, env: &Env) -> Result<RevisionInfo>;
    fn is_ancestor(
        &self,
        dir: &Path,
        ancestor: &RevId,
        descendant: &RevId,
        env: &Env,
    ) -> Result<bool>;
    fn common_ancestor(&self, dir: &Path, a: &RevId, b: &RevId) -> Result<Option<RevId>>;
    /// Revisions reachable from `start`, newest first, optionally limited to
    /// those touching `path`.
    fn log(
        &self,
        dir: &Path,
        start: &RevId,
        path: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RevisionInfo>>;
    /// Revisions in `new` that are not in `old` (all history when `old` is
    /// absent), newest first, capped at `limit`.
    fn rev_list(
        &self,
        dir: &Path,
        old: Option<&RevId>,
        new: &RevId,
        limit: usize,
        env: &Env,
    ) -> Result<Vec<RevisionInfo>>;
    fn tree_at(&self, dir: &Path, id: &RevId, path: &str) -> Result<Vec<TreeEntry>>;
    /// Blob content by path at a revision, with its object id.
    fn blob(&self, dir: &Path, id: &RevId, path: &str) -> Result<(String, Vec<u8>)>;
    fn diff_stat(
        &self,
        dir: &Path,
        from: &RevId,
        to: &RevId,
        path: Option<&str>,
    ) -> Result<Vec<FileDiffInfo>>;
    fn diff_patch(&self, dir: &Path, from: &RevId, to: &RevId, path: &str) -> Result<String>;

    // merges (only kinds in caps())
    #[allow(clippy::too_many_arguments)]
    fn merge(
        &self,
        dir: &Path,
        kind: MergeKind,
        base: &RevId,
        head: &RevId,
        msg: &CommitMsg,
        signer: Option<&Signer>,
        trailers: &[(String, String)],
    ) -> Result<MergeOutcome>;

    // signatures (only if signed_revs)
    fn verify_signature(
        &self,
        dir: &Path,
        id: &RevId,
        allowed_signers: &Path,
        env: &Env,
    ) -> Result<SigStatus>;

    // history transfer
    fn export_pack(
        &self,
        dir: &Path,
        have: &[RevId],
        want: &[RevId],
        out: &mut dyn Write,
    ) -> Result<()>;
    /// Index a pack into `objects/pack`. Returns the pack name stem
    /// (`pack-<hash>`) when the backend reports it.
    fn import_pack(&self, dir: &Path, input: &mut dyn Read) -> Result<Option<String>>;

    /// Write a pack with every object reachable from `want` that none of the
    /// `covered_idx` pack indexes holds. Returns the object count; zero means
    /// nothing was written.
    fn export_pack_excluding(
        &self,
        dir: &Path,
        want: &[RevId],
        covered_idx: &[PathBuf],
        out: &mut dyn Write,
    ) -> Result<usize>;

    // wire protocol: the caller owns the pipes
    fn spawn_service(
        &self,
        dir: &Path,
        svc: WireService,
        stateless_rpc: bool,
        advertise_refs: bool,
        env: &Env,
    ) -> Result<Child>;
}

/// Match a protection pattern against a full ref name. A pattern without a
/// `refs/` prefix is a branch pattern. `*` matches within one segment and
/// `**` matches across segments.
pub fn ref_pattern_matches(pattern: &str, full_ref: &str) -> bool {
    let pattern = if pattern.starts_with("refs/") {
        pattern.to_string()
    } else {
        format!("refs/heads/{pattern}")
    };
    glob(&pattern, full_ref)
}

fn glob(p: &str, s: &str) -> bool {
    fn inner(p: &[char], s: &[char]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some(('*', rest)) if rest.first() == Some(&'*') => {
                let rest = &rest[1..];
                let rest = if rest.first() == Some(&'/') {
                    &rest[1..]
                } else {
                    rest
                };
                (0..=s.len()).any(|i| inner(rest, &s[i..]))
            }
            Some(('*', rest)) => (0..=s.len())
                .take_while(|&i| i == 0 || s[i - 1] != '/')
                .any(|i| inner(rest, &s[i..])),
            Some((c, rest)) => s.first() == Some(c) && inner(rest, &s[1..]),
        }
    }
    let p: Vec<char> = p.chars().collect();
    let s: Vec<char> = s.chars().collect();
    inner(&p, &s)
}

/// Name checks shared by the API and the hook.
pub fn check_ref_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || !name.starts_with("refs/")
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
        || name
            .chars()
            .any(|c| c.is_control() || c == ' ' || "~^:?*[\\".contains(c));
    if bad {
        Err(VcsError::Invalid(format!("bad ref name {name:?}")))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(ref_pattern_matches("main", "refs/heads/main"));
        assert!(!ref_pattern_matches("main", "refs/heads/maint"));
        assert!(ref_pattern_matches("release/*", "refs/heads/release/1.2"));
        assert!(!ref_pattern_matches("release/*", "refs/heads/release/1/2"));
        assert!(ref_pattern_matches("release/**", "refs/heads/release/1/2"));
        assert!(ref_pattern_matches("refs/tags/v*", "refs/tags/v1.0"));
        assert!(!ref_pattern_matches("refs/tags/v*", "refs/heads/v1.0"));
    }

    #[test]
    fn ref_names() {
        assert!(check_ref_name("refs/heads/main").is_ok());
        assert!(check_ref_name("refs/heads/feature/x").is_ok());
        assert!(check_ref_name("main").is_err());
        assert!(check_ref_name("refs/heads/a..b").is_err());
        assert!(check_ref_name("refs/heads/a b").is_err());
        assert!(check_ref_name("refs/heads/x.lock").is_err());
    }
}
