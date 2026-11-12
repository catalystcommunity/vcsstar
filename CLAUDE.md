# vcsstar

Self-hosted VCS collaboration platform: the GitHub interaction plane for an
organization, with GitHub kept as a mirrored golden source. Rust, sync
threads, CSIL on every wire. `README.md` is the front page and
`docs/architecture.md` shows the pieces with diagrams; keep both current
when a piece changes. Design documents live in `plan/` (local only,
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
  `events` is one plain table keyed `(lane, seq)`; rows are kept forever
  (no partitions, no retention, no maintenance job, by decision).
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
  overlay `~/.config/vcsstar/values-squizzlezig.yaml`, deployed with
  `~/vcsstar-redeploy.sh --build squizzlezig` (dev image from the working
  tree; see `~/agent-learning/vcsstar-design.md`). Production:
  `vcsstar.catalystsquad.com` through the reactorcide deploy workflow.
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
- `Rp/sign-request` asks for the claims `handle` (required), `display_name`
  and `email` (optional), and names the application `vcsstar`
  (`instance_keys::APPLICATION_ID`, the same id the instance keys and
  act-as grants use). LinkKeys 0.16.7 keys standing consent by user, RP
  domain, and application, so a person consents once per application, and
  the consent screen shows the display name of the instance's LinkKeys
  account: give that account a name people recognize. A server older than
  0.16.7 ignores the field. The linkkeys crates are pinned by git rev in
  `Cargo.toml` (`liblinkkeys`, `linkkeys-rpc-client`; v0.16.7 now).
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

- HTTP handlers that touch Postgres, corndogs, git, or the network wrap
  their body in `http::blocking` (`block_in_place`). Rocket 0.5 runs a
  sync handler on one of its four async workers; a few slow requests
  (an `import-repo` fetch) then stalled `/healthz` and the liveness probe
  restarted the pod.

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
  `hook-worker`, `mirror-worker`, `ingest-worker`, `repack-worker`.
  `vcsstard serve --roles
  api,git-ssh,dispatcher,sweeper,hook-worker,mirror-worker,ingest-worker,repack-worker`
  runs everything in one process.
- `repack-worker`: every ten minutes, the repository with the most live
  history segments (at least `VCSSTAR__REPACK_MIN_SEGMENTS`, default 20,
  and not repacked in six hours) is consolidated: `git repack -a -d -k -b`
  (unreachable objects kept, bitmap written) plus a commit graph, the one
  pack is uploaded with its `.bitmap`, `segments::consolidate` swaps the
  manifest, and the replaced objects are deleted. Claimed through
  `repos.repacked_at`. Pushes during the repack add segments after the
  ones it saw and are untouched. Hydration skips a segment whose object is
  gone and warns; the next operation reads the new manifest.
- Cache warm: a node with the `api` or `git-ssh` role hydrates every
  repository (segments and refs) in the background at start and every
  `VCSSTAR__CACHE_WARM_INTERVAL_S` (600) unless `VCSSTAR__CACHE_WARM=false`.
  The cache directory is ephemeral (emptyDir) by default; the object store
  is the truth and the warm pass rebuilds the cache after a restart.
  Metrics `vcsstar_cache_warm_total{outcome}`, `vcsstar_repack_total{outcome}`.
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
- `POST /hooks/github` verifies `X-Hub-Signature-256` first with the
  `webhook_secret` of every configured GitHub App (field of
  `SECRET_VCSSTAR__GITHUB_APPS`, or `SECRET_VCSSTAR__GITHUB_APP_WEBHOOK_SECRET`;
  `github::signed_by_app`), then with the repo's `webhook_inbound` set. An
  App-signed delivery without a repository (installation events) or for
  a repository with no mirror answers 202 (`not_mirrored`). The rest:
  verified with the `webhook_inbound` credential, captures the raw body into corndogs
  (key `github:<delivery id>`), and answers 202. For `push` the ingest
  worker verifies the hinted ref; it never applies the payload. A GitHub
  App hook and a repository hook deliver one event with the *same*
  delivery id and different bodies; corndogs refuses the second (code 4)
  and the receiver answers 202 `duplicate`
  (`vcsstar_github_webhooks_total{outcome="duplicate"}`).

## GitHub sync of pull requests (both ways)

- `vcsstar_core::github_sync`. Outbound: PR, comment, review, and
  `status_set` events become mirror actions (`pr:{n}`, `pr-comment:{id}`,
  `pr-review:{id}`, `status:{sha}:{ctx}`, purpose `mirror_api`); the
  executors create or patch the GitHub pull request, post the comment or
  review, or report the status. Inbound: `pull_request`,
  `pull_request_review`, `pull_request_review_comment`, `issue_comment`,
  `status`, and `check_run` webhooks upsert the vcsstar rows and emit the
  usual PR events.
- Shadow identities stand for GitHub accounts: `user_id` is UUID v5 of
  `https://api.github.com/user/{id}`, domain `github.com`, handle = login,
  table `github_identities`. They never log in. The dispatcher skips the
  mirror for every event whose actor is a shadow, so nothing GitHub caused
  is sent back. Rows remember their GitHub id (`pull_requests.github_number`,
  `pr_reviews.github_id`, `pr_comments.github_id`), and bodies vcsstar
  posts carry `<!-- vcsstar:<kind>:<id> -->` markers so an early webhook
  links instead of duplicating.
- Both-ways sync needs mirror mode `import_drift`: a branch pushed to
  GitHub (a PR head) is imported into vcsstar; in `authoritative_vcsstar`
  mode the reconcile pass deletes GitHub-only branches. A PR from a fork is
  imported (`head_remote = github:<owner/name>`) but a vcsstar fork PR is
  not created on GitHub. A squash or rebase merge on vcsstar closes the
  GitHub PR with a comment; a merge commit lets GitHub detect the merge
  itself. GitHub refuses a self-review from the mirror credential; the
  verdict is then posted as a review comment.
- A deleted base branch retargets its open PRs to the default branch
  (`pr_retargeted`). GitHub closes such PRs instead, and a closed PR cannot
  change its base, so the mirror moves the GitHub PRs first and deletes
  the branch after. A branch deleted on GitHub is deleted here when vcsstar
  did not move it since the mirror (else it is pushed back).
- A PR that GitHub closed because its base branch was deleted *there* is
  dead on GitHub (state and base cannot change any more); vcsstar closes
  its PR too. Delete the base of a stacked PR through vcsstar, or merge
  the stack bottom-up, to keep both open. GitHub also refuses a rebase
  merge of a stacked PR after the bottom was squashed; merge commits work.
- The runtime image is Debian trixie: rebase merges run
  `git merge-tree --merge-base`, which needs git 2.40 or newer.
- Never call `vcs::hydrate` for the pushed repository from inside the
  push hook path (`prs::on_push` and what it calls): materialize deletes
  refs that are gone from Postgres before `git receive-pack` writes them,
  and the push fails after the transaction committed.
- Metrics: `vcsstar_mirror_actions_total{kind,outcome}`,
  `vcsstar_github_webhooks_total{event,outcome}`,
  `vcsstar_github_ingest_total{event,outcome}`.
- Tracing: `VCSSTAR__OTLP_ENDPOINT` (OTLP/HTTP, `/v1/traces` appended)
  turns on span export; unset means logs only. Chart: `otlp.endpoint`,
  `metrics.serviceMonitor.enabled`.

## GitHub account links and the instance GitHub App

- `vcsstar_core::github_link`, table `github_links`. A user connects
  their GitHub account on the Account page: `/auth/github/start` sends
  the browser to the instance's GitHub App authorization and
  `/auth/github/callback` completes it (single-use `state`, 15 minutes,
  must be the signed-in identity; a callback without our state is sent
  through `start` again). `Auth/set-github-token` links by a pasted
  personal access token instead. `Auth/github-link-status` and
  `Auth/unlink-github` complete the ops. Tokens are sealed with
  `SECRET_VCSSTAR__CREDENTIAL_KEY`; App user tokens are refreshed five
  minutes before expiry; a refused token marks the link broken
  (`broken_reason`) and the user connects again.
- The instance has a list of GitHub Apps (`config.github_apps`):
  `SECRET_VCSSTAR__GITHUB_APPS` is a JSON list of `{app_id, client_id,
  client_secret, private_key, slug?}`; the older single App
  (`SECRET_VCSSTAR__GITHUB_APP_ID`, `_CLIENT_ID`, `_CLIENT_SECRET`,
  `_PRIVATE_KEY`, all four, and `VCSSTAR__GITHUB_APP_SLUG`) is added when
  its id is not in the list. The first App links user accounts;
  `github_links.app_id` records the issuing App, and refresh and revoke
  use it. GitHub limits the rate per installation, so
  `cred::best_install` looks up every App's installation for the
  repository or owner and takes the one not held with the most calls left.
  Token label `app:<app id>/<installation>`. Chart:
  `githubApp.existingSecret`, `githubApp.slug`; the deploy job writes
  Secret `vcsstar-github-app` from `GITHUB_APPS` (store key
  `catalystcommunity/vcsstar:<instance>-github-apps`, made by
  `tools/store-github-apps.sh`). `Mirror/list-github-installations`
  lists where the Apps are installed (home page). Every App needs the callback
  URL `<publicUrl>/auth/github/callback`, "Request user authorization
  during installation", permissions contents, pull requests, and commit
  statuses (write) plus metadata (read), and the webhook URL
  `<publicUrl>/hooks/github` with the secret given as the App's
  `webhook_secret` (`WEBHOOK_SECRET_FILE` for `tools/store-github-apps.sh`),
  or a member of the org's `webhook_inbound` set.
- Token order for every mirror operation (`cred::token_for_actor`): the
  event actor's own linked token, then the instance App's installation
  token when the App is installed for the repository (looked up by
  `GET /repos/{full}/installation`, cached per repository; "not
  installed" is remembered for ten minutes), then the credential set for
  the purpose. GitHub shows a linked user's changes under their login; the
  attribution line ("Synced from vcsstar ... opened by") is only added when
  the mirror acts with the App or a credential. The `<!-- vcsstar:... -->`
  marker is always added.
- Ownership of a pull request's title and body is by marker, not by
  actor: one born on vcsstar carries a marker in its GitHub body and
  keeps vcsstar's text; one born on GitHub has none and follows GitHub.
  Comment edits made on GitHub always apply here (last writer wins):
  vcsstar has no comment edit op, so nothing could re-assert its text.
- Echo guard: the ingest worker applies every delivery inside
  `outbox::with_origin("github", ..)`, which stamps `events.origin`;
  the dispatcher mirrors nothing with origin `github` and nothing with a
  shadow actor. A webhook whose sender is linked is attributed to the
  vcsstar identity, not to a shadow; linking also moves the rows the
  shadow owned to the identity.
- A terminal GitHub refusal of a linked user's token (401, bad
  credentials, "not accessible") marks the link broken and the action is
  retried with the next credential (`github_sync::settle_refusal`).
- Discovery: `Mirror/list-github-repos` lists the repositories under the
  organization's `github_owner` with the vcsstar repository that mirrors
  each one. Token order for reads that act as nobody
  (`cred::token_for_owner`): the instance App's installation on the owner
  (`GET /orgs|users/{owner}/installation`), the organization's credential
  set, then the caller's own link. `Mirror/import-repo` ("Enable" on the
  Org page, mode `import_drift`) uses the same order.
