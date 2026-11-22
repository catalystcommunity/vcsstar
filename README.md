# vcsstar

VCSStar is a self-hosted collaboration platform for Git. One repository
can connect to multiple GitHub and GitLab repositories. People can change
code and collaboration records in any connected system. VCSStar sends
those changes to the other destinations under the repository conflict
policy. Protected branches accept updates from one configured authority.
Each destination keeps control of its own access rules.

- Rust services, one binary with roles (`api`, `git-ssh`, `dispatcher`,
  `sweeper`, `hook-worker`, `mirror-worker`, `ingest-worker`,
  `repack-worker`), stateless pods that scale out.
- Refs in Postgres with compare-and-swap, history as immutable packfile
  segments in an S3-compatible object store, node caches that rebuild
  themselves.
- Login through LinkKeys or an administrator-created local account. A
  LinkKeys binding permanently disables local password login for that account.
  API tokens, service accounts, and provider account links remain available.
- Default branch protections cover `main`, `master`, `dev`, `stage`,
  `default`, and the current default branch. Git signatures are not required.
  Overrides name specific accounts; an admin role alone does not bypass a rule.
- GitHub and GitLab imports include closed and merged requests, comments,
  reviews, and statuses. Each connection has separate mappings and sync
  progress. API limits can use attributed summaries.
- Conflict policies select a preferred source or an assigned resolver.
  Conflicts stop only the affected item. See [GitLab and sync setup](docs/gitlab.md)
  for setup, API operations, and prototype limits.
- Forks into any organization on the instance and pull requests back from
  them; side by side or in-line diffs with syntax highlighting; rendered
  markdown and mermaid diagrams.
- Roles on organizations, repositories, and the instance (admin, viewer);
  the UI shows each caller only what their role allows.
- A CSIL contract (`csil/vcsstar.csil`) that every client, the web UI, and
  the `vcsstar` CLI are generated from. Every capability is an operation in
  that file first.

## Install the CLI

