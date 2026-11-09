//! The repository store. Durable truth is Postgres (`repo_refs`, CAS) and,
//! from phase 3b, immutable history segments in an object store. Node disks
//! hold a disposable cache directory per repository that the VCS backend
//! operates on.
//!
//! Phase 1 ships the local-disk implementation: the cache directory also
//! holds the objects. Everything above this crate already treats the
//! directory as a cache, so the segment store slots in underneath.

pub mod hook;
pub mod refs;
pub mod segments;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;
use vcsstar_vcs::{RevId, VcsBackend, VcsError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("vcs: {0}")]
    Vcs(#[from] VcsError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Db(#[from] diesel::result::Error),
    #[error("compare-and-swap failed on {0}")]
    Cas(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Where repositories live on this node.
pub struct LocalStore {
    root: PathBuf,
    backend: Arc<dyn VcsBackend>,
    /// Durable history. `None` keeps objects on the node disk only (dev).
    segments: Option<Arc<dyn segments::SegmentStore>>,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>, backend: Arc<dyn VcsBackend>) -> Self {
        Self {
            root: root.into(),
            backend,
            segments: None,
        }
    }

    pub fn with_segments(mut self, store: Arc<dyn segments::SegmentStore>) -> Self {
        self.segments = Some(store);
        self
    }

    pub fn segment_store(&self) -> Option<&Arc<dyn segments::SegmentStore>> {
        self.segments.as_ref()
    }

    pub fn has_segment_store(&self) -> bool {
        self.segments.is_some()
    }

    /// Download every manifest segment this directory lacks into
    /// `objects/pack`. Idempotent; safe to call before every operation.
    pub fn hydrate_segments(&self, dir: &Path, manifest: &[segments::Segment]) -> Result<usize> {
        let Some(store) = &self.segments else {
            return Ok(0);
        };
        let pack_dir = dir.join("objects").join("pack");
        std::fs::create_dir_all(&pack_dir)?;
        let mut fetched = 0;
        for seg in manifest {
            let stem = seg
                .object_key
                .rsplit('/')
                .next()
                .unwrap_or(&seg.object_key)
                .trim_end_matches(".pack");
            let pack = pack_dir.join(format!("{stem}.pack"));
            let idx = pack_dir.join(format!("{stem}.idx"));
            if pack.exists() && idx.exists() {
                continue;
            }
            let pack_bytes = store.get(&seg.object_key)?;
            let idx_bytes = store.get(&segments::idx_key(&seg.object_key))?;
            // Write idx last: git ignores a pack without its idx.
            write_atomic(&pack, &pack_bytes)?;
            write_atomic(&idx, &idx_bytes)?;
            fetched += 1;
        }
        Ok(fetched)
    }

    /// Upload one pack pair as a segment. Returns (key, size, hash).
    pub fn upload_pack(
        &self,
        repo_id: Uuid,
        files: &segments::PackFiles,
    ) -> Result<(String, i64, String)> {
        let store = self
            .segments
            .as_ref()
            .ok_or_else(|| StoreError::Other("no segment store".into()))?;
        let pack = std::fs::read(&files.pack)?;
        let idx = std::fs::read(&files.idx)?;
        let key = segments::object_key(repo_id, &files.stem);
        store.put(&segments::idx_key(&key), &idx)?;
        store.put(&key, &pack)?;
        Ok((key, pack.len() as i64, segments::pack_hash(&files.stem)))
    }

    /// Pack the objects reachable from `want` but not from `have` into a new
    /// pack in `objects/pack`, upload it, and return its manifest facts.
    /// `None` when nothing new exists.
    pub fn capture_objects(
        &self,
        repo_id: Uuid,
        dir: &Path,
        have: &[RevId],
        want: &[RevId],
    ) -> Result<Option<(String, i64, String)>> {
        if self.segments.is_none() || want.is_empty() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        self.backend.export_pack(dir, have, want, &mut bytes)?;
        // An empty pack is 12 header bytes + 20 trailer bytes.
        if bytes.len() <= 32 {
            return Ok(None);
        }
        let stem = self
            .backend
            .import_pack(dir, &mut bytes.as_slice())?
            .ok_or_else(|| StoreError::Other("index-pack reported no pack name".into()))?;
        let pack_dir = dir.join("objects").join("pack");
        let files = segments::PackFiles {
            pack: pack_dir.join(format!("{stem}.pack")),
            idx: pack_dir.join(format!("{stem}.idx")),
            stem,
        };
        if !files.pack.exists() || !files.idx.exists() {
            return Err(StoreError::Other(format!(
                "indexed pack {} is missing from {}",
                files.stem,
                pack_dir.display()
            )));
        }
        Ok(Some(self.upload_pack(repo_id, &files)?))
    }

    /// Pack the objects reachable from `want` that no live segment holds,
    /// upload the pack, and return (key, size, hash, object count). `None`
    /// when the segments already cover everything or no segment store exists.
    /// The manifest packs must be present in `dir` (hydrate first).
    pub fn capture_missing(
        &self,
        repo_id: Uuid,
        dir: &Path,
        want: &[RevId],
        manifest: &[segments::Segment],
    ) -> Result<Option<(String, i64, String, usize)>> {
        if self.segments.is_none() || want.is_empty() {
            return Ok(None);
        }
        let pack_dir = dir.join("objects").join("pack");
        let covered: Vec<PathBuf> = manifest
            .iter()
            .map(|seg| {
                let stem = seg
                    .object_key
                    .rsplit('/')
                    .next()
                    .unwrap_or(&seg.object_key)
                    .trim_end_matches(".pack");
                pack_dir.join(format!("{stem}.idx"))
            })
            .filter(|p| p.exists())
            .collect();
        let mut bytes = Vec::new();
        let objects = self
            .backend
            .export_pack_excluding(dir, want, &covered, &mut bytes)?;
        if objects == 0 || bytes.len() <= 32 {
            return Ok(None);
        }
        let stem = self
            .backend
            .import_pack(dir, &mut bytes.as_slice())?
            .ok_or_else(|| StoreError::Other("index-pack reported no pack name".into()))?;
        let files = segments::PackFiles {
            pack: pack_dir.join(format!("{stem}.pack")),
            idx: pack_dir.join(format!("{stem}.idx")),
            stem,
        };
        let (key, size, hash) = self.upload_pack(repo_id, &files)?;
        Ok(Some((key, size, hash, objects)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn backend(&self) -> &dyn VcsBackend {
        self.backend.as_ref()
    }

    /// Cache directory of one repository. Two-level fan-out keeps directory
    /// listings small at scale.
    pub fn repo_dir(&self, repo_id: Uuid) -> PathBuf {
        let s = repo_id.simple().to_string();
        self.root
            .join("repos")
            .join(&s[..2])
            .join(format!("{s}.git"))
    }

    /// Directory with the hook scripts every repository uses.
    pub fn hooks_dir(&self) -> PathBuf {
        self.root.join("hooks")
    }

    /// Write the hook scripts. `hook_command` is the executable (with any
    /// leading arguments) the hooks exec, e.g. `/usr/local/bin/vcsstard hook`.
    pub fn install_hooks(&self, hook_command: &str) -> Result<()> {
        let dir = self.hooks_dir();
        std::fs::create_dir_all(&dir)?;
        let name = "pre-receive";
        let path = dir.join(name);
        let script = format!("#!/bin/sh\nexec {hook_command} {name} \"$@\"\n");
        std::fs::write(&path, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    /// Make sure the cache directory exists and is a repository.
    pub fn ensure(&self, repo_id: Uuid) -> Result<PathBuf> {
        let dir = self.repo_dir(repo_id);
        if !dir.join("HEAD").exists() {
            self.backend.init(&dir)?;
        }
        Ok(dir)
    }

    /// Materialize refs from Postgres into the directory, after downloading
    /// the history segments the node lacks. Called before any operation;
    /// the directory is a projection, never the truth.
    pub fn materialize(
        &self,
        repo_id: Uuid,
        refs: &[(String, RevId)],
        default_branch: &str,
        manifest: &[segments::Segment],
    ) -> Result<PathBuf> {
        let dir = self.ensure(repo_id)?;
        let fetched = self.hydrate_segments(&dir, manifest)?;
        if fetched > 0 {
            tracing::info!(repo = %repo_id, segments = fetched, "hydrated history segments");
        }
        // Refs whose objects are missing locally (another node accepted the
        // push and this cache is stale) are skipped until hydration brings
        // the objects; the next materialize picks them up.
        let present: Vec<(String, RevId)> = refs
            .iter()
            .filter(|(_, id)| self.backend.object_exists(&dir, id, &Vec::new()))
            .cloned()
            .collect();
        if present.len() != refs.len() {
            tracing::warn!(
                repo = %repo_id,
                missing = refs.len() - present.len(),
                "refs point at objects this node does not have yet"
            );
        }
        self.backend.write_refs(&dir, &present, default_branch)?;
        Ok(dir)
    }

    pub fn fork(&self, src: Uuid, dst: Uuid) -> Result<PathBuf> {
        let s = self.repo_dir(src);
        let d = self.repo_dir(dst);
        if d.exists() {
            std::fs::remove_dir_all(&d)?;
        }
        self.backend.fork(&s, &d)?;
        Ok(d)
    }

    pub fn remove(&self, repo_id: Uuid) -> Result<()> {
        let dir = self.repo_dir(repo_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Encode a RevId for the `repo_refs.target` bytea column.
pub fn target_bytes(id: &RevId) -> Vec<u8> {
    id.0.as_bytes().to_vec()
}

pub fn target_from_bytes(b: &[u8]) -> RevId {
    RevId(String::from_utf8_lossy(b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "A")
            .env("GIT_AUTHOR_EMAIL", "a@x")
            .env("GIT_COMMITTER_NAME", "A")
            .env("GIT_COMMITTER_EMAIL", "a@x")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// History written before the segment store existed is backfilled by
    /// `capture_missing`, and a second call finds nothing to do.
    #[test]
    fn capture_missing_backfills_uncovered_history() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let backend: Arc<dyn VcsBackend> = Arc::new(vcsstar_vcs::GitBackend::default());
        // Node A has no segment store yet: history lands on disk only.
        let plain = LocalStore::new(tmp.path().join("a"), Arc::clone(&backend));
        let repo_id = Uuid::now_v7();
        let dir_a = plain.ensure(repo_id).unwrap();
        let work = tmp.path().join("w");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("f"), "1\n").unwrap();
        git(&work, &["add", "f"]);
        git(&work, &["commit", "-q", "-m", "one"]);
        let c1 = RevId(git(&work, &["rev-parse", "HEAD"]));
        git(
            &work,
            &[
                "push",
                "-q",
                dir_a.to_str().unwrap(),
                "main:refs/heads/main",
            ],
        );
        // The store arrives; the first incremental segment covers only c2.
        let mem = Arc::new(segments::MemoryStore::default());
        let node_a =
            LocalStore::new(tmp.path().join("a"), Arc::clone(&backend)).with_segments(mem.clone());
        std::fs::write(work.join("f"), "2\n").unwrap();
        git(&work, &["commit", "-q", "-am", "two"]);
        let c2 = RevId(git(&work, &["rev-parse", "HEAD"]));
        git(
            &work,
            &[
                "push",
                "-q",
                dir_a.to_str().unwrap(),
                "main:refs/heads/main",
            ],
        );
        let seg2 = node_a
            .capture_objects(
                repo_id,
                &dir_a,
                std::slice::from_ref(&c1),
                std::slice::from_ref(&c2),
            )
            .unwrap()
            .expect("incremental segment");
        let manifest = vec![seg_of(repo_id, &seg2, 1)];
        // Backfill: c1's objects are reachable but in no segment.
        let (key, size, hash, objects) = node_a
            .capture_missing(repo_id, &dir_a, std::slice::from_ref(&c2), &manifest)
            .unwrap()
            .expect("backfill segment");
        assert!(objects >= 3, "commit, tree, blob of c1: {objects}");
        assert!(size > 32);
        let manifest = vec![manifest[0].clone(), seg_of(repo_id, &(key, size, hash), 2)];
        assert!(node_a
            .capture_missing(repo_id, &dir_a, std::slice::from_ref(&c2), &manifest)
            .unwrap()
            .is_none());
        // A fresh node materializes the whole history from the segments.
        let node_b = LocalStore::new(tmp.path().join("b"), Arc::clone(&backend)).with_segments(mem);
        let dir_b = node_b
            .materialize(
                repo_id,
                &[("refs/heads/main".to_string(), c2.clone())],
                "main",
                &manifest,
            )
            .unwrap();
        assert_eq!(git(&dir_b, &["rev-parse", "main"]), c2.0);
        assert_eq!(git(&dir_b, &["rev-list", "--count", "main"]), "2");
        assert!(git(&dir_b, &["cat-file", "-t", &c1.0]) == "commit");
    }

    fn seg_of(repo_id: Uuid, s: &(String, i64, String), ordinal: i32) -> segments::Segment {
        segments::Segment {
            repo_id,
            ordinal,
            object_key: s.0.clone(),
            size: s.1,
            content_hash: s.2.clone(),
            kind: "pack".into(),
            created_at: chrono::Utc::now(),
            superseded_at: None,
        }
    }

    /// Objects captured on one node hydrate a fresh directory on another
    /// from the segment store alone.
    #[test]
    fn segments_round_trip_between_cache_dirs() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let mem = Arc::new(segments::MemoryStore::default());
        let backend: Arc<dyn VcsBackend> = Arc::new(vcsstar_vcs::GitBackend::default());
        let node_a =
            LocalStore::new(tmp.path().join("a"), Arc::clone(&backend)).with_segments(mem.clone());
        let node_b =
            LocalStore::new(tmp.path().join("b"), Arc::clone(&backend)).with_segments(mem.clone());
        let repo_id = Uuid::now_v7();
        let dir_a = node_a.ensure(repo_id).unwrap();

        // Make history in a work tree and push it into node A's cache dir.
        let work = tmp.path().join("w");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("f"), "1\n").unwrap();
        git(&work, &["add", "f"]);
        git(&work, &["commit", "-q", "-m", "one"]);
        let c1 = RevId(git(&work, &["rev-parse", "HEAD"]));
        git(
            &work,
            &[
                "push",
                "-q",
                dir_a.to_str().unwrap(),
                "main:refs/heads/main",
            ],
        );
        let seg1 = node_a
            .capture_objects(repo_id, &dir_a, &[], std::slice::from_ref(&c1))
            .unwrap()
            .expect("first segment");
        // A second commit becomes an incremental segment.
        std::fs::write(work.join("f"), "2\n").unwrap();
        git(&work, &["commit", "-q", "-am", "two"]);
        let c2 = RevId(git(&work, &["rev-parse", "HEAD"]));
        git(
            &work,
            &[
                "push",
                "-q",
                dir_a.to_str().unwrap(),
                "main:refs/heads/main",
            ],
        );
        let seg2 = node_a
            .capture_objects(
                repo_id,
                &dir_a,
                std::slice::from_ref(&c1),
                std::slice::from_ref(&c2),
            )
            .unwrap()
            .expect("second segment");
        assert_ne!(seg1.0, seg2.0);
        // Nothing new: no segment.
        assert!(node_a
            .capture_objects(
                repo_id,
                &dir_a,
                std::slice::from_ref(&c2),
                std::slice::from_ref(&c2)
            )
            .unwrap()
            .is_none());

        let manifest: Vec<segments::Segment> = [seg1, seg2]
            .iter()
            .enumerate()
            .map(|(i, (key, size, hash))| segments::Segment {
                repo_id,
                ordinal: i as i32 + 1,
                object_key: key.clone(),
                size: *size,
                content_hash: hash.clone(),
                kind: "pack".into(),
                created_at: chrono::Utc::now(),
                superseded_at: None,
            })
            .collect();
        let dir_b = node_b
            .materialize(
                repo_id,
                &[("refs/heads/main".into(), c2.clone())],
                "main",
                &manifest,
            )
            .unwrap();
        assert!(backend.object_exists(&dir_b, &c2, &Vec::new()));
        assert!(backend.object_exists(&dir_b, &c1, &Vec::new()));
        assert_eq!(
            backend.list_refs(&dir_b).unwrap(),
            vec![("refs/heads/main".to_string(), c2.clone())]
        );
        // Second hydration downloads nothing.
        assert_eq!(node_b.hydrate_segments(&dir_b, &manifest).unwrap(), 0);
    }
}