- Imports are durable work (`vcsstar_core::mirror_import`, worker
  `crates/vcsstar-workers/src/import.rs`, two threads of the
  `mirror-worker` role). `import-repo` and `import-all` ("Enable all":
  every GitHub repository of the owner, all pages) only make the row
  (paused, not enabled, `mirror_state.import_requested_*`) under origin
  `github` and submit a task to queue `import:github` (key
  `import:<repo id>:<request ms>`); the answer comes before any fetch.
  The worker claims the repository (`import_claimed_at`), fetches every
  branch and tag in batches, takes GitHub's default branch and
  description, records the refs under origin `github`, and enables the
  mirror. A failed attempt parks the task (1, 2, 4, 8, 16 min); after
  `MAX_ATTEMPTS` (6) the state is "failed" and a new request queues it
  again. A pod restart only delays a task (lease 30 min); a submission
  lost while corndogs was down is submitted again by the sweep
  (`resubmit_lost`, every 2 min). `MirrorStatus.import_state` and
  `GithubRepoInfo.import_state` report "queued", "failed", or "done";
  the Org page polls while imports are queued. Metric
  `vcsstar_github_import_total{outcome}`.
- Per-repository cadence: `mirror_state.pull_sync_interval_s` and
  `ref_reconcile_interval_s` (60 to 604800 s; NULL is 900 s), set with
  `Mirror/set-schedule` (0 restores the default; Mirror tab, Schedule).
  The worker's due queries read them (`due_filter`).
