import { createResource, createSignal, For, Show } from "solid-js";
import { A } from "@solidjs/router";
import { api, ctx, errorText, when } from "../api";
import { session } from "../App";

export function Home() {
  const [repos, { refetch }] = createResource(() => api.repos.list({ ctx: ctx(), page: { limit: 200 } }));
  const [orgs, { refetch: refetchOrgs }] = createResource(() => api.orgs.list({ ctx: ctx(), page: { limit: 200 } }));
  const [err, setErr] = createSignal("");
  const [newOrg, setNewOrg] = createSignal("");
  const [newRepo, setNewRepo] = createSignal({ org: "", name: "", visibility: "org" });

  async function createOrg(e: SubmitEvent) {
    e.preventDefault();
    setErr("");
    try {
      await api.orgs.create({ ctx: ctx(), name: newOrg() });
      setNewOrg("");
      refetchOrgs();
    } catch (ex) {
      setErr(errorText(ex));
    }
  }
  async function createRepo(e: SubmitEvent) {
    e.preventDefault();
    setErr("");
    try {
      const r = newRepo();
      await api.repos.create({ ctx: ctx(), org: r.org, name: r.name, visibility: r.visibility });
      setNewRepo({ ...r, name: "" });
      refetch();
    } catch (ex) {
      setErr(errorText(ex));
    }
  }

  return (
    <div>
      <h1>Repositories</h1>
      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>
      <Show when={repos.error}>
        <p class="err">{errorText(repos.error)}</p>
      </Show>
      <table>
        <thead>
          <tr>
            <th>Repository</th>
            <th>Visibility</th>
            <th>Default branch</th>
            <th>Mirror</th>
            <th>Created</th>
          </tr>
        </thead>
        <tbody>
          <For each={repos()?.repos ?? []}>
            {(r) => (
              <tr>
                <td>
                  <A href={`/${r.org}/${r.name}`}>
                    {r.org}/{r.name}
                  </A>
                  <Show when={r.description}>
                    <div class="muted small">{r.description}</div>
                  </Show>
                </td>
                <td>{r.visibility}</td>
                <td class="mono">{r.defaultBranch}</td>
                <td>{r.mirrorEnabled ? "on" : "off"}</td>
                <td class="muted">{when(r.createdAt)}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
      <Show when={repos() && repos()!.repos.length === 0}>
        <p class="muted">No repositories you can see yet.</p>
      </Show>

      <h2>Organizations</h2>
      <div class="row">
        <For each={orgs()?.orgs ?? []}>
          {(o) => (
            <A href={`/${o.name}`} class="btn">
              {o.displayName ?? o.name}
            </A>
          )}
        </For>
      </div>

      <Show when={session()}>
        <div class="grid2">
          <div class="panel">
            <h3>New repository</h3>
            <form class="inline" onSubmit={createRepo}>
              <select value={newRepo().org} onChange={(e) => setNewRepo({ ...newRepo(), org: e.currentTarget.value })}>
                <option value="">organization…</option>
                <For each={orgs()?.orgs ?? []}>{(o) => <option value={o.name}>{o.name}</option>}</For>
              </select>
              <input placeholder="name" value={newRepo().name} onInput={(e) => setNewRepo({ ...newRepo(), name: e.currentTarget.value })} />
              <select value={newRepo().visibility} onChange={(e) => setNewRepo({ ...newRepo(), visibility: e.currentTarget.value })}>
                <option value="public">public</option>
                <option value="org">org</option>
                <option value="private">private</option>
              </select>
              <button class="primary" disabled={!newRepo().org || !newRepo().name}>
                Create
              </button>
            </form>
          </div>
          <div class="panel">
            <h3>New organization</h3>
            <form class="inline" onSubmit={createOrg}>
              <input placeholder="name" value={newOrg()} onInput={(e) => setNewOrg(e.currentTarget.value)} />
              <button class="primary" disabled={!newOrg()}>
                Create
              </button>
            </form>
            <p class="muted small">Needs instance admin rights unless open creation is on.</p>
          </div>
        </div>
      </Show>
    </div>
  );
}
