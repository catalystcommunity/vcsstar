//! Curated commands: sugar over the same generated client. Nothing here
//! reaches the server by any other path than `call`.

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{json, Value as Json};

use crate::descriptor::Schema;
use crate::keys;
use crate::{call, connect, print_json, profiles, CliError, Conn};

pub fn commands() -> Vec<Command> {
    vec![
        Command::new("keys")
            .about("Developer SSH keys as LinkKeys application keys (enroll, renew, status, revoke, git-config)")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("enroll")
                    .about("Make auth, signing, and quorum keys; enroll this machine at your home domain; register with vcsstar")
                    .arg(Arg::new("api-key-file").long("api-key-file").required(true).help("File with a LinkKeys account API key for your home domain (used once, never stored)"))
                    .arg(Arg::new("instance-id").long("instance-id").help("Application instance id (default: <hostname>-<random>)"))
                    .arg(Arg::new("key-dir").long("key-dir").help("Directory for the key files (default: ~/.ssh)")),
            )
            .subcommand(
                Command::new("renew")
                    .about("Renew attestations below half-life and refresh them at vcsstar")
                    .arg(Arg::new("force").long("force").action(ArgAction::SetTrue).help("Renew every key now")),
            )
            .subcommand(Command::new("status").about("Show local keys, attestation windows, and vcsstar-side status"))
            .subcommand(
                Command::new("revoke")
                    .about("Revoke one key at the home domain (two sibling signatures) and at vcsstar")
                    .arg(Arg::new("role").required(true).help("auth | signing | quorum, or a key id"))
                    .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue).help("Do not ask")),
            )
            .subcommand(
                Command::new("git-config")
                    .about("Write gpg.format=ssh, user.signingkey, commit.gpgsign, tag.gpgsign")
                    .arg(Arg::new("global").long("global").action(ArgAction::SetTrue).help("Write to the global git config"))
                    .arg(Arg::new("allowed-signers").long("allowed-signers").help("Also set gpg.ssh.allowedSignersFile to this path")),
            ),
        Command::new("git-credential")
            .about("git credential helper: `git config credential.helper 'vcsstar git-credential'`")
            .arg(Arg::new("op").required(true).help("get | store | erase")),
        Command::new("pr")
            .about("Pull request shortcuts")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("open")
                    .about("Open a pull request: vcsstar pr open <org>/<name> --head <branch> [--base main] --title T")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("head").long("head").required(true).help("Head branch (same repo) or org/name:branch for a fork"))
                    .arg(Arg::new("base").long("base").default_value("main"))
                    .arg(Arg::new("title").long("title").required(true))
                    .arg(Arg::new("body").long("body")),
            )
            .subcommand(
                Command::new("list")
                    .about("List pull requests")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("state").long("state").default_value("open").help("open | closed | merged | all")),
            )
            .subcommand(
                Command::new("get")
                    .about("Show one pull request")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("number").required(true)),
            )
            .subcommand(
                Command::new("review")
                    .about("Review the current head: --approve | --request-changes | --comment")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("number").required(true))
                    .arg(Arg::new("approve").long("approve").action(ArgAction::SetTrue))
                    .arg(Arg::new("request-changes").long("request-changes").action(ArgAction::SetTrue))
                    .arg(Arg::new("body").long("body")),
            )
            .subcommand(
                Command::new("merge")
                    .about("Merge with a stale-view guard on the head you saw")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("number").required(true))
                    .arg(Arg::new("kind").long("kind").default_value("merge").help("merge | squash | rebase"))
                    .arg(Arg::new("expect-head").long("expect-head").help("Head sha you reviewed (default: the current head)"))
                    .arg(Arg::new("message").long("message")),
            )
            .subcommand(
                Command::new("close")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("number").required(true)),
            ),
        Command::new("repo")
            .about("Repository shortcuts")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("clone")
                    .about("Clone <org>/<name> with the profile's credentials")
                    .arg(Arg::new("repo").required(true).help("org/name"))
                    .arg(Arg::new("dir").help("Target directory"))
                    .arg(Arg::new("ssh").long("ssh").action(ArgAction::SetTrue).help("Clone over SSH instead of HTTPS")),
            )
            .subcommand(
                Command::new("create")
                    .about("Create a repository: vcsstar repo create <org>/<name> [--visibility v]")
                    .arg(Arg::new("repo").required(true))
                    .arg(Arg::new("visibility").long("visibility").default_value("org"))
                    .arg(Arg::new("description").long("description")),
            ),
    ]
}