- A closed or merged pull request from GitHub's history keeps GitHub's
  base sha of the time and gets `merge_base_sha` from the merge commit's
  first parent (merged) or the base (closed) and the head
  (`prs::settle_history_base`, at import and lazily on `PullRequests/get`
  for older rows). Without it the Files view compared the head with
  today's base branch.

## GitHub outages, reconcile, and rate limits

- `vcsstar_core::github_health` is the instance's shared view of GitHub,
  kept in the tables `github_lanes` and `github_rate_limits` (process
  memory only when no `App` exists, as in unit tests). Two lanes, `api`
  and `git`, are circuit breakers: three transient failures in a row
  (5xx, network, 429, 403 with a limit) open a lane for 30 s, doubling to
  10 min; while it is open `mirror::execute` answers `Exec::Held` and the
  worker parks the task for the hold plus jitter without recording an
  attempt; the reconcile passes skip the lane. When the hold ends one
  probe goes through (the lane row is locked `FOR UPDATE`, so it is one
  probe across nodes); success closes the lane and sets `recovered_at`,
  which makes every repository due for both reconcile passes at once.
  A 4xx answer (401, 403 without a limit, 404, 422) is not a lane failure.
  A healthy lane writes its row at most once a minute.
- Rate limits are tracked per token label (`app:<app id>/<installation>`,
  `user:<login>`, a credential purpose, `tok:<hash>`) from the `X-RateLimit-*` headers of
  every response. Below the reserve (2 %, at least 50 calls) the token is
  held until its window resets; a `Retry-After` holds it for that long.
  `github_sync::client` and the pull pass check the hold first; a parked
  task waits at least as long as the longest hold (`longest_hold`).
