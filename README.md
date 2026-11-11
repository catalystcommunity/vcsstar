# vcsstar

vcsstar is a self-hosted collaboration platform for git: pushes, pull
requests, reviews, commit statuses, webhooks, and an API, with GitHub kept as
a mirrored copy in both directions. It exists because GitHub has outages and
work should not stop when it does. People can work on vcsstar or on GitHub,
and each side shows up on the other when GitHub can be reached.

- Rust services, one binary with roles (`api`, `git-ssh`, `dispatcher`,
  `sweeper`, `hook-worker`, `mirror-worker`, `ingest-worker`,
  `repack-worker`), stateless pods that scale out.
- Refs in Postgres with compare-and-swap, history as immutable packfile
  segments in an S3-compatible object store, node caches that rebuild
  themselves.
- Identity through LinkKeys (`user_id@domain`), API tokens, service accounts,
  and GitHub account links so vcsstar acts on GitHub as you.
- A CSIL contract (`csil/vcsstar.csil`) that every client, the web UI, and
  the `vcsstar` CLI are generated from. Every capability is an operation in
  that file first.

## Read first

| Document | What it covers |
| --- | --- |
| [`docs/architecture.md`](docs/architecture.md) | The pieces and how they fit, with diagrams: a push, the GitHub mirror both ways, outage behaviour, identity |
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

```sh
tools/csilgen.sh ensure     # pinned csilgen and generators under .deps/
tools/csilgen.sh generate   # crates/vcsstar-types and ui/src/generated
tools/csilgen.sh check      # drift check, also run in CI
```

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
crates/vcsstar-core      domain: authz, outbox, orgs, repos, PRs, GitHub sync
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