pub fn run(schema: &Schema, m: &ArgMatches, name: &str, sm: &ArgMatches) -> Result<(), CliError> {
    match name {
        "keys" => run_keys(schema, m, sm),
        "git-credential" => git_credential(m, sm),
        "repo" => run_repo(schema, m, sm),
        "pr" => run_pr(schema, m, sm),
        _ => Err(CliError::Usage(format!("unknown command {name}"))),
    }
}

fn profile_name(m: &ArgMatches) -> Result<String, CliError> {
    let store = profiles::load()?;
    Ok(store
        .select(m.get_one::<String>("profile").map(String::as_str))
        .map(|(n, _)| n)
        .unwrap_or_else(|_| "default".into()))
}

fn whoami(schema: &Schema, conn: &Conn) -> Result<(String, String), CliError> {
    let who = call(
        schema,
        conn,
        "Auth",
        "whoami",
        Json::Object(Default::default()),
    )?;
    let i = who
        .get("identity")
        .ok_or_else(|| CliError::Transport("whoami returned no identity".into()))?;
    let user_id = i
        .get("user_id")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let domain = i
        .get("domain")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    Ok((user_id, domain))
}

fn run_keys(schema: &Schema, m: &ArgMatches, km: &ArgMatches) -> Result<(), CliError> {
    let profile = profile_name(m)?;
    match km.subcommand() {
        Some(("enroll", em)) => {
            let conn = connect(m, true)?;
            let (user_id, domain) = whoami(schema, &conn)?;
            let identity = format!("{user_id}@{domain}");
            let mut store = keys::load()?;
            if store.enrollments.contains_key(&profile) {
                return Err(CliError::Usage(format!(
                    "profile {profile} already has an enrollment; run `vcsstar keys status` or revoke first"
                )));
            }
            let api_key = std::fs::read_to_string(em.get_one::<String>("api-key-file").unwrap())
                .context("read api key file")?
                .trim()
                .to_string();
            let key_dir = em
                .get_one::<String>("key-dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".ssh")
                });
            eprintln!(
                "enrolling {identity} at its home domain as application {}...",
                keys::APPLICATION_ID
            );
            let mut e = keys::enroll(keys::EnrollOptions {
                profile: &profile,
                identity: &identity,
                api_key: &api_key,
                instance_id: em.get_one::<String>("instance-id").map(String::as_str),
                key_dir,
            })?;
            // Register auth and signing keys with vcsstar with their LinkKeys coordinates.
            for k in e.keys.iter_mut().filter(|k| k.role != "quorum") {
                let resp = call(
                    schema,
                    &conn,
                    "Auth",
                    "register-key",
                    json!({
                        "public_key": k.openssh_public,
                        "usage": k.role,
                        "application_id": e.application_id,
                        "instance_id": e.instance_id,
                        "lk_key_id": k.key_id,
                    }),
                )?;
                k.vcsstar_key_id = resp
                    .get("key_id")
                    .and_then(Json::as_str)
                    .map(str::to_string);
            }
            store.enrollments.insert(profile.clone(), e.clone());
            keys::save(&store)?;
            print_json(m, &keys::status_json(&e))?;
            eprintln!(
                "next: `vcsstar keys git-config --global` to sign commits with the signing key;"
            );
            eprintln!(
                "      add the auth key to your ssh config or agent: {}",
                e.keys[0].path.display()
            );
            Ok(())
        }
        Some(("renew", rm)) => {
            let mut store = keys::load()?;
            let e = store.enrollments.get_mut(&profile).ok_or_else(|| {
                CliError::Usage(format!(
                    "no enrollment for profile {profile}; run `vcsstar keys enroll`"
                ))
            })?;
            let changed = keys::renew(e, rm.get_flag("force"))?;
            if !changed.is_empty() {
                let conn = connect(m, true)?;
                for k in e
                    .keys
                    .iter_mut()
                    .filter(|k| k.role != "quorum" && changed.contains(&k.key_id))
                {
                    let resp = call(
                        schema,
                        &conn,
                        "Auth",
                        "register-key",
                        json!({
                            "public_key": k.openssh_public,
                            "usage": k.role,
                            "application_id": e.application_id,
                            "instance_id": e.instance_id,
                            "lk_key_id": k.key_id,
                        }),
                    )?;
                    k.vcsstar_key_id = resp
                        .get("key_id")
                        .and_then(Json::as_str)
                        .map(str::to_string);
                }
            }
            let out = keys::status_json(e);
            keys::save(&store)?;
            eprintln!("renewed {} key(s)", changed.len());
            print_json(m, &out)?;
            Ok(())
        }
        Some(("status", _)) => {
            let store = keys::load()?;
            let local = store.enrollments.get(&profile).map(keys::status_json);
            let remote = connect(m, true)
                .and_then(|conn| {
                    call(
                        schema,
                        &conn,
                        "Auth",
                        "list-keys",
                        Json::Object(Default::default()),
                    )
                })
                .ok();
            print_json(m, &json!({"local": local, "vcsstar": remote}))?;
            Ok(())
        }
        Some(("revoke", rm)) => {
            let mut store = keys::load()?;
            let e = store
                .enrollments
                .get_mut(&profile)
                .ok_or_else(|| CliError::Usage(format!("no enrollment for profile {profile}")))?;
            let sel = rm.get_one::<String>("role").unwrap();
            let idx = e
                .keys
                .iter()
                .position(|k| &k.role == sel || &k.key_id == sel)
                .ok_or_else(|| CliError::Usage(format!("no key {sel} in this enrollment")))?;
            if !rm.get_flag("yes") {
                if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    return Err(CliError::Usage(
                        "pass --yes to revoke without a prompt".into(),
                    ));
                }
                eprint!(
                    "revoke key {} ({})? [y/N] ",
                    e.keys[idx].key_id, e.keys[idx].role
                );
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).ok();
                if !line.trim().eq_ignore_ascii_case("y") {
                    return Err(CliError::Usage("cancelled".into()));
                }
            }
            let target = e.keys[idx].clone();
            let revoked_at = keys::revoke(e, &target)?;
            if let Some(id) = &target.vcsstar_key_id {
                let conn = connect(m, true)?;
                let _ = call(schema, &conn, "Auth", "revoke-key", json!({"key_id": id}));
            }
            e.keys.remove(idx);
            keys::save(&store)?;
            eprintln!(
                "revoked {} at {revoked_at}; the key file {} is no longer trusted",
                target.key_id,
                target.path.display()
            );
            Ok(())
        }
        Some(("git-config", gm)) => {
            let store = keys::load()?;
            let e = store
                .enrollments
                .get(&profile)
                .ok_or_else(|| CliError::Usage(format!("no enrollment for profile {profile}")))?;
            let applied = keys::git_config(
                e,
                gm.get_flag("global"),
                gm.get_one::<String>("allowed-signers")
                    .map(std::path::Path::new),
            )?;
            for a in applied {
                eprintln!("set {a}");
            }
            Ok(())
        }
        _ => Err(CliError::Usage("keys subcommand required".into())),
    }
}

