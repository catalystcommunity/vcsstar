//! In-process SSH server for git (`git-ssh` role). Public-key auth only, the
//! user name is always `git`, the only commands are `git-upload-pack` and
//! `git-receive-pack`. Keys resolve against `ssh_keys`; attested keys need a
//! current attestation (plan/01 time rules).

use std::collections::HashMap;
use std::sync::Arc;

use russh::keys::HashAlg;
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vcsstar_core::authz::{self, RepoRole};
use vcsstar_core::principal::{CredentialKind, Principal, ReqCtx};
use vcsstar_core::push::{self, PushCtx, PushGuard};
use vcsstar_core::services::keys;
use vcsstar_core::{vcs, App};
use vcsstar_transport::{Carrier, CredentialSource, Peer};
use vcsstar_vcs::{Env, WireService};

use crate::shutdown;

pub fn serve(app: App, state: shutdown::State) -> anyhow::Result<()> {
    let host_key = load_or_make_host_key(&app)?;
    let config = russh::server::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        keys: vec![host_key],
        methods: russh::MethodSet::from(&[russh::MethodKind::PublicKey][..]),
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&app.config.listen_ssh).await?;
        tracing::info!(addr = %app.config.listen_ssh, "git ssh listening");
        let mut server = SshServer { app };
        let running = server.run_on_socket(Arc::new(config), &listener);
        let handle = running.handle();
        tokio::spawn(async move {
            loop {
                if state.stopping() {
                    handle.shutdown("vcsstar is restarting".into());
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });
        running.await?;
        Ok::<(), anyhow::Error>(())
    })
}

fn load_or_make_host_key(app: &App) -> anyhow::Result<russh::keys::PrivateKey> {
    let path = &app.config.ssh_host_key_path;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        tracing::warn!(path = %path.display(), "generating an SSH host key; mount a persistent one in production");
        let st = std::process::Command::new("ssh-keygen")
            .args([
                "-q",
                "-t",
                "ed25519",
                "-N",
                "",
                "-C",
                "vcsstar host key",
                "-f",
            ])
            .arg(path)
            .status()?;
        if !st.success() {
            anyhow::bail!("ssh-keygen failed to make the host key");
        }
    }
    Ok(russh::keys::load_secret_key(path, None)?)
}

struct SshServer {
    app: App,
}

impl russh::server::Server for SshServer {
    type Handler = SshSession;
    fn new_client(&mut self, peer_addr: Option<std::net::SocketAddr>) -> SshSession {
        SshSession {
            app: self.app.clone(),
            remote: peer_addr.map(|a| a.ip().to_string()).unwrap_or_default(),
            principal: None,
            channels: HashMap::new(),
        }
    }
    fn handle_session_error(&mut self, error: <Self::Handler as russh::server::Handler>::Error) {
        tracing::debug!(error = %error, "ssh session ended with error");
    }
}

struct Running {
    stdin: Option<tokio::process::ChildStdin>,
}

struct SshSession {
    app: App,
    remote: String,
    principal: Option<Principal>,
    channels: HashMap<ChannelId, Running>,
}

fn parse_command(cmd: &str) -> Option<(WireService, String)> {
    let cmd = cmd.trim();
    let (svc, rest) = [
        ("git-upload-pack ", WireService::UploadPack),
        ("git-receive-pack ", WireService::ReceivePack),
        ("git upload-pack ", WireService::UploadPack),
        ("git receive-pack ", WireService::ReceivePack),
    ]
    .iter()
    .find_map(|(prefix, svc)| cmd.strip_prefix(prefix).map(|r| (*svc, r)))?;
    let path = rest.trim().trim_matches('\'').trim_matches('"');
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    Some((svc, path.to_string()))
}

