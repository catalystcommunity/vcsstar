# vcsstar

Self-hosted VCS collaboration platform: the GitHub interaction plane for an
organization, with GitHub kept as a mirrored golden source. Rust, sync
threads, CSIL on every wire. Design documents live in `plan/` (local only,
gitignored); read `plan/00-overview.md` first and the document for the
area you touch. Ecosystem notes live in `~/agent-learning/`; read
`vcsstar-design.md`, `csilgen-usage.md`, `corndogs-client-behavior.md`,
`linkkeys-regular-rp-sdk.md`, `tls-for-csil-services.md`, and
`rust-services-on-kubernetes.md` before work that touches those systems.

## Rules that are not negotiable

- `csil/vcsstar.csil` is the contract. Every capability is an op there
  first. Generated code is committed and drift-checked in CI. Never edit
  generated files by hand.
- Every op is reachable from the `vcsstar` CLI with no per-op work. The
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
- No RP keys or RP crypto in vcsstar: all RP operations go through the
  domain linkkeys server's `Rp` service. Dial it with our own TcpStream and
  pinned rustls config, never `linkkeys_rpc_client::send_request`.
- Refs live in Postgres with compare-and-swap. History lives in immutable
  segments in an S3-compatible bucket. Node disks are disposable caches.
- Documentation follows ASD-STE100 Simplified Technical English.
- Test instance: `vcsstar.squizzlezig.com`, namespace `vcsstar-squizzlezig`,
  overlay `~/.config/vcsstar/values-squizzlezig.yaml`, deployed by hand (see
  `~/agent-learning/vcsstar-design.md`). Production: `vcsstar.catalystsquad.com`
  through the reactorcide deploy workflow.
- Never `git add`, `git commit`, or `git push` unless asked.

## Layout

```
csil/vcsstar.csil        contract (wire-ids on every service and op)
crates/vcsstar-types     generated Rust types + client + server (committed)
crates/vcsstar-transport carriers: Rocket mount, TCP, WS, events driver
crates/vcsstar-db        Diesel schema, models, repositories
crates/vcsstar-core      domain: authz, outbox writer, orgs/repos, PRs
crates/vcsstar-vcs       VcsBackend trait + git backend
crates/vcsstar-store     RepoStore: refs (Postgres) + segments (object store)
crates/vcsstar-workers   corndogs loops: dispatcher, sweeper, mirror, hooks
crates/vcsstard          server binary, role flags
crates/vcsstar-cli       `vcsstar` binary
migrations/              Diesel migrations (raw SQL, incl. partition DDL)
dev/                     docker compose for Postgres, corndogs, SeaweedFS
helm/                    chart for the catalyst cluster
.reactorcide/            CI workflows and jobs
```

## Browser and CLI login

- Browser pages are server-rendered by `vcsstard` (`/auth/login`,
  `/auth/callback`, `/auth/device`, `/auth/me`). The sign-in form is a
  native POST. The attempt token rides an HttpOnly cookie scoped to
  `/auth`; the session cookie is `vcsstar_session`.
- LinkKeys `Rp` ops go over TCP 4987 only, through
  `vcsstar_core::linkkeys::TcpRpCaller`. When `VCSSTAR__LINKKEYS_RP_ADDR`
  is unset, login is disabled and tokens still work.
- After `Rp/verify-assertion`, `login::complete` fetches the home domain's
  signing keys (`Rp/resolve-domain-keys`, stale results refused) and checks
  the `handle` and `email` claim signatures itself. A handle without a
  valid home-domain signature fails the login. An email is `email_verified`
  only with a valid signature; unsigned stays display data; a bad
  signature drops the value.
- The login entry points are rate limited per node with token buckets
  (`vcsstar_core::ratelimit`): `VCSSTAR__RATE_LOGIN_PER_MINUTE` (begin,
  complete, device begin; per peer and per typed identity, default 10),
  `VCSSTAR__RATE_DEVICE_POLL_PER_MINUTE` (per device code and peer, 40),
  `VCSSTAR__RATE_DEVICE_APPROVE_PER_MINUTE` (per approving principal, 6).
  `0` disables one. Over the limit answers `ResourceExhausted` (code 30)
  with "try again in N s" in the message and `retry_after_s=N` in
  `detail`; the CLI device poll backs off on it. Every refusal increments
  `vcsstar_rate_limited_total{entry}`.
