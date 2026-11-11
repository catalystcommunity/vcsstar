import { createResource, createSignal, For, Show } from "solid-js";
import { A, useParams } from "@solidjs/router";
import { api, ctx, errorText, who, when } from "../api";
import type { GithubRepoInfo } from "../generated/types.gen.ts";
import { HooksPanel } from "./Repo";

export function OrgPage() {
  const params = useParams();
  const org = () => params.org!;
  const [info] = createResource(org, (o) => api.orgs.get({ ctx: ctx(), org: o }));
  const [repos, { refetch: refetchRepos }] = createResource(org, (o) => api.repos.list({ ctx: ctx(), org: o, page: { limit: 200 } }));
  const [members, { refetch: refetchMembers }] = createResource(org, (o) => api.orgs.listMembers({ ctx: ctx(), org: o, page: { limit: 200 } }).catch(() => null));
  const [teams, { refetch: refetchTeams }] = createResource(org, (o) => api.orgs.listTeams({ ctx: ctx(), org: o }).catch(() => null));
  const [creds, { refetch: refetchCreds }] = createResource(org, (o) => api.mirror.listCredentials({ ctx: ctx(), target: { org: o } }).catch(() => null));
  const [accounts, { refetch: refetchAccounts }] = createResource(org, (o) => api.serviceAccounts.list({ ctx: ctx(), org: o }).catch(() => null));
  const [err, setErr] = createSignal("");
  const [ghPage, setGhPage] = createSignal<string | undefined>(undefined);
  const [ghErr, setGhErr] = createSignal("");
  const [gh, { refetch: refetchGh }] = createResource(
    () => [org(), ghPage(), info()?.githubOwner] as const,
    async ([o, cursor, owner]) => {
      setGhErr("");
      if (!owner) return null;
      try {
        return await api.mirror.listGithubRepos({ ctx: ctx(), org: o, page: { cursor, limit: 100 } });
      } catch (e) {
        setGhErr(errorText(e));
        return null;
      }
    },
  );
  const [enabling, setEnabling] = createSignal<string | null>(null);
  const [newAccount, setNewAccount] = createSignal({ name: "", description: "" });
  const [tokenFor, setTokenFor] = createSignal<string | null>(null);
  const [newToken, setNewToken] = createSignal({ name: "", scopes: "repo:read,pr:read" });
  const [minted, setMinted] = createSignal<{ account: string; secret: string } | null>(null);
  const [tokens, { refetch: refetchTokens }] = createResource(
    () => [org(), tokenFor()] as const,
    async ([o, name]) => (name ? api.serviceAccounts.listTokens({ ctx: ctx(), org: o, name }).catch(() => null) : null),
  );
  const [member, setMember] = createSignal({ identity: "", role: "member" });
  const [team, setTeam] = createSignal("");
  const [cred, setCred] = createSignal({ purpose: "mirror_git_push", authMethod: "pat", secretRef: "stored", secret: "" });

  const run = async (f: () => Promise<unknown>, after?: () => void) => {
    setErr("");
    try {
      await f();
      after?.();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <h1>
        {info()?.displayName ?? org()} <span class="muted small">{org()}</span>
      </h1>
      <Show when={info()?.githubOwner}>
        <p class="muted">GitHub owner: {info()!.githubOwner}</p>
      </Show>
      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>

      <h2>Repositories</h2>
      <ul>
        <For each={repos()?.repos ?? []}>
          {(r) => (
            <li>
              <A href={`/${r.org}/${r.name}`}>{r.name}</A> <span class="muted small">{r.visibility}</span>
            </li>
          )}
        </For>
      </ul>

      <Show when={info()?.githubOwner && (gh() || ghErr())}>
        <h2>GitHub repositories</h2>
        <p class="muted small">
          Repositories under <code>{info()!.githubOwner}</code> on GitHub
          <Show when={gh()}> as seen by <code>{gh()!.source}</code>{gh()!.installationId ? ` (installation ${gh()!.installationId})` : ""}</Show>. Enable one to
          import it and keep both sides in sync (mode <code>import_drift</code>).
        </p>
        <Show when={ghErr()}>
          <p class="err">{ghErr()}</p>
        </Show>
        <Show when={gh()}>
          <table>
            <thead>
              <tr>
                <th>Repository</th>
                <th>Visibility</th>
                <th>Default branch</th>
                <th>Last push</th>
                <th>Here</th>
              </tr>
            </thead>
            <tbody>
              <For each={gh()!.repos}>
                {(r: GithubRepoInfo) => (
                  <tr>
                    <td>
                      <a href={r.htmlUrl} rel="external noreferrer" target="_blank">
                        {r.fullName}
                      </a>
                      <Show when={r.description}>
                        <div class="muted small">{r.description}</div>
                      </Show>
                    </td>
                    <td class="small">
                      {r.private ? "private" : "public"}
                      {r.archived ? " · archived" : ""}
                      {r.fork ? " · fork" : ""}
                    </td>
                    <td class="mono small">{r.defaultBranch}</td>
                    <td class="muted small">{r.pushedAt ? when(r.pushedAt) : "—"}</td>
                    <td>
                      <Show
                        when={r.mirroredAs}
                        fallback={
                          <button
                            disabled={enabling() === r.fullName}
                            onClick={() => {
                              setEnabling(r.fullName);
                              run(
                                () =>
                                  api.mirror.importRepo({
                                    ctx: ctx(),
                                    org: org(),
                                    name: r.fullName.split("/")[1]!,
                                    githubRepo: r.fullName,
                                    mode: "import_drift",
                                    visibility: r.private ? "private" : "public",
                                  }),
                                () => {
                                  setEnabling(null);
                                  refetchGh();
                                  refetchRepos();
                                },
                              ).finally(() => setEnabling(null));
                            }}
                          >
                            {enabling() === r.fullName ? "Importing…" : "Enable"}
                          </button>
                        }
                      >
                        <A href={`/${org()}/${r.mirroredAs}`}>{r.mirroredAs}</A>
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
          <p class="row">
            <Show when={ghPage()}>
              <button class="link" onClick={() => setGhPage(undefined)}>
                first page
              </button>
            </Show>
            <Show when={gh()!.nextCursor}>
              <button class="link" onClick={() => setGhPage(gh()!.nextCursor!)}>
                more
              </button>
            </Show>
          </p>
        </Show>
      </Show>

      <Show when={members()}>
        <h2>Members</h2>
        <table>
          <tbody>
            <For each={members()!.members}>
              {(m) => (
                <tr>
                  <td>{who(m.identity)}</td>
                  <td>{m.role}</td>
                  <td>
                    <button class="link danger" onClick={() => run(() => api.orgs.removeMember({ ctx: ctx(), org: org(), identity: `${m.identity.userId}@${m.identity.domain}` }), refetchMembers)}>
                      remove
                    </button>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.orgs.setMember({ ctx: ctx(), org: org(), identity: member().identity, role: member().role }), () => {
              setMember({ identity: "", role: "member" });
              refetchMembers();
            });
          }}
        >
          <input placeholder="user_id@domain" size={44} value={member().identity} onInput={(e) => setMember({ ...member(), identity: e.currentTarget.value })} />
          <select value={member().role} onChange={(e) => setMember({ ...member(), role: e.currentTarget.value })}>
            <option>member</option>
            <option>admin</option>
            <option>owner</option>
          </select>
          <button disabled={!member().identity}>Add member</button>
        </form>
        <p class="muted small">A person appears here after one sign-in; their identity is user_id@domain, never the handle.</p>
      </Show>

      <Show when={teams()}>
        <h2>Teams</h2>
        <For each={teams()!.teams}>
          {(t) => (
            <div class="panel">
              <div class="row">
                <strong>{t.name}</strong>
                <span class="muted small">{t.members.map(who).join(", ") || "no members"}</span>
                <div class="grow" />
                <button class="link danger" onClick={() => run(() => api.orgs.deleteTeam({ ctx: ctx(), org: org(), team: t.name }), refetchTeams)}>
                  delete
                </button>
              </div>
            </div>
          )}
        </For>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.orgs.createTeam({ ctx: ctx(), org: org(), name: team() }), () => {
              setTeam("");
              refetchTeams();
            });
          }}
        >
          <input placeholder="team name" value={team()} onInput={(e) => setTeam(e.currentTarget.value)} />
          <button disabled={!team()}>Create team</button>
        </form>
      </Show>

      <Show when={creds()}>
        <h2>Mirror credentials</h2>
        <table>
          <thead>
            <tr>
              <th>Scope</th>
              <th>Purpose</th>
              <th>Method</th>
              <th>Secret</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <For each={creds()!.credentials}>
              {(c) => (
                <tr>
                  <td>{c.repo ? `${c.org}/${c.repo}` : c.org}</td>
                  <td>{c.purpose}</td>
                  <td>{c.authMethod}</td>
                  <td class="mono">{c.secretRef}</td>
                  <td>
                    <button class="link danger" onClick={() => run(() => api.mirror.removeCredential({ ctx: ctx(), credentialId: c.id }), refetchCreds)}>
                      remove
                    </button>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            const c = cred();
            run(
              () =>
                api.mirror.setCredential({
                  ctx: ctx(),
                  org: org(),
                  provider: "github",
                  purpose: c.purpose,
                  authMethod: c.authMethod,
                  host: "github.com",
                  secretRef: c.secretRef,
                  secret: c.secretRef === "stored" ? c.secret : undefined,
                }),
              () => {
                setCred({ ...c, secret: "" });
                refetchCreds();
              },
            );
          }}
        >
          <select value={cred().purpose} onChange={(e) => setCred({ ...cred(), purpose: e.currentTarget.value })}>
            <option>mirror_git_push</option>
            <option>mirror_api</option>
            <option>import_fetch</option>
            <option>webhook_inbound</option>
          </select>
          <select value={cred().authMethod} onChange={(e) => setCred({ ...cred(), authMethod: e.currentTarget.value })}>
            <option value="pat">personal access token</option>
            <option value="github_app">GitHub App</option>
          </select>
          <input placeholder="stored or env:VAR" value={cred().secretRef} onInput={(e) => setCred({ ...cred(), secretRef: e.currentTarget.value })} />
          <Show when={cred().secretRef === "stored" && cred().authMethod === "pat"}>
            <input type="password" placeholder="token" size={36} value={cred().secret} onInput={(e) => setCred({ ...cred(), secret: e.currentTarget.value })} />
          </Show>
          <Show when={cred().secretRef === "stored" && cred().authMethod === "github_app"}>
            <textarea
              placeholder='{"app_id": "…", "installation_id": "…", "private_key": "-----BEGIN RSA PRIVATE KEY-----…"}'
              rows={4}
              cols={48}
              value={cred().secret}
              onInput={(e) => setCred({ ...cred(), secret: e.currentTarget.value })}
            />
          </Show>
          <button>Set credential</button>
        </form>
        <p class="muted small">
          A stored secret is sealed with the instance key and never shown again. A GitHub App credential is the App id, the
          installation id, and the App's private key; vcsstar mints one-hour installation tokens from it.
        </p>
      </Show>

      <Show when={accounts()}>
        <h2>Service accounts</h2>
        <p class="muted small">
          A service account never signs in. It holds API tokens for integrations that do not come through LinkKeys. Grant it
          roles like any identity (member, team, repository grant) by its identity key.
        </p>
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Identity</th>
              <th>Tokens</th>
              <th>State</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <For each={accounts()!.accounts}>
              {(a) => (
                <tr>
                  <td>
                    <strong>{a.name}</strong>
                    <Show when={a.description}>
                      <div class="muted small">{a.description}</div>
                    </Show>
                  </td>
                  <td class="mono small">
                    {a.identity.userId}@{a.identity.domain}
                  </td>
                  <td>
                    {a.activeTokens}{" "}
                    <button class="link" onClick={() => setTokenFor(tokenFor() === a.name ? null : a.name)}>
                      {tokenFor() === a.name ? "hide" : "manage"}
                    </button>
                  </td>
                  <td>
                    <span class={`badge ${a.disabledAt ? "bad" : "ok"}`}>{a.disabledAt ? "disabled" : "active"}</span>
                  </td>
                  <td>
                    <Show
                      when={a.disabledAt}
                      fallback={
                        <button class="link danger" onClick={() => run(() => api.serviceAccounts.disable({ ctx: ctx(), org: org(), name: a.name }), refetchAccounts)}>
                          disable
                        </button>
                      }
                    >
                      <button class="link" onClick={() => run(() => api.serviceAccounts.enable({ ctx: ctx(), org: org(), name: a.name }), refetchAccounts)}>
                        enable
                      </button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <Show when={tokenFor()}>
          {(name) => (
            <div class="panel">
              <strong>Tokens of {name()}</strong>
              <table>
                <tbody>
                  <For each={tokens()?.tokens ?? []}>
                    {(t) => (
                      <tr>
                        <td>{t.name}</td>
                        <td class="small">{t.scopes.join(", ")}</td>
                        <td class="muted">{when(t.createdAt)}</td>
                        <td class="muted">{t.expiresAt ? when(t.expiresAt) : "never"}</td>
                        <td>
                          <button
                            class="link danger"
                            onClick={() =>
                              run(() => api.serviceAccounts.revokeToken({ ctx: ctx(), org: org(), name: name(), tokenId: t.id }), () => {
                                refetchTokens();
                                refetchAccounts();
                              })
                            }
                          >
                            revoke
                          </button>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
              <form
                class="inline"
                onSubmit={(e) => {
                  e.preventDefault();
                  run(
                    async () => {
                      const t = await api.serviceAccounts.createToken({
                        ctx: ctx(),
                        org: org(),
                        name: name(),
                        tokenName: newToken().name,
                        scopes: newToken().scopes.split(",").map((s) => s.trim()).filter(Boolean),
                      });
                      setMinted({ account: name(), secret: t.secret ?? "" });
                    },
                    () => {
                      setNewToken({ ...newToken(), name: "" });
                      refetchTokens();
                      refetchAccounts();
                    },
                  );
                }}
              >
                <input placeholder="token name" value={newToken().name} onInput={(e) => setNewToken({ ...newToken(), name: e.currentTarget.value })} />
                <input size={40} value={newToken().scopes} onInput={(e) => setNewToken({ ...newToken(), scopes: e.currentTarget.value })} />
                <button disabled={!newToken().name}>Create token</button>
              </form>
              <Show when={minted()?.account === name()}>
                <div class="panel">
                  <strong>Copy this token now; it is not shown again.</strong>
                  <pre class="code">{minted()!.secret}</pre>
                  <p class="muted small">
                    Send it as <code>Authorization: Bearer</code> on <code>/csil/v1/rpc</code>, or <code>vcsstar login {location.origin} --token-stdin</code>.
                  </p>
                </div>
              </Show>
            </div>
          )}
        </Show>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.serviceAccounts.create({ ctx: ctx(), org: org(), name: newAccount().name, description: newAccount().description || undefined }), () => {
              setNewAccount({ name: "", description: "" });
              refetchAccounts();
            });
          }}
        >
          <input placeholder="account name" value={newAccount().name} onInput={(e) => setNewAccount({ ...newAccount(), name: e.currentTarget.value })} />
          <input placeholder="description" size={40} value={newAccount().description} onInput={(e) => setNewAccount({ ...newAccount(), description: e.currentTarget.value })} />
          <button disabled={!newAccount().name}>Create service account</button>
        </form>
      </Show>

      <HooksPanel org={org()} />
      <p class="muted small">Created {when(info()?.createdAt)}</p>
    </div>
  );
}
