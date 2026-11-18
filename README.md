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