- `GET /metrics` serves Prometheus text (`vcsstar_core::metrics`); the
  chart adds `prometheus.io/*` scrape annotations (`metrics.scrape`).
- CLI: `vcsstar login <url>` runs the device flow by default; the user
  approves the code at `/auth/device`. `--token-stdin` and `--token-file`
  store a token directly. First admin: `vcsstard bootstrap-token`.

## Git serving (phase 1)

- Roles: `api` serves CSIL-RPC, the browser pages, and git smart HTTP at
  `/{org}/{repo}.git/...`; `git-ssh` serves git over SSH with russh
  (`VCSSTAR__LISTEN_SSH`, user `git`, public-key auth against `ssh_keys`).
  Both roles start the hook socket (`VCSSTAR__HOOK_SOCKET`, under 108 bytes).
- Every operation hydrates the cache dir first (`vcs::hydrate`): refs are
  written from Postgres into the directory. The directory is never truth.
- A push runs `git receive-pack` with `VCSSTAR_PUSH_ID`; the `pre-receive`
  hook (`vcsstard hook pre-receive`) forwards the ref updates over the
  socket to `push::pre_receive`, which applies protections, CAS, and the
  outbox in one transaction. Rejections reach the pusher as `remote:` lines.
- HTTP auth for git: Basic with the API token as the password (any user
  name) or `Authorization: Bearer`. Anonymous reads work on public repos.
- Signatures: `AllowedSigners::build` writes every registered signing key
  as `user_id@domain namespaces="git" <key>`; `git verify-commit --raw`
  does the check. Status `verified` needs a key attested at commit time.
- `vcsstar keys enroll --api-key-file F` makes auth, signing, and quorum
  keys, enrolls at the home domain (`Account/enroll-application-instance`),
  and registers them with `Auth/register-key`; the server verifies through
  `Rp/resolve-application-keys`. `keys renew`, `keys status`,
  `keys revoke`, `keys git-config`. State: `$XDG_CONFIG_HOME/vcsstar/keys.json`.
- `vcsstar git-credential` is the credential helper; `vcsstar repo clone`
  installs it for the clone.

## Workers (phases 2 and 3)

- Roles: `dispatcher` (outbox -> hook deliveries + mirror actions, one
  keyed corndogs task each), `sweeper` (`CleanUpTimedOut` every 2 s),
  `hook-worker`, `mirror-worker`, `ingest-worker`. `vcsstard serve --roles
  api,git-ssh,dispatcher,sweeper,hook-worker,mirror-worker,ingest-worker`
  runs everything in one process.
- Queues: `hooks:{endpoint_id}`, `mirror:{org}/{repo}`, `ingest:github`.
  Task payloads are CBOR pointers; the desired state is read from Postgres
  when the task runs.
- Mirror credentials: `secret_ref` is `env:<VAR>` or `stored` (sealed with
  `SECRET_VCSSTAR__CREDENTIAL_KEY`). The token never enters argv: git gets
  it through `GIT_CONFIG_*` environment entries.
- Credentials with the same scope and purpose form a set: `set-credential`
  adds a member, `remove-credential` drops one by id, and every operation
  picks a member at random (`cred::find`); the webhook receiver accepts a
  signature by any member. The repository's own set shadows the
  organization's.
- `auth_method` is `pat` (the secret is the token) or `github_app` (the
  secret is JSON `{"app_id","installation_id","private_key"}`).
  `vcsstar_core::github_app` mints a ten-minute RS256 App JWT and exchanges
  it for a one-hour installation token, cached per credential on the node
  and dropped on a terminal GitHub failure. Workers see a plain token
  either way. `webhook_inbound` has no `github_app` form.
- `POST /hooks/github` verifies `X-Hub-Signature-256` with the repo's
  `webhook_inbound` credential, captures the raw body into corndogs, and
  answers 202. The ingest worker verifies the hinted ref; it never applies
  the payload.

## Web UI (phase 5)

- `ui/` is a SolidJS + Vite app. `npm ci && npm run build` writes `ui/dist`;
  `vcsstard` serves it when `VCSSTAR__UI_DIR` points there (the image sets
  it). `npm run dev` proxies `/csil`, `/auth`, and `/hooks` to
  `127.0.0.1:8080`.
