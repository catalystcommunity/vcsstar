import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show, Switch, Match } from "solid-js";
import { A, useParams, useNavigate } from "@solidjs/router";
import { api, bytesToText, ctx, errorText, short, when, who } from "../api";
import { live } from "../live";
import { session } from "../App";
import type { BlobResponse, BranchProtection, Commit, Repo, TreeResponse } from "../generated/types.gen.ts";

function sigBadge(status: string) {
  const cls = status === "verified" ? "ok" : status === "valid_unattested" ? "muted" : status === "missing" ? "muted" : "bad";
  return <span class={`badge ${cls}`}>{status}</span>;
}

export function RepoPage() {
  const params = useParams();
  const org = () => params.org!;
  const name = () => params.repo!;
  const rest = () => params.rest ?? "";
  const tab = () => rest().split("/")[0] || "code";
  const base = () => `/${org()}/${name()}`;
  const [repo, { refetch }] = createResource(() => `${org()}/${name()}`, () => api.repos.get({ ctx: ctx(), repo: { org: org(), name: name() } }));
  const [version, setVersion] = createSignal(0);

  createEffect(() => {
    const lane = `${org()}/${name()}`;
    const off = live.subscribe([lane], () => {
      setVersion((v) => v + 1);
      refetch();
    });
    onCleanup(off);
  });

  const tabs = ["code", "commits", "pulls", "mirror", "hooks", "settings"] as const;
  return (
    <div>
      <h1>
        <A href={`/${org()}`}>{org()}</A> / {name()}
        <Show when={repo()}>
          {(r) => (
            <span class="muted small">
              {" "}
              {r().visibility}
              {r().archived ? " · archived" : ""}
              {r().forkOf ? ` · fork of ${r().forkOf!.org}/${r().forkOf!.name}` : ""}
            </span>
          )}
        </Show>
      </h1>
      <Show when={repo.error}>
        <p class="err">{errorText(repo.error)}</p>
      </Show>
      <Show when={repo()}>
        {(r) => (
          <>
            <p class="muted">{r().description}</p>
            <div class="row small mono">
              <span>clone: {r().httpsUrl}</span>
              <Show when={r().sshUrl}>
                <span>{r().sshUrl}</span>
              </Show>
            </div>
          </>
        )}
      </Show>
      <div class="tabs">
        <For each={tabs}>
          {(t) => (
            <A href={t === "code" ? base() : `${base()}/${t}`} class={tab() === t ? "active" : ""}>
              {t[0].toUpperCase() + t.slice(1)}
            </A>
          )}
        </For>
      </div>
      <Show when={repo()}>
        {(r) => (
          <Switch>
            <Match when={tab() === "code"}>
              <CodeTab org={org()} name={name()} repo={r()} rest={rest()} version={version()} />
            </Match>
            <Match when={tab() === "commits"}>
              <CommitsTab org={org()} name={name()} repo={r()} rest={rest()} version={version()} />
            </Match>
            <Match when={tab() === "pulls"}>
              <PullsTab org={org()} name={name()} repo={r()} rest={rest()} version={version()} />
            </Match>
            <Match when={tab() === "mirror"}>
              <MirrorTab org={org()} name={name()} version={version()} />
            </Match>
            <Match when={tab() === "hooks"}>
              <HooksPanel org={org()} repo={name()} />
            </Match>
            <Match when={tab() === "settings"}>
              <SettingsTab org={org()} name={name()} repo={r()} onChange={refetch} />
            </Match>
          </Switch>
        )}
      </Show>
    </div>
  );
}

type TabProps = { org: string; name: string; repo: Repo; rest: string; version: number };

