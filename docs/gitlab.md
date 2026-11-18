# GitLab and repository sync

VCSStar connects one repository to multiple GitHub and GitLab repositories.
Changes to unprotected branches can start in any connected repository.
A protected branch accepts updates only from its configured authority.
The worker sends accepted changes to the other connections. GitLab.com and self-hosted GitLab use the
same adapter. Connections to other VCSStar instances are future work.

## Scope

Sync includes branches, tags, requests, comments, reviews, and commit
statuses. The first pass reads all request pages, including closed and
merged requests. It reads commit status history for branches, tags, and
request commits. Each source keeps a complete status history record. Other
destinations receive that record as an attributed summary. Later passes compare each item with its last accepted
copy at that connection.

VCSStar stores native collaboration records and the original provider
responses. A deletion removes the active local item where the data model
permits it. The stored record remains available through **Stored records**
on the repository **Sync** tab and `Mirror/list-sync-records`.

| Data | Sync rule |
| --- | --- |
| Description, default branch, archive state | Send changes to each connection |
| Visibility, membership, permissions | Keep local to each destination |
| Branch and tag protection rules | Keep local to each destination |
| Authentication and authorization | Each system controls its own rules |

A permission refusal blocks the affected operation. VCSStar does not change
access rules to make sync succeed. Local ref imports also obey VCSStar
branch protections. A resolver assignment does not grant remote access.
A preferred conflict source does not change the merge authority.

## Protected branches

Migration 20 adds missing rules for `main`, `master`, `dev`, `stage`,
`default`, and `@default`. The `@default` rule follows the current default
branch. Existing rules with the same pattern keep their values. New
repositories receive the same defaults.

The default rules require one approval and a pull request. A change to the
head or base invalidates the approval. The author and a delegated agent
cannot approve by default. Force updates and deletion are denied. Git
commit signatures are not required. An empty repository permits one
initial update to its default branch so that a pull request can be opened.

Use **Repository settings → Branch protections** or `Repos/set-protection`.
Set `merge_authority` to `local` or to a GitHub/GitLab connection ID. All
matching rules apply. Set the same authority in each matching rule,
including `@default` when applicable. An update from another connection
is refused even if the conflict policy selects it.

The configured GitHub or GitLab authority supplies ref state through its
authenticated provider connection. Configure its branch rules there.
VCSStar does not translate a copied review into local content approval.
Imported statuses do not retain a local base-SHA binding.

Options include the approval count, allowed approver accounts, author
approval, delegated approval, dismissal after head or base changes,
required checks, blocking change requests, merge methods, force updates,
and deletion. An empty
approver list permits accounts with write access. Required checks must
name trusted reporter accounts. With base approval enabled, both reviews
and check results must name the current base SHA. The latest review
decision replaces the previous decision by that reviewer. A comment alone
does not withdraw approval. Invalidated reviews stay invalidated even if
a branch returns to an earlier SHA.

`override_accounts` permits specified account identities to bypass the
matching protection rule. It does not grant repository access or change
the authority. An admin role alone is not an override. Configure release
accounts through this list if they need direct updates.

Configure authority and override accounts after deployment. New connections
start with inbound and outbound disabled. Set the branch rules, then enable
each direction on the repository Sync tab. No deployment bootstrap file is
required. A connection switch does not change branch authority or bypass
protection rules.

VCSStar-to-VCSStar connections are not implemented. Future connections
must use LinkKeys to authenticate instance metadata about Git objects and
events. The destination must verify the source, repository, branch, old
and new object IDs, and approval result before it updates a protected ref.
This trust requirement does not require Git commit signatures. GitHub and
GitLab connections do not require LinkKeys.

## Conflicts

Each repository selects a preferred source or a resolver identity or team.
A preferred source can be VCSStar or a connection. A new connection uses
its creator as the resolver until an administrator changes the policy.
An adopted legacy mirror without a policy uses repository administrators
as resolvers.

The worker compares each side with its own last accepted copy. If both
sides changed, it applies the conflict policy. It stops only the affected
item. Unrelated items continue. Deletions use the same policy.

A resolver selects the local or remote version on the **Sync** tab. The
worker reads both sides again before it applies the choice. If either side
changed, the item needs another decision. A summary must contain the full
stored data before it can replace a native record. Status history is an
audit record. Change the actual status at its source; an edited history
summary needs a resolver and does not replace the audit record.

## History and attribution

The worker uses a linked account when one is available. Otherwise, it uses
a repository or organization credential, or the instance GitHub App. It
includes the original author, time, and source link when available. Link a
GitLab account on the **Account** page. GitHub account links still work.

A destination does not always support the original record:

- A closed, merged, or fork request can use an attributed issue with its
  full data. Child comments and reviews attach to that record.
- GitLab receives historical reviews as attributed notes. This preserves
  the verdict and commit without making a bot approve another user's work.
- An inline comment can use a conversation comment with its original
  file and line. The full local record keeps the location.
- A merge made elsewhere can close the remote request and add its merge
  information. Sync does not execute another merge at the destination.
- A request deletion uses deletion where supported, or a closed request
  with an explanation. A review that cannot be deleted gets a deletion
  notice where the API permits one.

If neither the record nor a summary can be sent because of an API
capability limit, the worker marks that destination copy as omitted and
complete. VCSStar keeps the full record. Missing permissions, rate limits,
and network failures are errors; they are not successful omissions.

Markers and connection mappings let a retry find a copy that it already
created. Each connection stores separate local and remote baselines. A
summary can therefore differ from the native record without a sync loop.