- History import (`github_sync::backfill_pulls`, `crates/vcsstar-workers/
  src/backfill.rs`, a thread of the `mirror-worker` role): every mirrored
  repository imports all its GitHub pull requests, oldest first
  (`sort=created&direction=asc`), with reviews and comments (all pages)
  and, for open ones, statuses and check runs. Cursor `mirror_state.
  backfill_page`/`backfill_index`, saved after each pull request;
  `backfill_done_at` NULL means pending; `backfill_pass_at` is the claim
  and the round-robin schedule. Before the first page every
  `refs/pull/*/head` is fetched in one go (`mirror::fetch_pull_heads`) and
  captured as one segment. A pass spends at most `BACKFILL_BUDGET` (300)
  calls and stops at `github_health::bulk_hold` (reserve a tenth of the
  window, at least 250 calls), so live work keeps priority; nothing runs
  while the api lane is open. A pull request that fails three passes in a
  row (or fails for good) is skipped and named in `last_error`. Imports
  keep GitHub's numbers when free (`number_for_import`; the counter moves
  past them) and GitHub's `created_at`, `closed_at`/`merged_at`,
  `submitted_at`; a closed pull request never recreates its branch.
  `Mirror/backfill` starts it again; `MirrorStatus.backfill_*` reports it.
  Metrics `vcsstar_github_backfill_total{outcome}`,
  `vcsstar_github_backfill_pulls_total`. `Mirror/import-repo` fetches all
  refs in batched `git fetch` calls (`mirror::fetch_refs`).