/// git credential protocol: key=value lines on stdin, answer on stdout.
fn git_credential(m: &ArgMatches, gm: &ArgMatches) -> Result<(), CliError> {
    let op = gm.get_one::<String>("op").unwrap();
    if op != "get" {
        // store/erase: the profile store owns the token; nothing to do.
        return Ok(());
    }
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).context("read stdin")?;
    let mut protocol = None;
    let mut host = None;
    for line in input.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "protocol" => protocol = Some(v.to_string()),
                "host" => host = Some(v.to_string()),
                _ => {}
            }
        }
    }
    let (Some(protocol), Some(host)) = (protocol, host) else {
        return Ok(());
    };
    let store = profiles::load()?;
    let wanted = format!("{protocol}://{host}");
    let selected = m.get_one::<String>("profile").map(String::as_str);
    let profile = store
        .profiles
        .iter()
        .filter(|(n, _)| selected.is_none_or(|s| s == n.as_str()))
        .find(|(_, p)| p.url.trim_end_matches('/').eq_ignore_ascii_case(&wanted))
        .map(|(_, p)| p);
    if let Some(p) = profile {
        if let Some(token) = &p.token {
            println!("username=vcsstar");
            println!("password={token}");
        }
    }
    Ok(())
}

fn run_repo(schema: &Schema, m: &ArgMatches, rm: &ArgMatches) -> Result<(), CliError> {
    match rm.subcommand() {
        Some(("clone", cm)) => {
            let spec = cm.get_one::<String>("repo").unwrap();
            let (org, name) = spec
                .split_once('/')
                .ok_or_else(|| CliError::Usage("repo must be org/name".into()))?;
            let conn = connect(m, false)?;
            let repo = call(
                schema,
                &conn,
                "Repos",
                "get",
                json!({"repo": {"org": org, "name": name}}),
            )?;
            let url = if cm.get_flag("ssh") {
                repo.get("ssh_url")
                    .and_then(Json::as_str)
                    .ok_or_else(|| CliError::Usage("the instance advertises no SSH URL".into()))?
                    .to_string()
            } else {
                repo.get("https_url")
                    .and_then(Json::as_str)
                    .ok_or_else(|| CliError::Transport("no https_url in the response".into()))?
                    .to_string()
            };
            let mut c = std::process::Command::new("git");
            c.arg("clone");
            if !cm.get_flag("ssh") {
                // The credential helper reads the profile token; no token in argv.
                let exe = std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "vcsstar".into());
                c.arg("-c")
                    .arg(format!("credential.helper={exe} git-credential"));
            }
            c.arg(&url);
            if let Some(d) = cm.get_one::<String>("dir") {
                c.arg(d);
            }
            let st = c.status().context("run git clone")?;
            if !st.success() {
                return Err(CliError::Other(anyhow!("git clone failed")));
            }
            Ok(())
        }
        Some(("create", cm)) => {
            let spec = cm.get_one::<String>("repo").unwrap();
            let (org, name) = spec
                .split_once('/')
                .ok_or_else(|| CliError::Usage("repo must be org/name".into()))?;
            let conn = connect(m, true)?;
            let mut req = json!({"org": org, "name": name, "visibility": cm.get_one::<String>("visibility").unwrap()});
            if let Some(d) = cm.get_one::<String>("description") {
                req["description"] = json!(d);
            }
            let repo = call(schema, &conn, "Repos", "create", req)?;
            print_json(m, &repo)?;
            Ok(())
        }
        _ => Err(CliError::Usage("repo subcommand required".into())),
    }
}

