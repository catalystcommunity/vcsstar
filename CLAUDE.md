# vscstar

Self-hosted VCS collaboration platform: the GitHub interaction plane for an
organization, with GitHub kept as a mirrored golden source. Rust, sync
threads, CSIL on every wire. Design documents live in `plan/` (local only,
gitignored); read `plan/00-overview.md` first and the document for the
area you touch. Ecosystem notes live in `~/agent-learning/`; read
`vscstar-design.md`, `csilgen-usage.md`, `corndogs-client-behavior.md`,
`linkkeys-regular-rp-sdk.md`, `tls-for-csil-services.md`, and
`rust-services-on-kubernetes.md` before work that touches those systems.

## Rules that are not negotiable

- `csil/vscstar.csil` is the contract. Every capability is an op there
  first. Generated code is committed and drift-checked in CI. Never edit
  generated files by hand.
- Every op is reachable from the `vscstar` CLI with no per-op work. The
  generated CLI layer is built from the schema descriptor; a test fails the
  build if the subcommand set differs from the contract.
- The web UI has no private endpoints.
- corndogs is durable work capture only. One task per unit of pending
  work, keyed submission and guarded claims only (0.8.0 `required`
  policies). Workers are idempotent and read desired state from Postgres.
- Inbound items (GitHub webhooks, peer events) are ingest-first:
  authenticate, submit the raw item to corndogs, ack. No processing in the
  request cycle.
- Every state change writes the outbox (`events`) in the same transaction.
- Identity keys are `user_id@domain`. Handles are display data and are
  never unique. Cache application keys on `user_id@domain + application +
  instance`, never on a handle.
- No RP keys or RP crypto in vscstar: all RP operations go through the
  domain linkkeys server's `Rp` service. Dial it with our own TcpStream and
  pinned rustls config, never `linkkeys_rpc_client::send_request`.
- Refs live in Postgres with compare-and-swap. History lives in immutable
  segments in an S3-compatible bucket. Node disks are disposable caches.
- Documentation follows ASD-STE100 Simplified Technical English.
- Never `git add`, `git commit`, or `git push` unless asked.

## Layout

```
csil/vscstar.csil        contract (wire-ids on every service and op)
crates/vscstar-types     generated Rust types + client + server (committed)
crates/vscstar-transport carriers: Rocket mount, TCP, WS, events driver
crates/vscstar-db        Diesel schema, models, repositories
crates/vscstar-core      domain: authz, outbox writer, orgs/repos, PRs
crates/vscstar-vcs       VcsBackend trait + git backend
crates/vscstar-store     RepoStore: refs (Postgres) + segments (object store)
crates/vscstar-workers   corndogs loops: dispatcher, sweeper, mirror, hooks
crates/vscstard          server binary, role flags
crates/vscstar-cli       `vscstar` binary
migrations/              Diesel migrations (raw SQL, incl. partition DDL)
dev/                     docker compose for Postgres, corndogs, SeaweedFS
helm/                    chart for the catalyst cluster
.reactorcide/            CI workflows and jobs
```

## Browser and CLI login

- Browser pages are server-rendered by `vscstard` (`/auth/login`,
  `/auth/callback`, `/auth/device`, `/auth/me`). The sign-in form is a
  native POST. The attempt token rides an HttpOnly cookie scoped to
  `/auth`; the session cookie is `vscstar_session`.
- LinkKeys `Rp` ops go over TCP 4987 only, through
  `vscstar_core::linkkeys::TcpRpCaller`. When `VSCSTAR__LINKKEYS_RP_ADDR`
  is unset, login is disabled and tokens still work.
- CLI: `vscstar login <url>` runs the device flow by default; the user
  approves the code at `/auth/device`. `--token-stdin` and `--token-file`
  store a token directly. First admin: `vscstard bootstrap-token`.

## Deployment

- `Containerfile` builds `vscstard` (target `runtime`) and the CLI image
  (target `cli`). `helm/vscstar` is the chart; instance values live in
  `deploy/values-<instance>.yaml`. The reactorcide workflow
  `vscstar-deploy-catalystsquad` builds and deploys on push to `main`.
- Secrets reach the cluster from the reactorcide store through the deploy
  job (`vscstar-linkkeys` Secret). Never put a secret value in values files.

## Local development

```sh
docker compose -f dev/docker-compose.yml up -d
. dev/env.sh
diesel migration run
cargo build --workspace
cargo test --workspace
```

Ports: Postgres `127.0.0.1:55433`, corndogs `127.0.0.1:55080`, SeaweedFS
S3 `127.0.0.1:58333` (bucket `vscstar-dev`, key `vscstar-dev`).

## Code generation

```sh
tools/csilgen.sh ensure     # installs the pinned csilgen + generators under .deps/
tools/csilgen.sh generate   # regenerates crates/vscstar-types and ui/src/generated
tools/csilgen.sh check      # fails if generated output differs (CI drift check)
```

The pin lives in `tools/csilgen.sh` (`CSILGEN_PIN`). Generators load from
`./.generators` (a link into `.deps/`), never from `$HOME/.csilgen`.

## Conventions

- Errors cross the wire as `ServiceError { code, message, detail }`; codes
  are in `crates/vscstar-core/src/errors.rs` and never reused.
- `Ctx` is the first field of every request record.
- Config is env-first with the prefix `VSCSTAR__` (double underscore, so
  Kubernetes Service links never match). Secrets use `SECRET_VSCSTAR__`.
- Every binary installs a SIGTERM handler and drains: fail readiness, stop
  listeners, wait bounded, stop loops, exit 0.
- Tests that need Postgres read `DATABASE_URL` and skip with a message when
  it is unset.

## Toolchains

Cross-language builds use the shared `catalyst-tools` bundle:

```sh
source "${CATALYST_TOOLS:-$HOME/.local/catalyst-tools}/env.sh"
```