- Webhooks are not retried by GitHub, so the mirror worker also runs a
  pull-based pass (`github_sync::reconcile_pulls`): every 15 minutes per
  repository (two repositories per minute per node, oldest first, and all
  of them again after the api lane recovers; the clock is
  `mirror_state.last_pull_pass_at`, claimed with a conditional update so
  worker nodes never run the same repository at once, as is
  `last_full_reconcile_at` for the ref pass) it lists the pull requests by
  `updated_at` newest first, stops at `mirror_state.last_pull_sync_at`,
  and replays the newer ones with their reviews, comments, statuses, and
  check runs through the webhook upserts under origin `github`. The first
  pass takes open pull requests only (the history import does the rest). A pass spends
  at most `PULL_SYNC_BUDGET` (150) calls and stops at a rate limit hold
  without moving the watermark past what it replayed (it replays oldest
  first and keeps the progress). Deletions made on GitHub are not seen by
  this pass. `Mirror/reconcile` runs it too and reports `pulls_checked`,
  `pulls_changed`, `pulls_complete`.
- Every inbound snapshot carries GitHub's `updated_at`; it is stored on
  `pull_requests.github_updated_at` and `pr_comments.github_updated_at`
  and a snapshot older than the one applied is ignored (redeliveries and
  out-of-order deliveries never revert a newer edit, close, or reopen).
- `Mirror/health` returns the lanes, the rate limits, and the backlog
  (pending and blocked actions, oldest pending). Metrics:
  `vcsstar_github_lane_open{lane}`,
  `vcsstar_github_lane_transitions_total{lane,to}`,
  `vcsstar_github_rate_limit_remaining`, `_limit`, `_reset_seconds`,
  `_held` (labels `token`, `app_id`, `installation`; from the shared rate
  table at scrape time, so every pod answers the same),
  `vcsstar_github_pull_sync_total{outcome}`,
  `vcsstar_hook_deliveries_total{outcome}`, the mirror action outcome
  `held`, and three gauges read from Postgres at scrape time on any node:
  `vcsstar_mirror_pending_actions`, `vcsstar_mirror_blocked_actions`,
  `vcsstar_mirror_lag_seconds`.

## Scaling out

- Nothing a request or a worker needs lives only in one pod. Sessions,
  tokens, link attempts, device codes, instance keys, mirror state, the
  lane breakers, the rate table, and the pass clocks are in Postgres;
  pending work is in corndogs under keyed submission and guarded claims,
  so any number of pods may claim from the same queues; every worker
  reads the desired state at execution time and is idempotent, and the
  dispatcher's rows (`hook_deliveries` unique per endpoint and event,
  `mirror_actions` one per key) tolerate two dispatchers expanding the
  same event.
- What stays per pod is a cache or a per-node limit by design: the
  repository cache directory (needs the object store; drop
  `cache.createClaim` to run more than one replica), the installation
  token cache, the login token buckets (`VCSSTAR__RATE_*` apply per
  node, so N api pods allow N times the rate), and pushes in flight (the
  hook socket is local to the pod running `git receive-pack`).
- `replicaCount` with every role in each pod scales the whole instance;
  separate deployments per role are a chart change, not a code change.

## Instance roles and the viewer's role

- `Instance/*` (`services/instance.rs`, table `instance_roles`): an
  instance admin gives an identity (`user_id@domain`, seen once) the role
  `admin` (as `INSTANCE_ADMINS`) or `viewer` (reads every repository,
  private ones too; no writes). `source` is `manual`; `claims` is
  reserved for a LinkKeys claim policy. The role is read when the
  credential resolves (`Principal.instance_role`); `authz::is_instance_admin`
  takes the configuration or the table. Events on lane `@instance`.
  Account page, "Instance roles" (instance admins only).
- The caller's role rides on the records: `WhoAmI.instance_role`,
  `Org.viewer_role` (owner, admin, member), `Repo.viewer_role` (admin,
  maintain, write, read; set by `get`, `list`, `list-forks`). The UI
  builds and requests only what the role allows: anonymous viewers see
  public repositories, the Org repository list, and the Code, Commits,
  Pull requests, and read-only Mirror tabs; members see members and
  teams; org admins get GitHub, credentials, service accounts, webhooks;
  repo admins get Settings, Webhooks, and the mirror schedule; merge
  needs write (the author may also close and reopen).