impl russh::server::Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: russh::server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if user != "git" {
            return Ok(Auth::reject());
        }
        let fingerprint = key.public_key.fingerprint(HashAlg::Sha256).to_string();
        let app = self.app.clone();
        let found = tokio::task::spawn_blocking(move || {
            let mut conn = app.conn()?;
            match keys::identity_for_auth_key(&mut conn, &fingerprint)? {
                Some((identity, key_row)) => {
                    let role = vcsstar_core::principal::load_instance_role(&mut conn, identity.id)?;
                    Ok::<_, vcsstar_core::Error>(Some((identity, key_row, role)))
                }
                None => Ok(None),
            }
        })
        .await??;
        match found {
            Some((identity, key_row, instance_role)) => {
                self.principal = Some(Principal {
                    instance_role,
                    identity,
                    scopes: vec!["repo:write".into(), "repo:read".into()],
                    kind: CredentialKind::ApiToken,
                    acting_via: Some(format!("ssh-key:{}", key_row.fingerprint)),
                    credential_id: Some(key_row.id),
                    expires_at: key_row.attested_until,
                });
                Ok(Auth::Accept)
            }
            None => Ok(Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), Running { stdin: None });
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data).into_owned();
        let handle = session.handle();
        let Some(principal) = self.principal.clone() else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        let Some((svc, path)) = parse_command(&cmd) else {
            let _ = handle
                .extended_data(
                    channel,
                    1,
                    bytes(
                        b"vcsstar: only git-upload-pack and git-receive-pack are served\r\n"
                            .to_vec(),
                    ),
                )
                .await;
            let _ = handle.exit_status_request(channel, 1).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
            return Ok(());
        };
        let Some((org, name)) = path.split_once('/') else {
            let _ = handle
                .extended_data(
                    channel,
                    1,
                    bytes(b"vcsstar: repository path must be org/name\r\n".to_vec()),
                )
                .await;
            let _ = handle.exit_status_request(channel, 1).await;
            let _ = handle.close(channel).await;
            return Ok(());
        };
        let app = self.app.clone();
        let (org, name) = (org.to_string(), name.to_string());
        let remote = self.remote.clone();
        // Authorize and hydrate on a blocking thread.
        let prepared =
            tokio::task::spawn_blocking(move || -> Result<(PushCtxParts, Env), String> {
                let ctx = ReqCtx {
                    request_id: uuid::Uuid::now_v7().to_string(),
                    principal: Some(principal.clone()),
                    peer: Peer {
                        credential: None,
                        credential_source: CredentialSource::None,
                        remote,
                        carrier: Carrier::Tcp,
                    },
                };
                let mut conn = app.conn().map_err(|e| e.message)?;
                let (org_row, repo, role) =
                    authz::load_repo_visible(&app, &mut conn, &ctx, &org, &name)
                        .map_err(|e| e.message)?;
                let needed = match svc {
                    WireService::UploadPack => RepoRole::Read,
                    WireService::ReceivePack => RepoRole::Write,
                };
                if role < needed {
                    return Err("write access required".into());
                }
                if svc == WireService::ReceivePack && repo.archived {
                    return Err("repository is archived".into());
                }
                let dir = vcs::hydrate(&app, &mut conn, &repo).map_err(|e| e.message)?;
                Ok((
                    PushCtxParts {
                        org: org_row,
                        repo,
                        dir,
                        principal,
                        role,
                        request_id: ctx.request_id,
                    },
                    Env::new(),
                ))
            })
            .await?;
        let (parts, mut env) = match prepared {
            Ok(v) => v,
            Err(msg) => {
                let _ = handle
                    .extended_data(
                        channel,
                        1,
                        bytes(format!("vcsstar: {msg}\r\n").into_bytes()),
                    )
                    .await;
                let _ = handle.exit_status_request(channel, 1).await;
                let _ = handle.eof(channel).await;
                let _ = handle.close(channel).await;
                return Ok(());
            }
        };
        let mut guard: Option<PushGuard> = None;
        if svc == WireService::ReceivePack {
            let push_id = uuid::Uuid::now_v7();
            guard = Some(self.app.pushes.begin(PushCtx {
                id: push_id,
                org: parts.org.clone(),
                repo: parts.repo.clone(),
                dir: parts.dir.clone(),
                principal: parts.principal.clone(),
                role: parts.role,
                request_id: parts.request_id.clone(),
            }));
            env.extend(push::receive_env(&self.app, push_id));
        }
        let mut child = self
            .app
            .store
            .backend()
            .spawn_service(&parts.dir, svc, false, false, &env)?;
        let stdin = child
            .stdin
            .take()
            .map(tokio::process::ChildStdin::from_std)
            .transpose()?;
        let mut stdout =
            tokio::process::ChildStdout::from_std(child.stdout.take().expect("piped"))?;
        let mut stderr =
            tokio::process::ChildStderr::from_std(child.stderr.take().expect("piped"))?;
        if let Some(r) = self.channels.get_mut(&channel) {
            r.stdin = stdin;
        }
        session.channel_success(channel)?;
        let h_out = handle.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if h_out.data(channel, bytes(buf[..n].to_vec())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let h_err = handle.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8 * 1024];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if h_err
                            .extended_data(channel, 1, bytes(buf[..n].to_vec()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        tokio::spawn(async move {
            let status = tokio::task::spawn_blocking(move || child.wait()).await;
            let code = status
                .ok()
                .and_then(|s| s.ok())
                .and_then(|s| s.code())
                .unwrap_or(1) as u32;
            // Give the stdout reader a moment to drain.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = handle.exit_status_request(channel, code).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
            drop(guard);
        });
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(r) = self.channels.get_mut(&channel) {
            if let Some(stdin) = r.stdin.as_mut() {
                if stdin.write_all(data).await.is_err() {
                    r.stdin = None;
                }
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(r) = self.channels.get_mut(&channel) {
            if let Some(mut stdin) = r.stdin.take() {
                let _ = stdin.shutdown().await;
            }
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        Ok(())
    }
}

fn bytes(v: Vec<u8>) -> russh::keys::ssh_encoding::bytes::Bytes {
    russh::keys::ssh_encoding::bytes::Bytes::from(v)
}

struct PushCtxParts {
    org: vcsstar_db::models::Org,
    repo: vcsstar_db::models::Repo,
    dir: std::path::PathBuf,
    principal: Principal,
    role: RepoRole,
    request_id: String,
}
