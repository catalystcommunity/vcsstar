# GitLab prototype

This prototype adds GitLab project import and Git mirroring. It supports
GitLab.com and self-hosted servers with HTTPS. Project paths can include
subgroups, for example `group/subgroup/project`.

The prototype has not been built or tested. No migration or deployment was
run for this change.

## Setup

1. Apply migration `2026111400000017_gitlab_mirrors` before you start a server
   with this code. Existing mirrors receive provider `github` and host
   `github.com`.
2. Create the project on GitLab if it does not exist.
3. Create a personal, project, or group access token. For import and
   mirroring, give the token `read_api` and `write_repository` scopes.
   For import without push access, use `read_api` and `read_repository`.
   The token account must have access to the project. See the
   [GitLab token documentation](https://docs.gitlab.com/user/profile/personal_access_tokens/).
4. Open the vcsstar organization page. Under **Mirror credentials**, select
   provider `gitlab`, method `pat`, and purpose `mirror_git_push`. Enter the
   GitLab host and token. Use the same host for the import. The stored token
   uses the existing instance credential key.
5. Under **Import from GitLab (prototype)**, enter the host, project path,
   and local repository name. Select **Import project**.
6. Open the repository **Mirror** tab to read import progress. Use
   **Reconcile now** to request a branch and tag pass.

Imports default to private visibility and mode `import_drift`. A non-public
GitLab project cannot be imported into a public vcsstar repository. The
worker reads the default branch and description from the
[GitLab Projects API](https://docs.gitlab.com/api/projects/).

For an existing vcsstar repository, use its **Mirror** tab to configure a
GitLab target. Create that target on GitLab first. A configured mirror
cannot change its provider, host, or project path. Use a new local repository
for a different target.

## API and CLI

`Mirror/import-repo` and `Mirror/configure` accept optional `provider` and
`host` fields. `Mirror/status` returns these fields. The default provider is
`github`. The default host is `github.com` or `gitlab.com`, as applicable.
Use a hostname with an optional port, without a scheme or path.

The existing `github_repo` field holds the remote project path for either
provider. Its name stays unchanged for existing clients. Do not include a
URL or a `.git` suffix.

Example after CLI login and credential setup:

```sh
vcsstar api Mirror import-repo \
  --set org=example \
  --set name=project \
  --set provider=gitlab \
  --set host=gitlab.com \
  --set github_repo=group/subgroup/project \
  --set mode=import_drift \
  --set visibility=private
```

`Mirror/set-credential` accepts `provider=gitlab` and `auth_method=pat`.
The supported purposes are `import_fetch`, `mirror_git_push`, and
`mirror_api`. An import can use any of these purposes. Git push can use
`mirror_git_push` or `mirror_api`. Repository credentials take precedence
over organization credentials for the same provider, host, and purpose.
Public projects can be imported without a token. Push operations need one.

## Operation and limits

- The import worker uses the existing durable queue. Its legacy name remains
  `import:github`. The task identifies a repository; its database row selects
  the provider.
- Branch and tag actions use the existing lease checks, retries, blocked
  action list, and replay operation. The default reconcile interval is
  900 seconds. The three existing mirror modes are available.
- `import_drift` imports remote branches and compatible ref changes.
  `authoritative_vcsstar` can remove branches that exist only on GitLab.
  `flag_drift` reports remote changes for review.
- Merge requests, comments, reviews, commit statuses, releases, and project
  settings do not sync. There is no GitLab webhook receiver, account link,
  project discovery, or bulk import. GitHub discovery remains GitHub only.
- The prototype uses HTTPS with certificate verification. It does not
  support HTTP, SSH credentials, URL path prefixes, IPv6 host literals,
  or redirects. Use the final GitLab hostname and project path.
- GitLab API failures with status 429 or 5xx use import retries. There is no
  GitLab rate limit table or shared outage control yet. GitLab transfers do
  not change the GitHub outage state.

## Next Actions

Keep this change as an untested prototype. Before operational use, review
the migration and validate private import, nested project paths, token
selection, ref changes, and retries against a disposable GitLab project.