- The browser speaks CSIL only: `ui/src/api.ts` wraps the generated async
  client with a fetch carrier (`POST /csil/v1/rpc`, `X-Vcsstar-Csrf`
  header, same-origin cookie). `ui/src/csil/` is the csilgen TypeScript
  transport, vendored. Generated TS field names are camelCase.
- Live updates: `ui/src/live.ts` speaks CSIL-Events (verbose profile) to
  `GET /csil/v1/events`; the gateway (`crates/vcsstard/src/events_ws.rs`)
  polls the outbox for the subscribed lanes every second.
- Every button calls a contract op. Nothing in the UI reaches the server
  by another path.

## Deployment

- `Containerfile` builds `vcsstard` (target `runtime`) and the CLI image
  (target `cli`). `helm/vcsstar` is the chart. Instance overlays describe a
  network and are never committed: `deploy/values-example.yaml` is the
  template, real overlays live in `~/.config/vcsstar/values-<instance>.yaml`,
  and the deploy job reads its overlay from the secret store
  (`catalystcommunity/vcsstar:values-catalystsquad` -> `DEPLOY_VALUES`).
  The reactorcide workflow `vcsstar-deploy-catalystsquad` builds and deploys
  on push to `production`.
- Secrets reach the cluster from the reactorcide store through the deploy
  job (`vcsstar-linkkeys` Secret). Never put a secret value in values files.
- The repository cache is an emptyDir by default. Without an object store
  set `cache.createClaim.enabled` (PVC, `Recreate` strategy) so history
  survives a pod restart; it still lives on one node.
- Object store keys come from `objectStore.existingSecret` with the keys
  `SECRET_VCSSTAR__OBJECT_STORE_ACCESS_KEY` and
  `SECRET_VCSSTAR__OBJECT_STORE_SECRET_KEY` (envFrom adds no prefix).
  `Repos/capture-history` backfills a repository whose history predates
  the store.
- The catalystsquad deploy workflow runs on push to the `production`
  branch, not `main`. Fast-forward `production` to deploy.

## Local development

```sh
docker compose -f dev/docker-compose.yml up -d
. dev/env.sh
diesel migration run
cargo build --workspace
cargo test --workspace
```

Ports: Postgres `127.0.0.1:55433`, corndogs `127.0.0.1:55080`, SeaweedFS
S3 `127.0.0.1:58333` (bucket `vcsstar-dev`, key `vcsstar-dev`).

Run the server with git serving for manual tests:

```sh
VCSSTAR__REPO_CACHE_DIR=/tmp/vcsstar-dev VCSSTAR__HOOK_SOCKET=/tmp/vcsstar-dev-hook.sock \
VCSSTAR__SSH_PUBLIC_HOST=127.0.0.1:2222 target/debug/vcsstard serve --roles api,git-ssh
```

`cargo test --workspace` with `DATABASE_URL` set also runs the database
tests (`login_flow`, `push_flow`); they skip without it.

Local builds link the system `libpq`. The CI runner image has none, so the
test job builds with `--features vcsstar-db/bundled-pq` (static libpq
without TLS). Production images keep the system library.

## Code generation

```sh
tools/csilgen.sh ensure     # installs the pinned csilgen + generators under .deps/
tools/csilgen.sh generate   # regenerates crates/vcsstar-types and ui/src/generated
tools/csilgen.sh check      # fails if generated output differs (CI drift check)
```

The pin lives in `tools/csilgen.sh` (`CSILGEN_PIN`). Generators load from
`./.generators` (a link into `.deps/`), never from `$HOME/.csilgen`.

## Conventions

- Errors cross the wire as `ServiceError { code, message, detail }`; codes
  are in `crates/vcsstar-core/src/errors.rs` and never reused.
- `Ctx` is the first field of every request record.
- Config is env-first with the prefix `VCSSTAR__` (double underscore, so
  Kubernetes Service links never match). Secrets use `SECRET_VCSSTAR__`.
- Every binary installs a SIGTERM handler and drains: fail readiness, stop
  listeners, wait bounded, stop loops, exit 0.
- Tests that need Postgres read `DATABASE_URL` and skip with a message when
  it is unset.

## Toolchains

Cross-language builds use the shared `catalyst-tools` bundle:

```sh
source "${CATALYST_TOOLS:-$HOME/.local/catalyst-tools}/env.sh"
```