## Forks and pull requests across organizations

- `Repos/fork` copies refs and the segment manifest into an organization
  where the caller is admin ("Fork" on the repository page).
  `Repo.fork_of` names the parent; `Repos/list-forks` lists the direct
  forks the caller can see. A pull request goes into a repository from
  the repository itself or a direct fork of it, in any organization on
  the instance ("New pull request": into this repository or its parent,
  from the base or one of its forks). A same-repository pair shows the
  compare diff before it opens.

## Service accounts

- `ServiceAccounts/*` (`services/service_accounts.rs`, table
  `service_accounts`): an organization admin creates an identity that
  never signs in, with the key `<user_id>@svc.<instance host>` (a domain
  no LinkKeys server serves, so a login can never produce one), and mints
  API tokens for it with the same rules as `Auth/create-token` (scopes a
  subset of the caller's, `tokens:manage` for an API token caller, no
  act-as). Grant it roles like any identity. `disable` makes the principal
  resolver refuse its tokens ("identity disabled") until `enable`;
  `revoke-token` is final. Every change writes an org-lane event
  (`service_account_*`). The Org page has the section; a person's own
  tokens stay on the Account page (`Auth/create-token`).

## Web UI (phase 5)

- `ui/` is plain TypeScript on Zest web components (`@catalystcommunity/*`:
  base-components, controls, containers, menus, simple-router,
  design-tokens) built with Vite. No application framework. `npm ci && npm
  run build` writes `ui/dist`; `vcsstard` serves it when `VCSSTAR__UI_DIR`
  points there (the image sets it). `npm run dev` proxies `/csil`, `/auth`,
  and `/hooks` to `127.0.0.1:8080`.
- Structure: `src/main.ts` defines the Zest elements and starts the shell;
  `src/app.ts` is the shell (header, account menu, routing with
  `SimpleRouter`, session); `src/page.ts` is the page base class (a light
  DOM `BaseComponent` that builds one time in `build()`, loads in
  `start()`, subscribes in `connected()` with `this.own(...)`, and may
  accept a route change through `setPath()`); `src/dom.ts` is the `h()`
  builder and the control helpers; `src/ui.ts` holds badges, tables,
  sections, notices, `act()`, `confirmDanger()`, and the one-shot secret
  dialog; `src/pages/` holds the pages. After each `await`, stop when
  `this.isActive()` is false. Change the parts that changed; do not build a
  page again for a small change.