Open the [GitHub releases](https://github.com/catalystcommunity/vcsstar/releases).
Select a `vcsstar/vX.Y.Z` release. Download the archive for your system.
Each archive has the name `vcsstar-X.Y.Z-<target>.tar.gz`.

| System | x86-64 | ARM64 |
| --- | --- | --- |
| Linux with glibc 2.28 or later | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-gnu` | `aarch64-pc-windows-gnullvm` |

Extract the archive with `tar -xf <archive>`. Put the executable in a
directory on your `PATH`. Its name is `vcsstar.exe` on Windows and
`vcsstar` on Linux and macOS. To build from this checkout,
run `cargo install --locked --path crates/vcsstar-cli`.

```sh
vcsstar --version
vcsstar login https://vcsstar.catalystsquad.com
vcsstar whoami
```

For another instance, use its URL in the login command. Open the URL that
the CLI prints and approve the code. The CLI saves the instance and token
in a profile for subsequent commands.

## Run a server with Docker Compose

Use Docker Compose v2 on a host that can run Linux x86-64 containers.
Download `compose.yaml` from the same GitHub release to use its server
version. The [Compose file in this checkout](compose.yaml) uses the
`stable` image tag. Set `VCSSTAR_TAG` to select a specific release.

1. In the directory with `compose.yaml`, run this command once with
   Python 3. Keep the resulting `.env` file for subsequent starts and backups.

   ```sh
   python3 - <<'PYTHON'
   import base64, os, secrets, uuid
   os.umask(0o077)
   with open(".env", "x") as config:
       print("POSTGRES_PASSWORD=" + secrets.token_hex(24), file=config)
       print("VCSSTAR_CREDENTIAL_KEY=" + base64.b64encode(secrets.token_bytes(32)).decode(), file=config)
       print("VCSSTAR_ADMIN_ID=" + str(uuid.uuid4()), file=config)
   PYTHON
   ```

2. Start the services. The readiness check returns `ready` when the server
   can use PostgreSQL.

   ```sh
   docker compose up -d
   curl --retry 30 --retry-delay 1 --retry-all-errors -fsS http://127.0.0.1:8080/readyz
   ```

3. Create the first administrator token and send it directly to the CLI.
   Use the `local` profile to check the account.

   ```sh
   docker compose exec -T vcsstar sh -c \
     'exec vcsstard bootstrap-token --identity "$VCSSTAR__INSTANCE_ADMINS" --handle admin' |
     vcsstar login http://127.0.0.1:8080 --name local --token-stdin
   vcsstar --profile local whoami
   ```

For browser access, create a local account. Enter a password of at least
15 characters at the prompt.

```sh
python3 -c 'import getpass, json; print(json.dumps({"username": "owner", "password": getpass.getpass("Password: ")}))' |
  vcsstar --profile local api Instance create-local-account --input -
```

Open `http://127.0.0.1:8080/auth/login` and use the local account form to
sign in as `owner`. The `local` CLI profile keeps the administrator token.

Compose binds HTTP and Git SSH to localhost. PostgreSQL stores metadata,
Corndogs stores queued work, and the server stores Git history and the SSH
host key in named volumes. Keep all three volumes and the credential key.
Use `docker compose down` to stop the instance and keep its data.

## Read first

| Document | What it covers |
| --- | --- |
| [`docs/architecture.md`](docs/architecture.md) | The pieces and how they fit, with diagrams: a push, the GitHub mirror both ways, outage behaviour, identity |
| [`docs/gitlab.md`](docs/gitlab.md) | GitLab and GitHub connection setup, conflict policy, and limits |
| [`CLAUDE.md`](CLAUDE.md) | The rules each piece follows, roles, configuration, local development, deployment |
| [`csil/vcsstar.csil`](csil/vcsstar.csil) | The contract |
| [`deploy/values-example.yaml`](deploy/values-example.yaml) | Template for an instance overlay (real overlays never live in the repository) |

## Local development

```sh
docker compose -f dev/docker-compose.yml up -d   # Postgres, corndogs, SeaweedFS
. dev/env.sh
diesel migration run
cargo build --workspace
cargo test --workspace
```

Run the server with git serving:

```sh
VCSSTAR__REPO_CACHE_DIR=/tmp/vcsstar-dev VCSSTAR__HOOK_SOCKET=/tmp/vcsstar-dev-hook.sock \
VCSSTAR__SSH_PUBLIC_HOST=127.0.0.1:2222 target/debug/vcsstard serve --roles api,git-ssh
```

The web UI lives in `ui/` (`npm ci && npm run build`); `vcsstard` serves
`ui/dist` when `VCSSTAR__UI_DIR` points at it.

## Code generation

Use the Rust version in `rust-toolchain.toml` and install its `rustfmt` component.
The commands below use the csilgen release set in `tools/csilgen.sh`.

```sh
tools/csilgen.sh ensure     # pinned csilgen and generators under .deps/
tools/csilgen.sh generate   # crates/vcsstar-types and ui/src/generated
tools/csilgen.sh check      # drift check, also run in CI
```

## Releases

After the checks on `main` pass, Reactorcide uses conventional commits to
select the next `vcsstar/vX.Y.Z` tag. The tag starts six CLI builds and one
server image build. The publish job checks the archives and attaches a
Compose file that uses the release version. The public server image is
`containers.catalystsquad.com/public/catalystcommunity/vcsstar:X.Y.Z`.

Before the first release, enable the `push` and `tag_created` events for
the Reactorcide project. Apply the [release job grants](.reactorcide/secret-grants.yaml). The tag and GitHub release jobs use the CI
GitHub credential. The server job uses the container registry credential.

## Deployment

`Containerfile` builds the server and CLI images; `helm/vcsstar` is the chart.
Postgres comes from CloudNativePG, work capture from corndogs, history from any
S3-compatible store. Secrets reach the cluster from the reactorcide secret
store through the deploy job; instance overlays describe a network and are
never committed. See the deployment section of `CLAUDE.md`.

## Layout

```
csil/vcsstar.csil        contract (wire-ids on every service and op)
crates/vcsstar-types     generated Rust types, client, server (committed)
crates/vcsstar-transport carriers: Rocket mount, TCP, WS, events driver
crates/vcsstar-db        Diesel schema, models, migrations runner
crates/vcsstar-core      domain: authz, outbox, orgs, repos, PRs, connection sync
crates/vcsstar-vcs       VcsBackend trait and the git backend
crates/vcsstar-store     refs (Postgres) and history segments (object store)
crates/vcsstar-workers   corndogs loops: dispatcher, sweeper, mirror, hooks, ingest, repack
crates/vcsstard          server binary with role flags
crates/vcsstar-cli       the vcsstar CLI
migrations/              Diesel migrations (raw SQL)
ui/                      web UI: TypeScript on Zest web components
helm/                    chart; .reactorcide/ CI and deploy jobs
```

Documentation follows ASD-STE100 Simplified Technical English.

## License

vcsstar is licensed under the Apache License, Version 2.0. Refer to
[`LICENSE`](LICENSE) for the full text.
