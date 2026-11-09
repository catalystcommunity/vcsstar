import { createResource, createSignal, For, Show } from "solid-js";
import { A, useParams } from "@solidjs/router";
import { api, ctx, errorText, who, when } from "../api";
import { HooksPanel } from "./Repo";

export function OrgPage() {
  const params = useParams();
  const org = () => params.org!;
  const [info] = createResource(org, (o) => api.orgs.get({ ctx: ctx(), org: o }));
  const [repos] = createResource(org, (o) => api.repos.list({ ctx: ctx(), org: o, page: { limit: 200 } }));
  const [members, { refetch: refetchMembers }] = createResource(org, (o) => api.orgs.listMembers({ ctx: ctx(), org: o, page: { limit: 200 } }).catch(() => null));
  const [teams, { refetch: refetchTeams }] = createResource(org, (o) => api.orgs.listTeams({ ctx: ctx(), org: o }).catch(() => null));
  const [creds, { refetch: refetchCreds }] = createResource(org, (o) => api.mirror.listCredentials({ ctx: ctx(), target: { org: o } }).catch(() => null));
  const [err, setErr] = createSignal("");
  const [member, setMember] = createSignal({ identity: "", role: "member" });
  const [team, setTeam] = createSignal("");
  const [cred, setCred] = createSignal({ purpose: "mirror_git_push", secretRef: "stored", secret: "" });

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
                  authMethod: "pat",
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
          <input placeholder="stored or env:VAR" value={cred().secretRef} onInput={(e) => setCred({ ...cred(), secretRef: e.currentTarget.value })} />
          <Show when={cred().secretRef === "stored"}>
            <input type="password" placeholder="token" size={36} value={cred().secret} onInput={(e) => setCred({ ...cred(), secret: e.currentTarget.value })} />
          </Show>
          <button>Set credential</button>
        </form>
        <p class="muted small">A stored token is sealed with the instance key and never shown again.</p>
      </Show>

      <HooksPanel org={org()} />
      <p class="muted small">Created {when(info()?.createdAt)}</p>
    </div>
  );
}