## Setup

1. Apply all database migrations, including
   `2026111400000017_gitlab_mirrors` and `2026111400000018_star_sync`.
2. Configure `SECRET_VCSSTAR__CREDENTIAL_KEY` on the instance. The key
   encrypts stored credentials, account tokens, and webhook secrets.
3. On the organization page, add a credential with provider `gitlab`,
   method `pat`, purpose `mirror_api`, and the GitLab host. A GitLab token
   with `api` scope supports API changes and Git access, subject to its
   account permissions. See the [GitLab token documentation](https://docs.gitlab.com/user/profile/personal_access_tokens/).
4. Create a local repository or use **Import from GitLab** on the
   organization page. Imports first fetch Git data through the existing
   import queue. The connection worker then imports collaboration history.
5. On the repository **Sync** tab, select **Add a connection**. Enter a
   provider, host, and project path. Paths can include GitLab subgroups.
6. Select **Create the remote repository** if the target does not exist.
   The initial remote visibility is private. Teams manage visibility after
   creation. Leave the option clear to connect an existing repository.
7. Set the conflict policy. An identity can use `user_id@domain`. A team
   uses its VCSStar team ID.
8. Use **Webhook setup** to get the URL and secret. Create the webhook at
   the destination. Manual setup is the default. Select automatic setup
   only when VCSStar should create or update its own webhook.
9. Set branch authority and override accounts. Enable **Inbound** to apply
   remote changes to VCSStar. Enable **Outbound** to send VCSStar changes
   to the remote. You can enable either direction, both, or neither.

Repository admins can change either switch at any time. New connections
start with both directions disabled unless the request enables them.
Existing connections keep both directions enabled when migration 23 runs.
Their pause setting stays unchanged. Adopted legacy mirrors also keep
both directions enabled. A rollback to the old schema pauses any connection
that has a disabled direction.

Disabled directions retain their sync baselines. When enabled again, they
compare current changes with those baselines. Conflicts still need the
configured resolver or preferred source. A conflict decision cannot bypass
a disabled direction. An item already in progress can finish. The next
item uses the current settings. **Pause all** stops both enabled directions
without changing their switches. **Sync now** does not enable a direction.

Outbound sync still reads remote state to detect conflicts. When both
directions are off, the worker does not contact the provider. Authenticated
webhook data can still be received and stored. Remote repository creation
and automatic webhook setup require outbound to be enabled. For inbound
alone, create the remote repository and its webhook manually, or use
polling.

A token might permit content changes but refuse repository or webhook
creation. The connection error identifies the failed operation and tells
you to supply the required permissions or complete setup manually. Select
manual webhook setup if you cannot grant webhook permissions. Polling
continues without a webhook.

GitLab webhooks use `X-Gitlab-Token`. GitHub webhooks use an HMAC signature.
The receiver authenticates the request and submits the raw body to
corndogs before it returns success. The worker stores the body and reads
the current remote state. Paused connections retain received webhook data.

## API and operation

All controls use CSIL. The generated CLI exposes the same operations.

| Purpose | Mirror operations |
| --- | --- |
| Connections | `list-connections`, `connect`, `update-connection`, `sync-now` |
| Webhook setup | `webhook-secret` |
| Conflict policy | `get-policy`, `set-policy`, `list-conflicts`, `resolve-conflict` |
| Account links | `link-provider`, `list-provider-links`, `unlink-provider` |
| Stored history | `list-sync-records` |

`PullRequests/edit-comment` and `PullRequests/delete-comment` also work
through the UI and CLI. Only the author or a repository maintainer can
change a comment.

Example for an existing remote after login and credential setup:

```sh
vcsstar api Mirror connect \
  --set repo.org=example \
  --set repo.name=project \
  --set provider=gitlab \
  --set host=gitlab.com \
  --set project=group/subgroup/project
```

The `mirror-worker` role runs connection tasks on `sync:connections`.
Outbox events request another pass. Polling also runs approximately once
a minute. A repository lock permits one connection pass at a time. Each
item has its own mapping, accepted copies, error, and conflict state.
Incomplete API reads do not trigger deletion detection.

Existing enabled mirrors become connections after their Git import
finishes. Their provider, host, project, and pause state remain in place.
The old mode no longer controls them; the repository conflict policy does.
Legacy configure, pause, resume, reconcile, and backfill operations direct
callers to the connection operations after adoption. The legacy import
contract keeps its `github_repo` field for both providers.

## Prototype limits and validation

The implementation uses GitLab API v4 and HTTPS with certificate checks.
Hosts can include a port. HTTP, SSH credentials, URL path prefixes, IPv6
host literals, GitLab OAuth, project discovery, and bulk GitLab import are
not implemented. Federation is not part of this change.

The worker scans history on each pass. Large repositories can need many
API calls. Pagination errors stop deletion detection and the pass retries.
This prototype has no incremental history cursor for connection sync. If
a provider no longer exposes the Git objects of a closed request, VCSStar
keeps the request data and available history.

Local validation uses PostgreSQL, temporary Git repositories, and mock
provider APIs. It covers history, conflict resolution, deletion retention,
protection enforcement, connection isolation, and GitLab summaries. No
live GitLab or GitHub account was changed for validation. A live service
check remains a separate step before deployment.

## Next Actions

Review the implementation and local check results before commit and push.
Before deployment, check the connection against a disposable project on
each supported provider. Use that check to confirm the selected token
permissions and the installed GitLab version.