function CodeTab(p: TabProps) {
  // rest = "code" | "code/<ref>" | "code/<ref>/<path...>"
  const parts = () => p.rest.split("/").slice(1);
  const refName = () => decodeURIComponent(parts()[0] || p.repo.defaultBranch);
  const path = () => parts().slice(1).join("/");
  const ref = () => ({ org: p.org, name: p.name });
  const [refs] = createResource(() => p.version, () => api.repos.listRefs({ ctx: ctx(), repo: ref() }));
  const [tree] = createResource(
    () => [refName(), path(), p.version] as const,
    ([r, pth]) => api.repos.tree({ ctx: ctx(), repo: ref(), rev: r, path: pth || undefined }).catch((e) => ({ error: e })),
  );
  const [blob] = createResource(
    () => [refName(), path(), p.version] as const,
    async ([r, pth]) => {
      if (!pth) return null;
      const t = tree();
      if (t && !("error" in t)) return null;
      return api.repos.blob({ ctx: ctx(), repo: ref(), rev: r, path: pth, limit: 512 * 1024 }).catch((e) => ({ error: e }));
    },
  );
  const navigate = useNavigate();
  const treeOk = () => {
    const t = tree();
    return t && !("error" in t) ? (t as TreeResponse) : undefined;
  };
  const blobOk = () => {
    const b = blob();
    return b && !("error" in b) ? (b as BlobResponse) : undefined;
  };
  const crumbs = () => {
    const segs = path() ? path().split("/") : [];
    return segs.map((s, i) => ({ label: s, href: `/${p.org}/${p.name}/code/${encodeURIComponent(refName())}/${segs.slice(0, i + 1).join("/")}` }));
  };
  return (
    <div>
      <div class="row">
        <select
          value={refName()}
          onChange={(e) => navigate(`/${p.org}/${p.name}/code/${encodeURIComponent(e.currentTarget.value)}`)}
        >
          <For each={(refs()?.refs ?? []).filter((r) => r.name.startsWith("refs/heads/") || r.name.startsWith("refs/tags/"))}>
            {(r) => (
              <option value={r.name.replace(/^refs\/(heads|tags)\//, "")} selected={r.name.replace(/^refs\/(heads|tags)\//, "") === refName()}>
                {r.name.startsWith("refs/tags/") ? "tag " : ""}
                {r.name.replace(/^refs\/(heads|tags)\//, "")}
              </option>
            )}
          </For>
        </select>
        <span class="muted small">{refs()?.refs.length ?? 0} refs</span>
      </div>
      <div class="crumbs">
        <A href={`/${p.org}/${p.name}/code/${encodeURIComponent(refName())}`}>{p.name}</A>
        <For each={crumbs()}>
          {(c) => (
            <>
              / <A href={c.href}>{c.label}</A>
            </>
          )}
        </For>
      </div>
      <Show when={treeOk()}>
        <div class="panel tree">
          <Show when={path()}>
            <A href={`/${p.org}/${p.name}/code/${encodeURIComponent(refName())}/${path().split("/").slice(0, -1).join("/")}`}>..</A>
          </Show>
          <For each={treeOk()!.entries}>
            {(e) => (
              <A href={`/${p.org}/${p.name}/code/${encodeURIComponent(refName())}/${path() ? path() + "/" : ""}${e.name}`}>
                {e.kind === "tree" ? "📁 " : "📄 "}
                {e.name}
                <Show when={e.kind === "blob"}>
                  <span class="muted small"> {e.size} B</span>
                </Show>
              </A>
            )}
          </For>
        </div>
      </Show>
      <Show when={blobOk()}>
        {(b) => {
          const bb = b();
          const text = bb.binary ? null : bytesToText(bb.content);
          return (
            <div>
              <p class="muted small">
                {bb.size} bytes{bb.truncated ? " (truncated)" : ""}
              </p>
              <Show when={text !== null} fallback={<p class="muted">Binary file.</p>}>
                <pre class="code">{text}</pre>
              </Show>
            </div>
          );
        }}
      </Show>
      <Show when={!treeOk() && !blobOk() && blob() && "error" in blob()!}>
        <p class="err">{errorText((blob() as { error: unknown }).error)}</p>
      </Show>
      <Show when={refs() && refs()!.refs.length === 0}>
        <p class="muted">
          Empty repository. Push with <code class="mono">git push {p.repo.httpsUrl} main</code>.
        </p>
      </Show>
    </div>
  );
}

function CommitsTab(p: TabProps) {
  const parts = () => p.rest.split("/").slice(1);
  const sel = () => parts()[0] || "";
  const ref = () => ({ org: p.org, name: p.name });
  const [log] = createResource(
    () => [sel(), p.version] as const,
    ([s]) => api.repos.log({ ctx: ctx(), repo: ref(), rev: s || p.repo.defaultBranch, page: { limit: 50 } }).catch((e) => ({ error: e })),
  );
  const [detail] = createResource(
    () => [sel(), p.version] as const,
    async ([s]) => (s ? api.repos.getCommit({ ctx: ctx(), repo: ref(), rev: s }) : null),
  );
  const [diff] = createResource(
    () => detail(),
    async (c) => (c && c.parents.length ? api.repos.diff({ ctx: ctx(), repo: ref(), base: c.parents[0], head: c.id }) : null),
  );
  return (
    <div class="grid2">
      <div>
        <Show when={log() && "error" in log()!}>
          <p class="err">{errorText((log() as { error: unknown }).error)}</p>
        </Show>
        <table>
          <tbody>
            <For each={(log() as { commits?: Commit[] })?.commits ?? []}>
              {(c) => (
                <tr>
                  <td>
                    <A href={`/${p.org}/${p.name}/commits/${c.id}`}>{c.message.split("\n")[0]}</A>
                    <div class="muted small">
                      {c.authorName} · {when(c.committedAt)} · <span class="mono">{short(c.id)}</span>
                    </div>
                  </td>
                  <td>{sigBadge(c.signature.status)}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
      <div>
        <Show when={detail()}>
          {(c) => (
            <div class="panel">
              <h3>{c().message.split("\n")[0]}</h3>
              <pre class="code">{c().message}</pre>
              <p class="small">
                author {c().authorName} &lt;{c().authorEmail}&gt; · committer {c().committerName} · {new Date(c().committedAt).toISOString()}
              </p>
              <p class="small">
                signature {sigBadge(c().signature.status)}
                <Show when={c().signature.signer}> by {who(c().signature.signer)}</Show>
                <Show when={c().signature.keyFingerprint}>
                  <span class="mono"> {c().signature.keyFingerprint}</span>
                </Show>
              </p>
              <p class="small mono">
                {c().id}
                <For each={c().parents}>{(pr) => <div>parent {pr}</div>}</For>
              </p>
              <Show when={diff()}>{(d) => <DiffView files={d().files} />}</Show>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}

export function DiffView(p: { files: { status: string; oldPath?: string; newPath?: string; additions: number; deletions: number; patch?: string }[] }) {
  return (
    <div>
      <For each={p.files}>
        {(f) => (
          <details open={p.files.length <= 6}>
            <summary>
              <span class="badge muted">{f.status}</span> {f.newPath ?? f.oldPath}{" "}
              <span class="small" style="color:var(--ok)">
                +{f.additions}
              </span>{" "}
              <span class="small" style="color:var(--bad)">
                −{f.deletions}
              </span>
            </summary>
            <Show when={f.patch} fallback={<p class="muted small">No text diff.</p>}>
              <pre class="diff">
                <For each={f.patch!.split("\n")}>
                  {(line) => (
                    <span class={line.startsWith("+") && !line.startsWith("+++") ? "add" : line.startsWith("-") && !line.startsWith("---") ? "del" : line.startsWith("@@") ? "hunk" : ""}>
                      {line || " "}
                    </span>
                  )}
                </For>
              </pre>
            </Show>
          </details>
        )}
      </For>
    </div>
  );
}

function PullsTab(p: TabProps) {
  const [state, setState] = createSignal("open");
  const ref = () => ({ org: p.org, name: p.name });
  const [prs, { refetch }] = createResource(
    () => [state(), p.version] as const,
    ([s]) => api.pullRequests.list({ ctx: ctx(), repo: ref(), state: s, page: { limit: 50 } }),
  );
  const [refs] = createResource(() => api.repos.listRefs({ ctx: ctx(), repo: ref(), prefix: "refs/heads/" }));
  const [form, setForm] = createSignal({ title: "", body: "", head: "", base: p.repo.defaultBranch, show: false });
  const [err, setErr] = createSignal("");
  const navigate = useNavigate();
  return (
    <div>
      <div class="row">
        <select value={state()} onChange={(e) => setState(e.currentTarget.value)}>
          <option value="open">open</option>
          <option value="merged">merged</option>
          <option value="closed">closed</option>
          <option value="all">all</option>
        </select>
        <div class="grow" />
        <Show when={session()}>
          <button class="primary" onClick={() => setForm({ ...form(), show: !form().show })}>
            New pull request
          </button>
        </Show>
      </div>
      <Show when={form().show}>
        <form
          class="panel"
          onSubmit={async (e) => {
            e.preventDefault();
            setErr("");
            try {
              const pr = await api.pullRequests.open({
                ctx: ctx(),
                repo: ref(),
                title: form().title,
                body: form().body || undefined,
                baseRef: form().base,
                head: { repo: ref(), ref: form().head },
              });
              navigate(`/${p.org}/${p.name}/pulls/${pr.number}`);
            } catch (ex) {
              setErr(errorText(ex));
            }
          }}
        >
          <div class="row">
            <label>
              head{" "}
              <select value={form().head} onChange={(e) => setForm({ ...form(), head: e.currentTarget.value })}>
                <option value="">branch…</option>
                <For each={refs()?.refs ?? []}>{(r) => <option value={r.name.replace("refs/heads/", "")}>{r.name.replace("refs/heads/", "")}</option>}</For>
              </select>
            </label>
            <label>
              base{" "}
              <select value={form().base} onChange={(e) => setForm({ ...form(), base: e.currentTarget.value })}>
                <For each={refs()?.refs ?? []}>{(r) => <option value={r.name.replace("refs/heads/", "")}>{r.name.replace("refs/heads/", "")}</option>}</For>
              </select>
            </label>
          </div>
          <p>
            <input placeholder="title" size={60} value={form().title} onInput={(e) => setForm({ ...form(), title: e.currentTarget.value })} />
          </p>
          <textarea placeholder="description" value={form().body} onInput={(e) => setForm({ ...form(), body: e.currentTarget.value })} />
          <p>
            <button class="primary" disabled={!form().head || !form().title}>
              Open
            </button>
          </p>
          <Show when={err()}>
            <p class="err">{err()}</p>
          </Show>
        </form>
      </Show>
      <Show when={prs.error}>
        <p class="err">{errorText(prs.error)}</p>
      </Show>
      <table>
        <tbody>
          <For each={prs()?.pullRequests ?? []}>
            {(pr) => (
              <tr>
                <td>
                  <A href={`/${p.org}/${p.name}/pulls/${pr.number}`}>
                    #{pr.number} {pr.title}
                  </A>
                  <div class="muted small">
                    {who(pr.author)} · {pr.head.ref.replace("refs/heads/", "")} → {pr.baseRef.replace("refs/heads/", "")} · {when(pr.updatedAt)}
                  </div>
                </td>
                <td>
                  <span class={`badge ${pr.state === "merged" ? "ok" : pr.state === "closed" ? "bad" : ""}`}>{pr.state}</span>{" "}
                  <Show when={pr.state === "open"}>
                    <span class={`badge ${pr.mergeable === "clean" ? "ok" : pr.mergeable === "conflicts" ? "bad" : "muted"}`}>{pr.mergeable}</span>
                  </Show>
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
      <Show when={prs() && prs()!.pullRequests.length === 0}>
        <p class="muted">No {state()} pull requests.</p>
      </Show>
      <button class="link" onClick={() => refetch()}>
        refresh
      </button>
    </div>
  );
}

function MirrorTab(p: { org: string; name: string; version: number }) {
  const ref = () => ({ org: p.org, name: p.name });
  const [status, { refetch }] = createResource(() => p.version, () => api.mirror.status({ ctx: ctx(), repo: ref() }));
  const [blocked, { refetch: refetchBlocked }] = createResource(() => p.version, () => api.mirror.listBlocked({ ctx: ctx(), repo: ref() }).catch(() => null));
  const [err, setErr] = createSignal("");
  const [msg, setMsg] = createSignal("");
  const [cfg, setCfg] = createSignal({ githubRepo: "", mode: "authoritative_vcsstar" });
  const run = async (f: () => Promise<unknown>, ok?: string) => {
    setErr("");
    setMsg("");
    try {
      await f();
      if (ok) setMsg(ok);
      refetch();
      refetchBlocked();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div>
      <Show when={status()}>
        {(s) => (
          <div class="panel">
            <div class="row">
              <strong>GitHub:</strong>
              <span class="mono">{s().githubRepo ?? "not configured"}</span>
              <span class="badge">{s().mode}</span>
              <span class={`badge ${s().paused ? "warn" : "ok"}`}>{s().paused ? "paused" : "active"}</span>
              <span class="badge muted">pending {s().pending}</span>
              <span class={`badge ${s().blocked ? "bad" : "muted"}`}>blocked {s().blocked}</span>
            </div>
            <p class="small muted">
              last success {when(s().lastSuccessAt)} · last full reconcile {when(s().lastFullReconcileAt)} · pull requests synced from GitHub up to{" "}
              {s().lastPullSyncAt ? new Date(s().lastPullSyncAt!).toISOString() : "never"}
              <Show when={s().lastError}>
                <div class="err">{s().lastError}</div>
              </Show>
            </p>
            <div class="row">
              <button onClick={() => run(() => api.mirror.reconcile({ ctx: ctx(), repo: ref() }), "reconcile ran")}>Reconcile now</button>
              <Show when={s().paused} fallback={<button onClick={() => run(() => api.mirror.pause({ ctx: ctx(), repo: ref() }), "paused")}>Pause</button>}>
                <button onClick={() => run(() => api.mirror.resume({ ctx: ctx(), repo: ref() }), "resumed")}>Resume</button>
              </Show>
              <button onClick={() => run(() => api.mirror.replayBlocked({ ctx: ctx(), repo: ref(), actionIds: [] }), "replayed")} disabled={!s().blocked}>
                Replay all blocked
              </button>
            </div>
          </div>
        )}
      </Show>
      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>
      <Show when={msg()}>
        <p class="okmsg">{msg()}</p>
      </Show>
      <Show when={blocked() && blocked()!.actions.length > 0}>
        <h3>Blocked actions</h3>
        <p class="muted small">Each row holds the latest desired state for its object; fix the credential, then replay.</p>
        <table>
          <thead>
            <tr>
              <th>Object</th>
              <th>Kind</th>
              <th>Credential purpose</th>
              <th>Exact error</th>
              <th>Blocked</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <For each={blocked()!.actions}>
              {(a) => (
                <tr>
                  <td class="mono">{a.objectKey}</td>
                  <td>{a.actionKind}</td>
                  <td>{a.credentialPurpose}</td>
                  <td class="small">{a.error}</td>
                  <td class="muted">{when(a.blockedAt)}</td>
                  <td>
                    <button class="link" onClick={() => run(() => api.mirror.replayBlocked({ ctx: ctx(), repo: ref(), actionIds: [a.id] }), "replayed")}>
                      replay
                    </button>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
      <Show when={session()}>
        <h3>Configure</h3>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.mirror.configure({ ctx: ctx(), repo: ref(), githubRepo: cfg().githubRepo, mode: cfg().mode }), "configured");
          }}
        >
          <input placeholder="owner/name on GitHub" value={cfg().githubRepo} onInput={(e) => setCfg({ ...cfg(), githubRepo: e.currentTarget.value })} />
          <select value={cfg().mode} onChange={(e) => setCfg({ ...cfg(), mode: e.currentTarget.value })}>
            <option>authoritative_vcsstar</option>
            <option>import_drift</option>
            <option>flag_drift</option>
          </select>
          <button disabled={!cfg().githubRepo.includes("/")}>Save</button>
        </form>
        <p class="muted small">Credentials are set on the organization page (purpose mirror_git_push and webhook_inbound).</p>
      </Show>
    </div>
  );
}

export function HooksPanel(p: { org: string; repo?: string }) {
  const target = () => ({ org: p.org, repo: p.repo });
  const [eps, { refetch }] = createResource(() => `${p.org}/${p.repo ?? ""}`, () => api.hooks.listEndpoints({ ctx: ctx(), target: target() }).catch(() => null));
  const [selected, setSelected] = createSignal<string | null>(null);
  const [deliveries, { refetch: refetchDeliveries }] = createResource(selected, async (id) => (id ? api.hooks.listDeliveries({ ctx: ctx(), endpointId: id, page: { limit: 50 } }) : null));
  const [err, setErr] = createSignal("");
  const [form, setForm] = createSignal({ kind: "https", url: "", corndogsQueue: "", secret: "", filter: "", unordered: false });
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
    <Show when={eps()}>
      <h2>Webhooks</h2>
      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>
      <table>
        <thead>
          <tr>
            <th>Endpoint</th>
            <th>Kind</th>
            <th>Events</th>
            <th>State</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <For each={eps()!.endpoints}>
            {(e) => (
              <tr>
                <td class="mono small">
                  {e.url ?? e.corndogsQueue}
                  <Show when={e.repo}>
                    <div class="muted">repo {e.repo}</div>
                  </Show>
                </td>
                <td>{e.kind}</td>
                <td class="small">{e.eventFilter.length ? e.eventFilter.join(", ") : "all"}</td>
                <td>
                  <span class={`badge ${e.active ? "ok" : "muted"}`}>{e.active ? "active" : "off"}</span>
                  {e.unordered ? <span class="badge muted">unordered</span> : null}
                </td>
                <td class="row">
                  <button class="link" onClick={() => setSelected(selected() === e.id ? null : e.id)}>
                    deliveries
                  </button>
                  <button class="link" onClick={() => run(() => api.hooks.updateEndpoint({ ctx: ctx(), endpointId: e.id, active: !e.active }), refetch)}>
                    {e.active ? "disable" : "enable"}
                  </button>
                  <button class="link danger" onClick={() => run(() => api.hooks.deleteEndpoint({ ctx: ctx(), endpointId: e.id }), refetch)}>
                    delete
                  </button>
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
      <Show when={deliveries()}>
        {(d) => (
          <div class="panel">
            <h3>Deliveries</h3>
            <table>
              <thead>
                <tr>
                  <th>Event</th>
                  <th>State</th>
                  <th>Attempts</th>
                  <th>Last</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                <For each={d().deliveries}>
                  {(x) => (
                    <tr>
                      <td>
                        {x.eventKind} <span class="muted small mono">{x.eventLane}#{x.eventSeq}</span>
                      </td>
                      <td>
                        <span class={`badge ${x.state === "delivered" ? "ok" : x.state === "dead" ? "bad" : "warn"}`}>{x.state}</span>
                      </td>
                      <td>{x.attempts}</td>
                      <td class="small">
                        {x.lastStatus ?? ""} {x.lastError ?? ""}
                        <Show when={x.nextAttemptAt}>
                          <div class="muted">next {when(x.nextAttemptAt)}</div>
                        </Show>
                      </td>
                      <td>
                        <Show when={x.state !== "pending"}>
                          <button class="link" onClick={() => run(() => api.hooks.redeliver({ ctx: ctx(), deliveryId: x.id }), refetchDeliveries)}>
                            redeliver
                          </button>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        )}
      </Show>
      <form
        class="inline"
        onSubmit={(e) => {
          e.preventDefault();
          const f = form();
          run(
            () =>
              api.hooks.createEndpoint({
                ctx: ctx(),
                org: p.org,
                repo: p.repo,
                kind: f.kind,
                url: f.kind === "https" ? f.url : undefined,
                corndogsQueue: f.kind === "corndogs" ? f.corndogsQueue : undefined,
                secret: f.kind === "https" ? f.secret : undefined,
                eventFilter: f.filter.split(",").map((s) => s.trim()).filter(Boolean),
                unordered: f.unordered,
              }),
            () => {
              setForm({ ...f, url: "", secret: "", corndogsQueue: "" });
              refetch();
            },
          );
        }}
      >
        <select value={form().kind} onChange={(e) => setForm({ ...form(), kind: e.currentTarget.value })}>
          <option value="https">https</option>
          <option value="corndogs">corndogs</option>
        </select>
        <Show when={form().kind === "https"} fallback={<input placeholder="corndogs queue" value={form().corndogsQueue} onInput={(e) => setForm({ ...form(), corndogsQueue: e.currentTarget.value })} />}>
          <input placeholder="https://…" size={36} value={form().url} onInput={(e) => setForm({ ...form(), url: e.currentTarget.value })} />
          <input type="password" placeholder="HMAC secret (16+ chars)" value={form().secret} onInput={(e) => setForm({ ...form(), secret: e.currentTarget.value })} />
        </Show>
        <input placeholder="event filter, e.g. push,pr_merged (empty = all)" size={34} value={form().filter} onInput={(e) => setForm({ ...form(), filter: e.currentTarget.value })} />
        <label class="small">
          <input type="checkbox" checked={form().unordered} onChange={(e) => setForm({ ...form(), unordered: e.currentTarget.checked })} /> unordered
        </label>
        <button>Add endpoint</button>
      </form>
    </Show>
  );
}

function SettingsTab(p: { org: string; name: string; repo: Repo; onChange: () => void }) {
  const ref = () => ({ org: p.org, name: p.name });
  const [prots, { refetch: refetchProts }] = createResource(() => api.repos.listProtections({ ctx: ctx(), repo: ref() }));
  const [grants, { refetch: refetchGrants }] = createResource(() => api.repos.listGrants({ ctx: ctx(), repo: ref() }).catch(() => null));
  const [err, setErr] = createSignal("");
  const [desc, setDesc] = createSignal(p.repo.description ?? "");
  const [prot, setProt] = createSignal<BranchProtection>({
    refPattern: p.repo.defaultBranch,
    requiredApprovals: 1,
    dismissStale: true,
    requiredSignatureLevel: "none",
    requiredChecks: [],
    allowedMergeKinds: ["merge", "squash", "rebase"],
    allowForcePush: false,
    allowDelete: false,
  });
  const [checks, setChecks] = createSignal("");
  const [grant, setGrant] = createSignal({ granteeKind: "identity", grantee: "", role: "write" });
  const run = async (f: () => Promise<unknown>, after?: () => void) => {
    setErr("");
    try {
      await f();
      after?.();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const kinds = ["merge", "squash", "rebase"];
  return (
    <div>
      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>
      <h2>General</h2>
      <form
        class="inline"
        onSubmit={(e) => {
          e.preventDefault();
          run(() => api.repos.update({ ctx: ctx(), repo: ref(), description: desc() }), p.onChange);
        }}
      >
        <input size={60} placeholder="description" value={desc()} onInput={(e) => setDesc(e.currentTarget.value)} />
        <button>Save</button>
        <select value={p.repo.visibility} onChange={(e) => run(() => api.repos.update({ ctx: ctx(), repo: ref(), visibility: e.currentTarget.value }), p.onChange)}>
          <option>public</option>
          <option>org</option>
          <option>private</option>
        </select>
        <button type="button" onClick={() => run(() => api.repos.update({ ctx: ctx(), repo: ref(), archived: !p.repo.archived }), p.onChange)}>
          {p.repo.archived ? "Unarchive" : "Archive"}
        </button>
      </form>

      <h2>Branch protections</h2>
      <table>
        <thead>
          <tr>
            <th>Pattern</th>
            <th>Approvals</th>
            <th>Checks</th>
            <th>Signatures</th>
            <th>Merge kinds</th>
            <th>Force / delete</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <For each={prots()?.protections ?? []}>
            {(x) => (
              <tr>
                <td class="mono">{x.refPattern}</td>
                <td>
                  {x.requiredApprovals}
                  {x.dismissStale ? " (dismiss stale)" : ""}
                </td>
                <td class="small">{x.requiredChecks.join(", ") || "—"}</td>
                <td>{x.requiredSignatureLevel}</td>
                <td class="small">{x.allowedMergeKinds.join(", ") || "any"}</td>
                <td class="small">
                  {x.allowForcePush ? "force ok" : "no force"} / {x.allowDelete ? "delete ok" : "no delete"}
                </td>
                <td>
                  <button class="link danger" onClick={() => run(() => api.repos.removeProtection({ ctx: ctx(), repo: ref(), refPattern: x.refPattern }), refetchProts)}>
                    remove
                  </button>
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
      <form
        class="panel"
        onSubmit={(e) => {
          e.preventDefault();
          const pr = { ...prot(), requiredChecks: checks().split(",").map((s) => s.trim()).filter(Boolean) };
          run(() => api.repos.setProtection({ ctx: ctx(), repo: ref(), protection: pr }), refetchProts);
        }}
      >
        <div class="row">
          <input placeholder="ref pattern (main, release/*)" value={prot().refPattern} onInput={(e) => setProt({ ...prot(), refPattern: e.currentTarget.value })} />
          <label>
            approvals{" "}
            <input type="number" min="0" style="width:4rem" value={prot().requiredApprovals} onInput={(e) => setProt({ ...prot(), requiredApprovals: Number(e.currentTarget.value) })} />
          </label>
          <label>
            <input type="checkbox" checked={prot().dismissStale} onChange={(e) => setProt({ ...prot(), dismissStale: e.currentTarget.checked })} /> dismiss stale
          </label>
          <label>
            signatures{" "}
            <select value={prot().requiredSignatureLevel} onChange={(e) => setProt({ ...prot(), requiredSignatureLevel: e.currentTarget.value })}>
              <option>none</option>
              <option>any</option>
              <option>attested</option>
            </select>
          </label>
        </div>
        <div class="row">
          <input placeholder="required checks, e.g. ci/test" size={30} value={checks()} onInput={(e) => setChecks(e.currentTarget.value)} />
          <For each={kinds}>
            {(k) => (
              <label>
                <input
                  type="checkbox"
                  checked={prot().allowedMergeKinds.includes(k)}
                  onChange={(e) => setProt({ ...prot(), allowedMergeKinds: e.currentTarget.checked ? [...prot().allowedMergeKinds, k] : prot().allowedMergeKinds.filter((x) => x !== k) })}
                />{" "}
                {k}
              </label>
            )}
          </For>
          <label>
            <input type="checkbox" checked={prot().allowForcePush} onChange={(e) => setProt({ ...prot(), allowForcePush: e.currentTarget.checked })} /> allow force push
          </label>
          <label>
            <input type="checkbox" checked={prot().allowDelete} onChange={(e) => setProt({ ...prot(), allowDelete: e.currentTarget.checked })} /> allow delete
          </label>
          <button disabled={!prot().refPattern}>Save protection</button>
        </div>
      </form>

      <Show when={grants()}>
        <h2>Access</h2>
        <table>
          <tbody>
            <For each={grants()!.grants}>
              {(g) => (
                <tr>
                  <td>
                    {g.granteeKind} <span class="mono">{g.grantee}</span>
                  </td>
                  <td>{g.role}</td>
                  <td>
                    <button class="link danger" onClick={() => run(() => api.repos.removeGrant({ ctx: ctx(), repo: ref(), granteeKind: g.granteeKind, grantee: g.grantee }), refetchGrants)}>
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
            run(() => api.repos.setGrant({ ctx: ctx(), repo: ref(), grant: grant() }), () => {
              setGrant({ ...grant(), grantee: "" });
              refetchGrants();
            });
          }}
        >
          <select value={grant().granteeKind} onChange={(e) => setGrant({ ...grant(), granteeKind: e.currentTarget.value })}>
            <option value="identity">identity</option>
            <option value="team">team</option>
          </select>
          <input placeholder="user_id@domain or team" size={44} value={grant().grantee} onInput={(e) => setGrant({ ...grant(), grantee: e.currentTarget.value })} />
          <select value={grant().role} onChange={(e) => setGrant({ ...grant(), role: e.currentTarget.value })}>
            <option>read</option>
            <option>write</option>
            <option>maintain</option>
            <option>admin</option>
          </select>
          <button disabled={!grant().grantee}>Grant</button>
        </form>
      </Show>
    </div>
  );
}
