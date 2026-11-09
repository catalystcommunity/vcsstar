import { createEffect, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { A, useParams } from "@solidjs/router";
import { api, ctx, errorText, short, when, who } from "../api";
import { live } from "../live";
import { session } from "../App";
import { DiffView } from "./Repo";
import type { Comment, Review } from "../generated/types.gen.ts";

export function PullPage() {
  const params = useParams();
  const org = () => params.org!;
  const name = () => params.repo!;
  const number = () => Number(params.number);
  const ref = () => ({ org: org(), name: name() });
  const key = () => `${org()}/${name()}#${number()}`;
  const [pr, { refetch }] = createResource(key, () => api.pullRequests.get({ ctx: ctx(), repo: ref(), number: number() }));
  const [reviews, { refetch: refetchReviews }] = createResource(key, () => api.pullRequests.listReviews({ ctx: ctx(), repo: ref(), number: number() }));
  const [comments, { refetch: refetchComments }] = createResource(key, () => api.pullRequests.listComments({ ctx: ctx(), repo: ref(), number: number() }));
  const [statuses] = createResource(
    () => pr()?.headSha,
    (sha) => api.statuses.list({ ctx: ctx(), repo: ref(), sha }).catch(() => null),
  );
  const [diff] = createResource(
    () => (pr() ? `${pr()!.baseSha}..${pr()!.headSha}` : null),
    () => api.repos.diff({ ctx: ctx(), repo: ref(), base: pr()!.mergeBaseSha ?? pr()!.baseSha, head: pr()!.headSha }).catch(() => null),
  );
  const [commits] = createResource(
    () => pr()?.headSha,
    (sha) => api.repos.log({ ctx: ctx(), repo: ref(), rev: sha, page: { limit: 50 } }).catch(() => null),
  );
  const [tab, setTab] = createSignal("conversation");
  const [err, setErr] = createSignal("");
  const [comment, setComment] = createSignal("");
  const [reviewState, setReviewState] = createSignal("approved");
  const [mergeKind, setMergeKind] = createSignal("merge");
  const [mergeMsg, setMergeMsg] = createSignal("");

  createEffect(() => {
    const off = live.subscribe([`${org()}/${name()}`], (e) => {
      if (e.subject === String(number()) || e.kind === "push" || e.kind === "status_set") {
        refetch();
        refetchReviews();
        refetchComments();
      }
    });
    onCleanup(off);
  });

  const run = async (f: () => Promise<unknown>) => {
    setErr("");
    try {
      await f();
      refetch();
      refetchReviews();
      refetchComments();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  type Item = { at: number } & ({ kind: "review"; r: Review } | { kind: "comment"; c: Comment });
  const timeline = (): Item[] =>
    [
      ...(reviews()?.reviews ?? []).map((r): Item => ({ at: r.createdAt, kind: "review", r })),
      ...(comments()?.comments ?? []).map((c): Item => ({ at: c.createdAt, kind: "comment", c })),
    ].sort((a, b) => a.at - b.at);
  const newCommits = () => {
    const base = pr()?.mergeBaseSha ?? pr()?.baseSha;
    const list = commits()?.commits ?? [];
    const idx = list.findIndex((c) => c.id === base);
    return idx >= 0 ? list.slice(0, idx) : list;
  };

  return (
    <div>
      <p class="muted small">
        <A href={`/${org()}`}>{org()}</A> / <A href={`/${org()}/${name()}`}>{name()}</A> / <A href={`/${org()}/${name()}/pulls`}>pulls</A>
      </p>
      <Show when={pr.error}>
        <p class="err">{errorText(pr.error)}</p>
      </Show>
      <Show when={pr()}>
        {(p) => (
          <div>
            <h1>
              {p().title} <span class="muted">#{p().number}</span>
            </h1>
            <p class="row">
              <span class={`badge ${p().state === "merged" ? "ok" : p().state === "closed" ? "bad" : ""}`}>{p().state}</span>
              <span>
                {who(p().author)} wants to merge <span class="mono">{p().head.ref.replace("refs/heads/", "")}</span>
                <Show when={p().head.repo.org !== org() || p().head.repo.name !== name()}>
                  {" "}
                  from {p().head.repo.org}/{p().head.repo.name}
                </Show>{" "}
                into <span class="mono">{p().baseRef.replace("refs/heads/", "")}</span>
              </span>
              <Show when={p().state === "open"}>
                <span class={`badge ${p().mergeable === "clean" ? "ok" : p().mergeable === "conflicts" ? "bad" : "muted"}`}>{p().mergeable}</span>
              </Show>
              <span class="muted small">
                head {short(p().headSha)} · base {short(p().baseSha)} · updated {when(p().updatedAt)}
              </span>
            </p>
            <div class="tabs">
              <For each={["conversation", "commits", "files"]}>
                {(t) => (
                  <a href="#" class={tab() === t ? "active" : ""} onClick={(e) => (e.preventDefault(), setTab(t))}>
                    {t[0].toUpperCase() + t.slice(1)}
                    {t === "commits" ? ` (${newCommits().length})` : t === "files" ? ` (${diff()?.files.length ?? 0})` : ""}
                  </a>
                )}
              </For>
            </div>
            <Show when={err()}>
              <p class="err">{err()}</p>
            </Show>

            <Show when={tab() === "conversation"}>
              <Show when={p().body}>
                <div class="comment">
                  <header>
                    {who(p().author)} opened {when(p().createdAt)}
                  </header>
                  <div class="body">{p().body}</div>
                </div>
              </Show>
              <For each={timeline()}>
                {(item) =>
                  item.kind === "review" ? (
                    <div class="comment">
                      <header>
                        {who(item.r.reviewer)}{" "}
                        <span class={`badge ${item.r.state === "approved" ? "ok" : item.r.state === "changes_requested" ? "bad" : "muted"}`}>{item.r.state}</span> at{" "}
                        <span class="mono">{short(item.r.headSha)}</span>
                        {item.r.headSha !== p().headSha ? <span class="badge warn">stale</span> : null} · {when(item.at)}
                      </header>
                      <Show when={item.r.body}>
                        <div class="body">{item.r.body}</div>
                      </Show>
                    </div>
                  ) : (
                    <div class="comment">
                      <header>
                        {who(item.c.author)} · {when(item.at)}
                        <Show when={item.c.path}>
                          {" "}
                          on <span class="mono">
                            {item.c.path}
                            {item.c.line ? `:${item.c.line}` : ""}
                          </span>
                        </Show>
                      </header>
                      <div class="body">{item.c.body}</div>
                    </div>
                  )
                }
              </For>

              <Show when={statuses() && statuses()!.statuses.length}>
                <div class="panel">
                  <strong>Checks on {short(p().headSha)}</strong>
                  <For each={statuses()!.statuses}>
                    {(s) => (
                      <div class="row small">
                        <span class={`badge ${s.state === "success" ? "ok" : s.state === "pending" ? "warn" : "bad"}`}>{s.state}</span>
                        <span>{s.context}</span>
                        <Show when={s.targetUrl}>
                          <a href={s.targetUrl} target="_blank" rel="noreferrer">
                            details
                          </a>
                        </Show>
                        <span class="muted">{s.description}</span>
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              <Show when={session()}>
                <div class="panel">
                  <textarea placeholder="Leave a comment or review" value={comment()} onInput={(e) => setComment(e.currentTarget.value)} />
                  <div class="row">
                    <button
                      disabled={!comment()}
                      onClick={() =>
                        run(async () => {
                          await api.pullRequests.comment({ ctx: ctx(), repo: ref(), number: number(), body: comment() });
                          setComment("");
                        })
                      }
                    >
                      Comment
                    </button>
                    <Show when={p().state === "open"}>
                      <select value={reviewState()} onChange={(e) => setReviewState(e.currentTarget.value)}>
                        <option value="approved">approve</option>
                        <option value="changes_requested">request changes</option>
                        <option value="comment">review comment</option>
                      </select>
                      <button
                        onClick={() =>
                          run(async () => {
                            await api.pullRequests.review({ ctx: ctx(), repo: ref(), number: number(), state: reviewState(), headSha: p().headSha, body: comment() || undefined });
                            setComment("");
                          })
                        }
                      >
                        Submit review on {short(p().headSha)}
                      </button>
                    </Show>
                  </div>
                </div>
                <Show when={p().state === "open"}>
                  <div class="panel">
                    <h3>Merge</h3>
                    <div class="row">
                      <select value={mergeKind()} onChange={(e) => setMergeKind(e.currentTarget.value)}>
                        <option value="merge">merge commit</option>
                        <option value="squash">squash</option>
                        <option value="rebase">rebase</option>
                      </select>
                      <input size={40} placeholder="message (optional)" value={mergeMsg()} onInput={(e) => setMergeMsg(e.currentTarget.value)} />
                      <button
                        class="primary"
                        disabled={p().mergeable === "conflicts"}
                        onClick={() => run(() => api.pullRequests.merge({ ctx: ctx(), repo: ref(), number: number(), expectedHeadSha: p().headSha, mergeKind: mergeKind(), message: mergeMsg() || undefined }))}
                      >
                        Merge {short(p().headSha)}
                      </button>
                      <button class="danger" onClick={() => run(() => api.pullRequests.close({ ctx: ctx(), repo: ref(), number: number() }))}>
                        Close
                      </button>
                    </div>
                    <p class="muted small">The merge is refused when the head moved since you saw it, or when a protection gate is not met; the reason is shown here.</p>
                  </div>
                </Show>
                <Show when={p().state === "closed"}>
                  <button onClick={() => run(() => api.pullRequests.reopen({ ctx: ctx(), repo: ref(), number: number() }))}>Reopen</button>
                </Show>
              </Show>
              <Show when={p().state === "merged"}>
                <p class="okmsg">
                  Merged as <A href={`/${org()}/${name()}/commits/${p().mergedCommit}`} class="mono">{short(p().mergedCommit)}</A> ({p().mergeKind}) by {who(p().mergedBy)}
                </p>
              </Show>
            </Show>

            <Show when={tab() === "commits"}>
              <table>
                <tbody>
                  <For each={newCommits()}>
                    {(c) => (
                      <tr>
                        <td>
                          <A href={`/${org()}/${name()}/commits/${c.id}`}>{c.message.split("\n")[0]}</A>
                          <div class="muted small">
                            {c.authorName} · {when(c.committedAt)} · <span class="mono">{short(c.id)}</span>
                          </div>
                        </td>
                        <td>
                          <span class={`badge ${c.signature.status === "verified" ? "ok" : c.signature.status === "unverified" ? "bad" : "muted"}`}>{c.signature.status}</span>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>

            <Show when={tab() === "files"}>
              <Show when={diff()} fallback={<p class="muted">Loading diff…</p>}>
                {(d) => <DiffView files={d().files} />}
              </Show>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
