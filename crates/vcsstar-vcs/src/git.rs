//! Git backend over the system `git` binary. The wire protocol, packs, and
//! merges come from git itself; vcsstar adds no protocol code.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::{
    CommitMsg, Env, FileDiffInfo, MergeKind, MergeOutcome, Result, RevId, RevisionInfo, SigStatus,
    Signer, TreeEntry, VcsBackend, VcsCaps, VcsError, WireService,
};

#[derive(Debug, Default, Clone)]
pub struct GitBackend {
    /// Path of the hooks directory every repository uses (`core.hooksPath`).
    pub hooks_path: Option<std::path::PathBuf>,
}

/// `--format` with 0x1f between fields and 0x1e between records.
const LOG_FORMAT: &str = "%H%x1f%P%x1f%T%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%B%x1e";

fn git() -> Command {
    let mut c = Command::new("git");
    // Never read the user's or system's configuration; every setting we
    // need is set explicitly below.
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("HOME", std::env::temp_dir())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    c
}

fn run(dir: &Path, env: &Env, args: &[&str]) -> Result<Vec<u8>> {
    let mut c = git();
    c.arg("-C").arg(dir).args(args).envs(env.iter().cloned());
    let out = c.output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(tool_err("git", &out.status, &out.stderr))
    }
}

fn run_str(dir: &Path, env: &Env, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&run(dir, env, args)?).into_owned())
}

fn run_stdin(dir: &Path, env: &Env, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut c = git();
    c.arg("-C")
        .arg(dir)
        .args(args)
        .envs(env.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn()?;
    {
        let mut stdin = child.stdin.take().expect("piped");
        stdin.write_all(input)?;
    }
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(tool_err("git", &out.status, &out.stderr))
    }
}

fn tool_err(tool: &str, status: &std::process::ExitStatus, stderr: &[u8]) -> VcsError {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if stderr.contains("not a valid object name")
        || stderr.contains("does not exist")
        || stderr.contains("unknown revision")
        || stderr.contains("bad revision")
        || stderr.contains("Not a valid object")
        || stderr.contains("exists on disk, but not in")
    {
        return VcsError::NotFound(stderr);
    }
    VcsError::Tool {
        tool: tool.to_string(),
        status: status.code().unwrap_or(-1),
        stderr,
    }
}

fn parse_revisions(raw: &str) -> Vec<RevisionInfo> {
    raw.split('\x1e')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|r| {
            let r = r.strip_prefix('\n').unwrap_or(r);
            let f: Vec<&str> = r.splitn(10, '\x1f').collect();
            if f.len() < 10 {
                return None;
            }
            Some(RevisionInfo {
                id: RevId(f[0].to_string()),
                parents: f[1]
                    .split_whitespace()
                    .map(|p| RevId(p.to_string()))
                    .collect(),
                tree: f[2].to_string(),
                author_name: f[3].to_string(),
                author_email: f[4].to_string(),
                authored_at_ms: f[5].parse::<i64>().unwrap_or(0) * 1000,
                committer_name: f[6].to_string(),
                committer_email: f[7].to_string(),
                committed_at_ms: f[8].parse::<i64>().unwrap_or(0) * 1000,
                message: f[9].trim_end_matches('\n').to_string(),
                signature: None,
            })
        })
        .collect()
}

fn check_rev(rev: &str) -> Result<()> {
    if rev.is_empty()
        || rev.starts_with('-')
        || rev.contains("..")
        || rev.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(VcsError::Invalid(format!("bad revision {rev:?}")));
    }
    Ok(())
}

fn check_path(path: &str) -> Result<()> {
    if path.starts_with('/')
        || path.split('/').any(|seg| seg == "..")
        || path.contains('\0')
        || path.starts_with('-')
    {
        return Err(VcsError::Invalid(format!("bad path {path:?}")));
    }
    Ok(())
}

impl GitBackend {
    pub fn new(hooks_path: Option<std::path::PathBuf>) -> Self {
        Self { hooks_path }
    }

    fn configure(&self, dir: &Path) -> Result<()> {
        let env = Env::new();
        let settings: Vec<(&str, String)> = vec![
            // Bare repositories only; the hook is the transaction point.
            ("core.bare", "true".into()),
            ("receive.denyNonFastForwards", "false".into()),
            ("receive.denyDeletes", "false".into()),
            ("receive.advertisePushOptions", "true".into()),
            // Keep every push as a pack so the quarantine holds one segment.
            ("transfer.unpackLimit", "1".into()),
            ("receive.fsckObjects", "true".into()),
            ("uploadpack.allowFilter", "true".into()),
            ("uploadpack.allowAnySHA1InWant", "false".into()),
            ("gc.auto", "0".into()),
            ("protocol.version", "2".into()),
            ("gpg.format", "ssh".into()),
        ];
        for (k, v) in settings {
            run(dir, &env, &["config", k, &v])?;
        }
        match &self.hooks_path {
            Some(p) => {
                run(
                    dir,
                    &env,
                    &["config", "core.hooksPath", &p.to_string_lossy()],
                )?;
            }
            None => {
                let _ = run(dir, &env, &["config", "--unset", "core.hooksPath"]);
            }
        }
        Ok(())
    }