fn split_repo(spec: &str) -> Result<(String, String), CliError> {
    spec.split_once('/')
        .map(|(o, n)| (o.to_string(), n.to_string()))
        .ok_or_else(|| CliError::Usage("repo must be org/name".into()))
}

fn number_arg(m: &ArgMatches) -> Result<u64, CliError> {
    m.get_one::<String>("number")
        .unwrap()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| CliError::Usage("number must be an integer".into()))
}

fn run_pr(schema: &Schema, m: &ArgMatches, pm: &ArgMatches) -> Result<(), CliError> {
    let conn = connect(m, true)?;
    match pm.subcommand() {
        Some(("open", om)) => {
            let (org, name) = split_repo(om.get_one::<String>("repo").unwrap())?;
            let head = om.get_one::<String>("head").unwrap();
            let (head_repo, head_ref) = match head.split_once(':') {
                Some((r, b)) => {
                    let (ho, hn) = split_repo(r)?;
                    (json!({"org": ho, "name": hn}), b.to_string())
                }
                None => (json!({"org": org, "name": name}), head.clone()),
            };
            let mut req = json!({
                "repo": {"org": org, "name": name},
                "title": om.get_one::<String>("title").unwrap(),
                "base_ref": om.get_one::<String>("base").unwrap(),
                "head": {"repo": head_repo, "ref": head_ref},
            });
            if let Some(b) = om.get_one::<String>("body") {
                req["body"] = json!(b);
            }
            let pr = call(schema, &conn, "PullRequests", "open", req)?;
            print_json(m, &pr)?;
            Ok(())
        }
        Some(("list", lm)) => {
            let (org, name) = split_repo(lm.get_one::<String>("repo").unwrap())?;
            let page = call(
                schema,
                &conn,
                "PullRequests",
                "list",
                json!({"repo": {"org": org, "name": name}, "state": lm.get_one::<String>("state").unwrap()}),
            )?;
            print_json(m, page.get("pull_requests").unwrap_or(&Json::Null))?;
            Ok(())
        }
        Some(("get", gm)) => {
            let (org, name) = split_repo(gm.get_one::<String>("repo").unwrap())?;
            let pr = call(
                schema,
                &conn,
                "PullRequests",
                "get",
                json!({"repo": {"org": org, "name": name}, "number": number_arg(gm)?}),
            )?;
            print_json(m, &pr)?;
            Ok(())
        }
        Some(("review", rm)) => {
            let (org, name) = split_repo(rm.get_one::<String>("repo").unwrap())?;
            let number = number_arg(rm)?;
            let pr = call(
                schema,
                &conn,
                "PullRequests",
                "get",
                json!({"repo": {"org": org, "name": name}, "number": number}),
            )?;
            let state = if rm.get_flag("approve") {
                "approved"
            } else if rm.get_flag("request-changes") {
                "changes_requested"
            } else {
                "comment"
            };
            let mut req = json!({
                "repo": {"org": org, "name": name}, "number": number, "state": state,
                "head_sha": pr.get("head_sha").and_then(Json::as_str).unwrap_or(""),
            });
            if let Some(b) = rm.get_one::<String>("body") {
                req["body"] = json!(b);
            }
            let review = call(schema, &conn, "PullRequests", "review", req)?;
            print_json(m, &review)?;
            Ok(())
        }
        Some(("merge", mm)) => {
            let (org, name) = split_repo(mm.get_one::<String>("repo").unwrap())?;
            let number = number_arg(mm)?;
            let expected = match mm.get_one::<String>("expect-head") {
                Some(e) => e.clone(),
                None => {
                    let pr = call(
                        schema,
                        &conn,
                        "PullRequests",
                        "get",
                        json!({"repo": {"org": org, "name": name}, "number": number}),
                    )?;
                    pr.get("head_sha")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_string()
                }
            };
            let mut req = json!({
                "repo": {"org": org, "name": name}, "number": number,
                "expected_head_sha": expected,
                "merge_kind": mm.get_one::<String>("kind").unwrap(),
            });
            if let Some(msg) = mm.get_one::<String>("message") {
                req["message"] = json!(msg);
            }
            let result = call(schema, &conn, "PullRequests", "merge", req)?;
            print_json(m, &result)?;
            Ok(())
        }
        Some(("close", cm)) => {
            let (org, name) = split_repo(cm.get_one::<String>("repo").unwrap())?;
            let pr = call(
                schema,
                &conn,
                "PullRequests",
                "close",
                json!({"repo": {"org": org, "name": name}, "number": number_arg(cm)?}),
            )?;
            print_json(m, &pr)?;
            Ok(())
        }
        _ => Err(CliError::Usage("pr subcommand required".into())),
    }
}