- Rules: Zest controls for every form control (`text-field`, `text-area`,
  `combo-box`, `check-box`, `spin-box`, `push-button`); `askConfirm` with
  `danger` before every action that cannot be undone; `showNotice` for the
  result of an action (an error notice stays until dismissed; a load error
  goes in the section's `Feedback`); a secret shown one time goes through
  `showSecretOnce` (a `dialog-window` with a Copy button); in-page tabs that
  are not routes use `tab-view`; routed tabs (the repository page) are
  links with `aria-current`. Empty a field after a submit with `clear()`,
  not `value = ""`, or the field shows "fill this out".
- Look: `src/styles.css` is the theme (dark blue, one gold accent, IBM Plex
  Sans and Mono from `@fontsource`), and it maps the palette to the
  `--zest-*` tokens, so a control follows the theme. Page rules live in
  `@layer page` before `zest`. The mark is `public/logo.svg` (the git tile
  with a star where the main node was) and `public/favicon.svg`.
- The browser speaks CSIL only: `ui/src/api.ts` wraps the generated async
  client with a fetch carrier (`POST /csil/v1/rpc`, `X-Vcsstar-Csrf`
  header, same-origin cookie). `ui/src/csil/` is the csilgen TypeScript
  transport, vendored. Generated TS field names are camelCase.
- Live updates: `ui/src/live.ts` speaks CSIL-Events (verbose profile) to
  `GET /csil/v1/events`; the gateway (`crates/vcsstard/src/events_ws.rs`)
  polls the outbox for the subscribed lanes every second.
- A section or tab that needs a sign-in is not built for an anonymous
  viewer, and its ops are not called (no error shown in its place). A
  signed-out viewer gets the Org page's repository list and the Code,
  Commits, Pull requests, and read-only Mirror tabs; another tab in the
  path opens Code.
- Code and diffs (`ui/src/render.ts`, `ui/src/pages/diff.ts`): markdown
  is markdown-it, sanitized with DOMPurify (HTML allowed, scripts never);
  relative links go to the code view, relative images load through
  `Repos/blob`. highlight.js (every language) and mermaid (`securityLevel:
  strict`) are dynamic imports. A file view highlights with line
  numbers, renders markdown and mermaid with a Source toggle, shows media
  inline, and has Raw (always plain text: an HTML or SVG file never runs
  as a page of this origin) and Download (bytes read in 2 MB pieces, up
  to 64 MB). The directory README renders under the listing. Diffs are
  side by side or in-line; `Auth/get-preferences`/`set-preferences`
  (`user_preferences.diff_layout`) keep a signed-in viewer's choice,
  anonymous viewers get side by side. A file's diff is drawn when it is
  first opened; the PR Files tab reads every page of `Repos/diff`.
- Class names in `@layer page` are global: check for a clash before you
  add one (`.split` and `.empty` exist).
- Every button calls a contract op. Nothing in the UI reaches the server
  by another path.
- The shell routes only links made with `link()` (`data-nav`). A link to a
  server-rendered page (`/auth/login`, `/auth/github/start`) is a plain
  `<a rel="external">`. The router also fires on window load, before the
  session is known: the shell ignores that call and renders after
  `whoami`, and it rebuilds the page when the session identity changes.

## Deployment

- `Containerfile` builds `vcsstard` (target `runtime`) and the CLI image
  (target `cli`). `helm/vcsstar` is the chart. Instance overlays describe a
  network and are never committed: `deploy/values-example.yaml` is the
  template, real overlays live in `~/.config/vcsstar/values-<instance>.yaml`,
  and the deploy job reads its overlay from the secret store
  (`catalystcommunity/vcsstar:values-catalystsquad` -> `DEPLOY_VALUES`).
- `.reactorcide/jobs/deploy.yaml` (`vcsstar-deploy`) knows no instance. An
  instance is an environment overlay, `.reactorcide/overlays/deploy-
  <instance>.yaml`: namespace and secret references only (hostnames and
  addresses stay in the store). `tools/deploy.sh dev <instance>` runs it
  through `reactorcide run-local` with the LOCAL store (`DEPLOY_MODE=dev`:
  tag `dev-<utc>-<sha>[-dirty]`, no `:latest`, no CLI image, pull Always);
  CI uses the PRODUCTION store through the workflow
  `vcsstar-deploy-catalystsquad` (node of the same name, `job_file:
  deploy.yaml` plus the overlay's environment; keep both in step) on push
  to `production`. The job writes Secrets `vcsstar-linkkeys`,
  `vcsstar-github-app`, `vcsstar-objectstore`, `vcsstar-backup-s3` from
  the store when their keys are set, without putting a value in argv.
- vcsstar.catalystsquad.com is an application of the todandlorna.com
  LinkKeys server (account `vcsstar@todandlorna.com`).
- Secrets reach the cluster from the reactorcide store through the deploy
  job (`vcsstar-linkkeys` Secret). Never put a secret value in values files.
- Postgres comes from CloudNativePG (`postgresql.operator: cnpg`, Secret
  `<fullname>-postgres-app`, Service `<fullname>-postgres-rw`, `backup`
  block for barman into the object store); `operator: zalando` is kept
  for an instance that has not moved. Rows are kept forever: no retention
  window and no maintenance job, by decision. CloudNativePG 1.31 removes
  the built-in barman; the chart's `backup` block then moves to the Barman
  Cloud Plugin. Future scale work is in `plan/15-future-ci-scale.md`.
- The repository cache is an emptyDir by default and the warm pass fills
  it from the object store after a restart. Without an object store set
  `cache.createClaim.enabled` (PVC, `Recreate` strategy) so history
  survives a pod restart; it still lives on one node and blocks a second
  replica.
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