    /// Raw commit object text (for signature extraction).
    fn cat_commit(&self, dir: &Path, id: &RevId, env: &Env) -> Result<String> {
        check_rev(&id.0)?;
        run_str(dir, env, &["cat-file", "commit", &id.0])
    }
}

impl VcsBackend for GitBackend {
    fn vcs_type(&self) -> &'static str {
        "git"
    }

    fn caps(&self) -> VcsCaps {
        VcsCaps {
            merge_commit: true,
            squash: true,
            rebase: true,
            signed_revs: true,
            shallow: true,
            shared_objects: true,
        }
    }

    fn init(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        if !dir.join("HEAD").exists() {
            run(
                dir,
                &Env::new(),
                &["init", "--bare", "--quiet", "-b", "main"],
            )?;
        }
        self.configure(dir)
    }

    fn fork(&self, src: &Path, dst: &Path) -> Result<()> {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut c = git();
        // Local clones hard-link immutable objects, so a fork is cheap.
        c.args(["clone", "--bare", "--quiet", "--no-local"])
            .arg(src)
            .arg(dst);
        let out = c.output()?;
        if !out.status.success() {
            return Err(tool_err("git clone", &out.status, &out.stderr));
        }
        // A fork has no remote; refs come from Postgres like any repo.
        let _ = run(dst, &Env::new(), &["remote", "remove", "origin"]);
        self.configure(dst)
    }

    fn list_refs(&self, dir: &Path) -> Result<Vec<(String, RevId)>> {
        let out = run_str(
            dir,
            &Env::new(),
            &["for-each-ref", "--format=%(objectname) %(refname)"],
        )?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let (id, name) = l.split_once(' ')?;
                Some((name.to_string(), RevId(id.to_string())))
            })
            .collect())
    }

    fn write_refs(&self, dir: &Path, refs: &[(String, RevId)], head: &str) -> Result<()> {
        let current = self.list_refs(dir)?;
        let mut script = String::from("start\n");
        let wanted: std::collections::HashMap<&str, &RevId> =
            refs.iter().map(|(n, t)| (n.as_str(), t)).collect();
        for (name, _) in &current {
            if !wanted.contains_key(name.as_str()) {
                script.push_str(&format!("delete {name}\n"));
            }
        }
        for (name, target) in refs {
            script.push_str(&format!("update {name} {target}\n"));
        }
        script.push_str("prepare\ncommit\n");
        run_stdin(
            dir,
            &Env::new(),
            &["update-ref", "--stdin", "--no-deref"],
            script.as_bytes(),
        )?;
        let head_ref = if head.starts_with("refs/") {
            head.to_string()
        } else {
            format!("refs/heads/{head}")
        };
        run(dir, &Env::new(), &["symbolic-ref", "HEAD", &head_ref])?;
        Ok(())
    }

    fn resolve(&self, dir: &Path, rev: &str, env: &Env) -> Result<RevId> {
        check_rev(rev)?;
        let out = run_str(
            dir,
            env,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &format!("{rev}^{{commit}}"),
            ],
        )?;
        let id = out.trim().to_string();
        if id.is_empty() {
            return Err(VcsError::NotFound(format!("revision {rev}")));
        }
        Ok(RevId(id))
    }

    fn object_exists(&self, dir: &Path, id: &RevId, env: &Env) -> bool {
        check_rev(&id.0).is_ok() && run(dir, env, &["cat-file", "-e", &id.0]).is_ok()
    }

    fn revision(&self, dir: &Path, id: &RevId, env: &Env) -> Result<RevisionInfo> {
        check_rev(&id.0)?;
        let raw = run_str(
            dir,
            env,
            &[
                "log",
                "-1",
                &format!("--format={LOG_FORMAT}"),
                "--end-of-options",
                &id.0,
            ],
        )?;
        let mut revs = parse_revisions(&raw);
        let mut rev = revs
            .pop()
            .ok_or_else(|| VcsError::NotFound(format!("revision {id}")))?;
        let commit = self.cat_commit(dir, &rev.id, env)?;
        rev.signature = extract_gpgsig(&commit);
        Ok(rev)
    }

    fn is_ancestor(
        &self,
        dir: &Path,
        ancestor: &RevId,
        descendant: &RevId,
        env: &Env,
    ) -> Result<bool> {
        check_rev(&ancestor.0)?;
        check_rev(&descendant.0)?;
        let mut c = git();
        c.arg("-C")
            .arg(dir)
            .args(["merge-base", "--is-ancestor", &ancestor.0, &descendant.0])
            .envs(env.iter().cloned());
        let out = c.output()?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(tool_err("git merge-base", &out.status, &out.stderr)),
        }
    }

    fn common_ancestor(&self, dir: &Path, a: &RevId, b: &RevId) -> Result<Option<RevId>> {
        check_rev(&a.0)?;
        check_rev(&b.0)?;
        let mut c = git();
        c.arg("-C").arg(dir).args(["merge-base", &a.0, &b.0]);
        let out = c.output()?;
        match out.status.code() {
            Some(0) => Ok(Some(RevId(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ))),
            Some(1) => Ok(None),
            _ => Err(tool_err("git merge-base", &out.status, &out.stderr)),
        }
    }

    fn log(
        &self,
        dir: &Path,
        start: &RevId,
        path: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RevisionInfo>> {
        check_rev(&start.0)?;
        let n = format!("-{}", limit.max(1));
        let fmt = format!("--format={LOG_FORMAT}");
        let mut args = vec!["log", &n, &fmt, "--end-of-options", &start.0];
        if let Some(p) = path {
            check_path(p)?;
            args.push("--");
            args.push(p);
        }
        let raw = run_str(dir, &Env::new(), &args)?;
        Ok(parse_revisions(&raw))
    }

    fn rev_list(
        &self,
        dir: &Path,
        old: Option<&RevId>,
        new: &RevId,
        limit: usize,
        env: &Env,
    ) -> Result<Vec<RevisionInfo>> {
        check_rev(&new.0)?;
        let n = format!("-{}", limit.max(1));
        let fmt = format!("--format={LOG_FORMAT}");
        let range = match old {
            Some(o) => {
                check_rev(&o.0)?;
                format!("{}..{}", o.0, new.0)
            }
            None => new.0.clone(),
        };
        let raw = run_str(dir, env, &["log", &n, &fmt, "--end-of-options", &range])?;
        Ok(parse_revisions(&raw))
    }

    fn tree_at(&self, dir: &Path, id: &RevId, path: &str) -> Result<Vec<TreeEntry>> {
        check_rev(&id.0)?;
        check_path(path)?;
        let spec = if path.is_empty() {
            id.0.clone()
        } else {
            format!("{}:{}", id.0, path.trim_end_matches('/'))
        };
        let out = run_str(dir, &Env::new(), &["ls-tree", "-l", "-z", &spec])?;
        let mut entries = Vec::new();
        for rec in out.split('\0').filter(|r| !r.is_empty()) {
            let Some((meta, name)) = rec.split_once('\t') else {
                continue;
            };
            let parts: Vec<&str> = meta.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }
            let mode = u32::from_str_radix(parts[0], 8).unwrap_or(0);
            let kind = match (parts[1], mode) {
                ("tree", _) => "tree",
                ("commit", _) => "submodule",
                (_, 0o120000) => "symlink",
                _ => "blob",
            };
            entries.push(TreeEntry {
                name: name.to_string(),
                kind: kind.to_string(),
                id: parts[2].to_string(),
                mode,
                size: parts[3].parse().ok(),
            });
        }
        Ok(entries)
    }

    fn blob(&self, dir: &Path, id: &RevId, path: &str) -> Result<(String, Vec<u8>)> {
        check_rev(&id.0)?;
        check_path(path)?;
        let spec = format!("{}:{}", id.0, path);
        let oid = run_str(
            dir,
            &Env::new(),
            &["rev-parse", "--verify", "--quiet", &spec],
        )?
        .trim()
        .to_string();
        if oid.is_empty() {
            return Err(VcsError::NotFound(format!("path {path}")));
        }
        let kind = run_str(dir, &Env::new(), &["cat-file", "-t", &oid])?;
        if kind.trim() != "blob" {
            return Err(VcsError::Invalid(format!("{path} is a {}", kind.trim())));
        }
        let content = run(dir, &Env::new(), &["cat-file", "blob", &oid])?;
        Ok((oid, content))
    }

    fn diff_stat(
        &self,
        dir: &Path,
        from: &RevId,
        to: &RevId,
        path: Option<&str>,
    ) -> Result<Vec<FileDiffInfo>> {
        check_rev(&from.0)?;
        check_rev(&to.0)?;
        let mut args = vec![
            "diff",
            "--numstat",
            "-M",
            "-z",
            "--end-of-options",
            &from.0,
            &to.0,
        ];
        if let Some(p) = path {
            check_path(p)?;
            args.push("--");
            args.push(p);
        }
        let out = run(dir, &Env::new(), &args)?;
        let text = String::from_utf8_lossy(&out);
        // numstat -z: "adds\tdels\tpath\0" or for renames "adds\tdels\t\0old\0new\0".
        let fields: Vec<&str> = text.split('\0').collect();
        let mut files = Vec::new();
        let mut i = 0;
        while i < fields.len() {
            let rec = fields[i];
            if rec.is_empty() {
                i += 1;
                continue;
            }
            let parts: Vec<&str> = rec.splitn(3, '\t').collect();
            if parts.len() < 3 {
                i += 1;
                continue;
            }
            let binary = parts[0] == "-";
            let additions = parts[0].parse().unwrap_or(0);
            let deletions = parts[1].parse().unwrap_or(0);
            let (old_path, new_path, status) = if parts[2].is_empty() {
                let old = fields.get(i + 1).copied().unwrap_or("");
                let new = fields.get(i + 2).copied().unwrap_or("");
                i += 2;
                (Some(old.to_string()), Some(new.to_string()), "renamed")
            } else {
                (
                    Some(parts[2].to_string()),
                    Some(parts[2].to_string()),
                    "modified",
                )
            };
            files.push(FileDiffInfo {
                status: if binary { "binary" } else { status }.to_string(),
                old_path,
                new_path,
                additions,
                deletions,
            });
            i += 1;
        }
        // Resolve added/deleted with --name-status (cheap second pass).
        let mut st_args = vec![
            "diff",
            "--name-status",
            "-M",
            "-z",
            "--end-of-options",
            &from.0,
            &to.0,
        ];
        if let Some(p) = path {
            st_args.push("--");
            st_args.push(p);
        }
        let st = run(dir, &Env::new(), &st_args)?;
        let st = String::from_utf8_lossy(&st);
        let toks: Vec<&str> = st.split('\0').filter(|s| !s.is_empty()).collect();
        let mut statuses = std::collections::HashMap::new();
        let mut j = 0;
        while j < toks.len() {
            let code = toks[j];
            if code.starts_with('R') || code.starts_with('C') {
                if let (Some(old), Some(new)) = (toks.get(j + 1), toks.get(j + 2)) {
                    statuses.insert(new.to_string(), ("renamed", Some(old.to_string())));
                }
                j += 3;
            } else {
                if let Some(p) = toks.get(j + 1) {
                    let s = match code.chars().next() {
                        Some('A') => "added",
                        Some('D') => "deleted",
                        _ => "modified",
                    };
                    statuses.insert(p.to_string(), (s, None));
                }
                j += 2;
            }
        }
        for f in &mut files {
            let key = f.new_path.clone().unwrap_or_default();
            if let Some((s, old)) = statuses.get(&key) {
                if f.status != "binary" {
                    f.status = s.to_string();
                }
                match *s {
                    "added" => f.old_path = None,
                    "deleted" => {
                        f.old_path = Some(key.clone());
                        f.new_path = None;
                    }
                    "renamed" => f.old_path = old.clone(),
                    _ => {}
                }
            }
        }
        Ok(files)
    }

    fn diff_patch(&self, dir: &Path, from: &RevId, to: &RevId, path: &str) -> Result<String> {
        check_rev(&from.0)?;
        check_rev(&to.0)?;
        check_path(path)?;
        run_str(
            dir,
            &Env::new(),
            &[
                "diff",
                "-M",
                "--no-color",
                "--end-of-options",
                &from.0,
                &to.0,
                "--",
                path,
            ],
        )
    }

    fn merge(
        &self,
        dir: &Path,
        kind: MergeKind,
        base: &RevId,
        head: &RevId,
        msg: &CommitMsg,
        signer: Option<&Signer>,
        trailers: &[(String, String)],
    ) -> Result<MergeOutcome> {
        check_rev(&base.0)?;
        check_rev(&head.0)?;
        let env = Env::new();
        let merge_tree =
            |a: &RevId, b: &RevId| -> Result<std::result::Result<String, Vec<String>>> {
                let mut c = git();
                c.arg("-C").arg(dir).args([
                    "merge-tree",
                    "--write-tree",
                    "-z",
                    "--name-only",
                    &a.0,
                    &b.0,
                ]);
                let out = c.output()?;
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                match out.status.code() {
                    Some(0) => Ok(Ok(text.split('\0').next().unwrap_or("").trim().to_string())),
                    Some(1) => {
                        let mut parts = text.split('\0');
                        let _tree = parts.next();
                        let conflicts: Vec<String> = parts
                            .take_while(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        Ok(Err(conflicts))
                    }
                    _ => Err(tool_err("git merge-tree", &out.status, &out.stderr)),
                }
            };
        let mut message = msg.message.trim_end().to_string();
        if !trailers.is_empty() {
            message.push_str("\n\n");
            for (k, v) in trailers {
                message.push_str(&format!("{k}: {v}\n"));
            }
        }
        let commit_tree = |tree: &str,
                           parents: &[&RevId],
                           author: (&str, &str, Option<i64>),
                           message: &str|
         -> Result<RevId> {
            let mut c = git();
            c.arg("-C").arg(dir);
            if let Some(s) = signer {
                c.arg("-c")
                    .arg("gpg.format=ssh")
                    .arg("-c")
                    .arg(format!("user.signingkey={}", s.private_key_path.display()));
            }
            c.arg("commit-tree").arg(tree);
            if signer.is_some() {
                c.arg("-S");
            }
            for p in parents {
                c.arg("-p").arg(&p.0);
            }
            c.env("GIT_AUTHOR_NAME", author.0)
                .env("GIT_AUTHOR_EMAIL", author.1)
                .env("GIT_COMMITTER_NAME", &msg.committer_name)
                .env("GIT_COMMITTER_EMAIL", &msg.committer_email)
                .envs(env.iter().cloned());
            if let Some(ts) = author.2 {
                c.env("GIT_AUTHOR_DATE", format!("{} +0000", ts / 1000));
            }
            c.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = c.spawn()?;
            child
                .stdin
                .take()
                .expect("piped")
                .write_all(message.as_bytes())?;
            let out = child.wait_with_output()?;
            if !out.status.success() {
                return Err(tool_err("git commit-tree", &out.status, &out.stderr));
            }
            Ok(RevId(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ))
        };
        match kind {
            MergeKind::Merge => match merge_tree(base, head)? {
                Ok(tree) => Ok(MergeOutcome::Clean(commit_tree(
                    &tree,
                    &[base, head],
                    (&msg.author_name, &msg.author_email, None),
                    &message,
                )?)),
                Err(c) => Ok(MergeOutcome::Conflicts(c)),
            },
            MergeKind::Squash => match merge_tree(base, head)? {
                Ok(tree) => Ok(MergeOutcome::Clean(commit_tree(
                    &tree,
                    &[base],
                    (&msg.author_name, &msg.author_email, None),
                    &message,
                )?)),
                Err(c) => Ok(MergeOutcome::Conflicts(c)),
            },
            MergeKind::Rebase => {
                // Replay head commits (oldest first) onto the advancing base.
                let mut commits = self.rev_list(dir, Some(base), head, 10_000, &env)?;
                commits.reverse();
                let mut tip = base.clone();
                for c in commits {
                    if c.parents.len() != 1 {
                        return Err(VcsError::Unsupported(
                            "rebase of merge commits is not supported".into(),
                        ));
                    }
                    // Three-way: merge-tree with the commit's parent as merge base.
                    let mut cmd = git();
                    cmd.arg("-C").arg(dir).args([
                        "merge-tree",
                        "--write-tree",
                        "-z",
                        "--name-only",
                        "--merge-base",
                        &c.parents[0].0,
                        &tip.0,
                        &c.id.0,
                    ]);
                    let out = cmd.output()?;
                    let text = String::from_utf8_lossy(&out.stdout).into_owned();
                    let tree = match out.status.code() {
                        Some(0) => text.split('\0').next().unwrap_or("").trim().to_string(),
                        Some(1) => {
                            let conflicts: Vec<String> = text
                                .split('\0')
                                .skip(1)
                                .take_while(|s| !s.is_empty())
                                .map(|s| s.to_string())
                                .collect();
                            return Ok(MergeOutcome::Conflicts(conflicts));
                        }
                        _ => return Err(tool_err("git merge-tree", &out.status, &out.stderr)),
                    };
                    let mut m = c.message.trim_end().to_string();
                    m.push_str(&format!("\n\nVcsstar-Original-Commit: {}\n", c.id));
                    for (k, v) in trailers {
                        m.push_str(&format!("{k}: {v}\n"));
                    }
                    tip = commit_tree(
                        &tree,
                        &[&tip],
                        (&c.author_name, &c.author_email, Some(c.authored_at_ms)),
                        &m,
                    )?;
                }
                Ok(MergeOutcome::Clean(tip))
            }
        }
    }

    fn verify_signature(
        &self,
        dir: &Path,
        id: &RevId,
        allowed_signers: &Path,
        env: &Env,
    ) -> Result<SigStatus> {
        let commit = self.cat_commit(dir, id, env)?;
        if extract_gpgsig(&commit).is_none() {
            return Ok(SigStatus::Missing);
        }
        let mut c = git();
        c.arg("-C")
            .arg(dir)
            .arg("-c")
            .arg("gpg.format=ssh")
            .arg("-c")
            .arg(format!(
                "gpg.ssh.allowedSignersFile={}",
                allowed_signers.display()
            ))
            .args(["verify-commit", "--raw", &id.0])
            .envs(env.iter().cloned());
        let out = c.output()?;
        let raw = String::from_utf8_lossy(&out.stderr).into_owned();
        // ssh-keygen output, relayed by git:
        //   Good "git" signature for <principal> with ED25519 key SHA256:...
        //   Good "git" signature with ED25519 key SHA256:...  +  No principal matched.
        //   Could not verify signature.
        let mut principal = None;
        let mut fingerprint = None;
        for line in raw.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("Good \"git\" signature for ") {
                if let Some((p, tail)) = rest.split_once(" with ") {
                    principal = Some(p.trim().to_string());
                    fingerprint = tail.rsplit(' ').next().map(|s| s.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("Good \"git\" signature with ") {
                fingerprint = rest.rsplit(' ').next().map(|s| s.to_string());
            }
        }
        if out.status.success() {
            if let (Some(p), Some(f)) = (principal, fingerprint.clone()) {
                return Ok(SigStatus::Good {
                    principal: p,
                    fingerprint: f,
                });
            }
        }
        let reason = if raw.contains("No principal matched") {
            format!(
                "signer {} is not a registered signing key",
                fingerprint.unwrap_or_default()
            )
        } else {
            raw.lines()
                .last()
                .unwrap_or("signature did not verify")
                .to_string()
        };
        Ok(SigStatus::Bad { reason })
    }

    fn export_pack(
        &self,
        dir: &Path,
        have: &[RevId],
        want: &[RevId],
        out: &mut dyn Write,
    ) -> Result<()> {
        let mut revs = String::new();
        for w in want {
            check_rev(&w.0)?;
            revs.push_str(&w.0);
            revs.push('\n');
        }
        for h in have {
            check_rev(&h.0)?;
            revs.push('^');
            revs.push_str(&h.0);
            revs.push('\n');
        }
        let pack = run_stdin(
            dir,
            &Env::new(),
            &["pack-objects", "--stdout", "--revs", "-q"],
            revs.as_bytes(),
        )?;
        out.write_all(&pack)?;
        Ok(())
    }

    fn export_pack_excluding(
        &self,
        dir: &Path,
        want: &[RevId],
        covered_idx: &[PathBuf],
        out: &mut dyn Write,
    ) -> Result<usize> {
        if want.is_empty() {
            return Ok(0);
        }
        let mut revs = String::new();
        for w in want {
            check_rev(&w.0)?;
            revs.push_str(&w.0);
            revs.push('\n');
        }
        let reachable = run_stdin(
            dir,
            &Env::new(),
            &["rev-list", "--objects", "--stdin"],
            revs.as_bytes(),
        )?;
        let mut covered: HashSet<String> = HashSet::new();
        for idx in covered_idx {
            let bytes = std::fs::read(idx)?;
            let listing = run_stdin(dir, &Env::new(), &["show-index"], &bytes)?;
            for line in String::from_utf8_lossy(&listing).lines() {
                if let Some(id) = line.split_whitespace().nth(1) {
                    covered.insert(id.to_string());
                }
            }
        }
        let mut missing = String::new();
        let mut count = 0usize;
        for line in String::from_utf8_lossy(&reachable).lines() {
            let Some(id) = line.split_whitespace().next() else {
                continue;
            };
            if !covered.contains(id) {
                missing.push_str(id);
                missing.push('\n');
                count += 1;
            }
        }
        if count == 0 {
            return Ok(0);
        }
        let pack = run_stdin(
            dir,
            &Env::new(),
            &["pack-objects", "--stdout", "-q"],
            missing.as_bytes(),
        )?;
        out.write_all(&pack)?;
        Ok(count)
    }

    fn import_pack(&self, dir: &Path, input: &mut dyn Read) -> Result<Option<String>> {
        let mut c = git();
        c.arg("-C")
            .arg(dir)
            .args(["index-pack", "--stdin", "--fix-thin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn()?;
        {
            let mut stdin = child.stdin.take().expect("piped");
            std::io::copy(input, &mut stdin)?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(tool_err("git index-pack", &out.status, &out.stderr));
        }
        // index-pack prints "pack\t<hash>" (or "keep\t<hash>").
        let stdout = String::from_utf8_lossy(&out.stdout);
        Ok(stdout.lines().find_map(|l| {
            l.split_once('\t')
                .map(|(_, h)| format!("pack-{}", h.trim()))
        }))
    }

    fn spawn_service(
        &self,
        dir: &Path,
        svc: WireService,
        stateless_rpc: bool,
        advertise_refs: bool,
        env: &Env,
    ) -> Result<Child> {
        let mut c = git();
        c.arg("-C").arg(dir);
        match svc {
            WireService::UploadPack => c.arg("upload-pack"),
            WireService::ReceivePack => c.arg("receive-pack"),
        };
        if stateless_rpc {
            c.arg("--stateless-rpc");
        }
        if advertise_refs {
            c.arg("--advertise-refs");
        }
        c.arg(".")
            .envs(env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(c.spawn()?)
    }
}

/// Pull the `gpgsig` header out of a raw commit object.
pub fn extract_gpgsig(commit: &str) -> Option<String> {
    let (headers, _) = commit.split_once("\n\n")?;
    let mut sig = String::new();
    let mut in_sig = false;
    for line in headers.lines() {
        if let Some(rest) = line.strip_prefix("gpgsig ") {
            in_sig = true;
            sig.push_str(rest);
            sig.push('\n');
        } else if in_sig && line.starts_with(' ') {
            sig.push_str(&line[1..]);
            sig.push('\n');
        } else if in_sig {
            break;
        }
    }
    if sig.is_empty() {
        None
    } else {
        Some(sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_git() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    /// Build a tiny repository with a working tree and push its history
    /// into a bare backend repo.
    fn seed(bare: &Path) -> (tempfile::TempDir, RevId, RevId) {
        let work = tempfile::tempdir().unwrap();
        let w = work.path();
        let g = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(w)
                .args(args)
                .env("GIT_AUTHOR_NAME", "A")
                .env("GIT_AUTHOR_EMAIL", "a@example.com")
                .env("GIT_COMMITTER_NAME", "C")
                .env("GIT_COMMITTER_EMAIL", "c@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(w.join("a.txt"), "hello\n").unwrap();
        std::fs::create_dir(w.join("d")).unwrap();
        std::fs::write(w.join("d/b.txt"), "b\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "first"]);
        let c1 = g(&["rev-parse", "HEAD"]);
        std::fs::write(w.join("a.txt"), "hello\nworld\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "second\n\nbody"]);
        let c2 = g(&["rev-parse", "HEAD"]);
        g(&["push", "-q", bare.to_str().unwrap(), "main:refs/heads/main"]);
        (work, RevId(c1), RevId(c2))
    }

    #[test]
    fn reads_and_refs() {
        if !have_git() {
            eprintln!("git not installed; skipping");
            return;
        }
        let be = GitBackend::default();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("r.git");
        be.init(&dir).unwrap();
        let (_w, c1, c2) = seed(&dir);
        let refs = be.list_refs(&dir).unwrap();
        assert_eq!(refs, vec![("refs/heads/main".to_string(), c2.clone())]);

        let rev = be.revision(&dir, &c2, &Env::new()).unwrap();
        assert_eq!(rev.parents, vec![c1.clone()]);
        assert_eq!(rev.author_name, "A");
        assert_eq!(rev.message, "second\n\nbody");
        assert!(rev.signature.is_none());

        let log = be.log(&dir, &c2, None, 10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].id, c2);
        let log_path = be.log(&dir, &c2, Some("d/b.txt"), 10).unwrap();
        assert_eq!(log_path.len(), 1);

        assert!(be.is_ancestor(&dir, &c1, &c2, &Env::new()).unwrap());
        assert!(!be.is_ancestor(&dir, &c2, &c1, &Env::new()).unwrap());
        assert_eq!(
            be.common_ancestor(&dir, &c1, &c2).unwrap(),
            Some(c1.clone())
        );

        let tree = be.tree_at(&dir, &c2, "").unwrap();
        let names: Vec<_> = tree.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "d"]);
        assert_eq!(tree[1].kind, "tree");
        let (oid, blob) = be.blob(&dir, &c2, "a.txt").unwrap();
        assert_eq!(blob, b"hello\nworld\n");
        assert_eq!(oid.len(), 40);

        let stat = be.diff_stat(&dir, &c1, &c2, None).unwrap();
        assert_eq!(stat.len(), 1);
        assert_eq!(stat[0].additions, 1);
        assert_eq!(stat[0].status, "modified");
        let patch = be.diff_patch(&dir, &c1, &c2, "a.txt").unwrap();
        assert!(patch.contains("+world"));

        // write_refs makes the dir match exactly
        be.write_refs(&dir, &[("refs/heads/dev".to_string(), c1.clone())], "dev")
            .unwrap();
        let refs = be.list_refs(&dir).unwrap();
        assert_eq!(refs, vec![("refs/heads/dev".to_string(), c1.clone())]);
        assert_eq!(be.resolve(&dir, "HEAD", &Env::new()).unwrap(), c1);
        assert!(matches!(
            be.verify_signature(&dir, &c1, Path::new("/dev/null"), &Env::new())
                .unwrap(),
            SigStatus::Missing
        ));
    }

    #[test]
    fn merges() {
        if !have_git() {
            return;
        }
        let be = GitBackend::default();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("r.git");
        be.init(&dir).unwrap();
        let (work, c1, c2) = seed(&dir);
        // Branch from c1 touching another file.
        let w = work.path();
        let g = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(w)
                .args(args)
                .env("GIT_AUTHOR_NAME", "A")
                .env("GIT_AUTHOR_EMAIL", "a@example.com")
                .env("GIT_COMMITTER_NAME", "C")
                .env("GIT_COMMITTER_EMAIL", "c@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        g(&["checkout", "-q", "-b", "feat", &c1.0]);
        std::fs::write(w.join("d/b.txt"), "b\nmore\n").unwrap();
        g(&["commit", "-q", "-am", "feat"]);
        let f = RevId(g(&["rev-parse", "HEAD"]));
        g(&["push", "-q", dir.to_str().unwrap(), "feat:refs/heads/feat"]);
        let msg = CommitMsg {
            message: "Merge feat".into(),
            author_name: "bot".into(),
            author_email: "bot@example.com".into(),
            committer_name: "bot".into(),
            committer_email: "bot@example.com".into(),
        };
        let trailers = vec![("Vcsstar-Acting-Identity".to_string(), "u@d".to_string())];
        match be
            .merge(&dir, MergeKind::Merge, &c2, &f, &msg, None, &trailers)
            .unwrap()
        {
            MergeOutcome::Clean(m) => {
                let r = be.revision(&dir, &m, &Env::new()).unwrap();
                assert_eq!(r.parents, vec![c2.clone(), f.clone()]);
                assert!(r.message.contains("Vcsstar-Acting-Identity: u@d"));
            }
            other => panic!("{other:?}"),
        }
        match be
            .merge(&dir, MergeKind::Squash, &c2, &f, &msg, None, &[])
            .unwrap()
        {
            MergeOutcome::Clean(m) => {
                let r = be.revision(&dir, &m, &Env::new()).unwrap();
                assert_eq!(r.parents, vec![c2.clone()]);
            }
            other => panic!("{other:?}"),
        }
        match be
            .merge(&dir, MergeKind::Rebase, &c2, &f, &msg, None, &[])
            .unwrap()
        {
            MergeOutcome::Clean(m) => {
                let r = be.revision(&dir, &m, &Env::new()).unwrap();
                assert_eq!(r.parents, vec![c2.clone()]);
                assert_eq!(r.author_name, "A");
                assert!(r.message.contains(&format!("Vcsstar-Original-Commit: {f}")));
            }
            other => panic!("{other:?}"),
        }
        // Conflict: both sides change a.txt differently.
        g(&["checkout", "-q", "-b", "conf", &c1.0]);
        std::fs::write(w.join("a.txt"), "different\n").unwrap();
        g(&["commit", "-q", "-am", "conf"]);
        let cf = RevId(g(&["rev-parse", "HEAD"]));
        g(&["push", "-q", dir.to_str().unwrap(), "conf:refs/heads/conf"]);
        match be
            .merge(&dir, MergeKind::Merge, &c2, &cf, &msg, None, &[])
            .unwrap()
        {
            MergeOutcome::Conflicts(c) => assert_eq!(c, vec!["a.txt".to_string()]),
            other => panic!("{other:?}"),
        }
        // pack export/import round trip into a fresh repo
        let dir2 = tmp.path().join("r2.git");
        be.init(&dir2).unwrap();
        let mut pack = Vec::new();
        be.export_pack(&dir, &[], std::slice::from_ref(&c2), &mut pack)
            .unwrap();
        be.import_pack(&dir2, &mut pack.as_slice()).unwrap();
        assert!(be.object_exists(&dir2, &c2, &Env::new()));
    }
}

#[cfg(test)]
mod sig_tests {
    use super::*;

    #[test]
    fn ssh_signed_commit_verifies_against_allowed_signers() {
        if Command::new("ssh-keygen").arg("-?").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let key = tmp.path().join("k");
        let st = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "u@d", "-f"])
            .arg(&key)
            .status()
            .unwrap();
        assert!(st.success());
        let pubkey = std::fs::read_to_string(tmp.path().join("k.pub")).unwrap();
        let work = tmp.path().join("w");
        std::fs::create_dir(&work).unwrap();
        let g = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&work)
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
        };
        g(&["init", "-q", "-b", "main"]);
        g(&[
            "-c",
            "gpg.format=ssh",
            "-c",
            &format!("user.signingkey={}", key.display()),
            "commit",
            "-q",
            "-S",
            "--allow-empty",
            "-m",
            "signed",
        ]);
        let id = RevId(g(&["rev-parse", "HEAD"]));
        let be = GitBackend::default();
        let rev = be.revision(&work, &id, &Env::new()).unwrap();
        assert!(rev.signature.is_some());
        let mut parts = pubkey.split_whitespace();
        let (kt, kb) = (parts.next().unwrap(), parts.next().unwrap());
        let allowed = tmp.path().join("allowed");
        std::fs::write(&allowed, format!("u@d namespaces=\"git\" {kt} {kb}\n")).unwrap();
        match be
            .verify_signature(&work, &id, &allowed, &Env::new())
            .unwrap()
        {
            SigStatus::Good {
                principal,
                fingerprint,
            } => {
                assert_eq!(principal, "u@d");
                assert!(fingerprint.starts_with("SHA256:"));
            }
            other => panic!("{other:?}"),
        }
        std::fs::write(&allowed, "x@y namespaces=\"git\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n").unwrap();
        assert!(matches!(
            be.verify_signature(&work, &id, &allowed, &Env::new())
                .unwrap(),
            SigStatus::Bad { .. }
        ));
    }
}
