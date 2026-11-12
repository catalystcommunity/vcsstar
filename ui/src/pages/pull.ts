// One pull request: the conversation, the commits, and the files, with the
// actions a signed-in person can take.
import { markdown } from "../render";
import { api, ctx, short, when, who } from "../api";
import { button, comboBox, fill, form, h, link, external, textArea, textField, clear } from "../dom";
import { live } from "../live";
import { Page } from "../page";
import { act, badge, confirmDanger, Feedback, firstLine, loading, meta, mono, pageHead, panel, refShort, table, td, tr, type Tone } from "../ui";
import { diffView, type DiffFile } from "./diff";
import type { Comment, PullRequest, Review } from "../generated/types.gen.ts";

export class PullPage extends Page {
  private org = "";
  private name = "";
  private number = 0;
  private pr?: PullRequest;
  private readonly feedback = new Feedback();
  /** The caller's role on the repository (from `Repos/get`). */
  private repoRole?: string;
  private readonly titleText = document.createTextNode("");
  private readonly numberText = h("span", { class: "muted pr-number" });
  private readonly summary = h("div", { class: "pr-summary" });
  private readonly facts = h("div", { class: "meta" });
  private readonly conversation = h("div");
  private readonly commits = h("div");
  private readonly files = h("div");
  private readonly commitsLabel = h("tab-page", { label: "Commits" }, this.commits);
  private readonly filesLabel = h("tab-page", { label: "Files" }, this.files);
  private readonly tabs = h("tab-view", { label: "Pull request" }, h("tab-page", { label: "Conversation" }, this.conversation), this.commitsLabel, this.filesLabel) as ZestTabView;

  private get ref() { return { org: this.org, name: this.name }; }

  protected build() {
    const parts = this.context.path.split("/").filter(Boolean).map(decodeURIComponent);
    this.org = parts[0] ?? ""; this.name = parts[1] ?? ""; this.number = Number(parts[3]);
    this.title = `${this.name} #${this.number}`;
    return [
      pageHead([[this.org, `/${this.org}`], [this.name, `/${this.org}/${this.name}`], ["Pull requests", `/${this.org}/${this.name}/pulls`]], [this.titleText, " ", this.numberText], this.summary, this.facts),
      this.feedback.element,
      this.tabs,
    ];
  }

  protected connected() {
    this.own(live.subscribe([`${this.org}/${this.name}`], (e) => {
      if (e.subject === String(this.number) || e.kind === "push" || e.kind === "status_set") void this.load();
    }));
  }

  protected async start() {
    fill(this.conversation, loading());
    await this.load();
  }

  private async load() {
    try {
      const [pr, repo] = await Promise.all([
        api.pullRequests.get({ ctx: ctx(), repo: this.ref, number: this.number }),
        this.context.session && this.repoRole === undefined ? api.repos.get({ ctx: ctx(), repo: this.ref }).catch(() => null) : Promise.resolve(null),
      ]);
      if (repo) this.repoRole = repo.viewerRole;
      if (!this.isActive()) return;
      this.pr = pr;
      this.title = `${firstLine(pr.title)} #${pr.number}`;
      document.title = `${this.title} - vcsstar`;
      this.showHead(pr);
      await Promise.all([this.loadConversation(pr), this.loadCommits(pr), this.loadFiles(pr)]);
    } catch (error) {
      if (this.isActive()) { fill(this.conversation); this.feedback.error(error); }
    }
  }

  private showHead(p: PullRequest) {
    this.titleText.data = p.title;
    this.numberText.textContent = `#${p.number}`;
    const fromFork = p.head.repo.org !== this.org || p.head.repo.name !== this.name;
    fill(this.summary,
      badge(p.state, p.state === "merged" ? "ok" : p.state === "closed" ? "bad" : "info"),
      p.state === "open" && badge(p.mergeable === "clean" ? "can merge" : p.mergeable === "conflicts" ? "conflicts" : p.mergeable, p.mergeable === "clean" ? "ok" : p.mergeable === "conflicts" ? "bad" : "neutral"),
      h("span", null, h("strong", null, who(p.author)), " wants to merge ", mono(refShort(p.head.ref)), fromFork ? ` from ${p.head.repo.org}/${p.head.repo.name}` : "", " into ", mono(refShort(p.baseRef))),
    );
    fill(this.facts, h("span", null, "head ", mono(short(p.headSha))), h("span", null, "base ", mono(short(p.baseSha))), h("span", { class: "muted" }, `updated ${when(p.updatedAt)}`), p.githubUrl && external(p.githubUrl, `GitHub #${p.githubNumber}`));
  }

  // --- Conversation ---------------------------------------------------------

  private async loadConversation(p: PullRequest) {
    const [reviews, comments, statuses] = await Promise.all([
      api.pullRequests.listReviews({ ctx: ctx(), repo: this.ref, number: this.number }),
      api.pullRequests.listComments({ ctx: ctx(), repo: this.ref, number: this.number }),
      api.statuses.list({ ctx: ctx(), repo: this.ref, sha: p.headSha }).catch(() => null),
    ]);
    if (!this.isActive()) return;
    type Item = { at: number } & ({ kind: "review"; r: Review } | { kind: "comment"; c: Comment });
    const timeline: Item[] = [
      ...reviews.reviews.map((r): Item => ({ at: r.createdAt, kind: "review", r })),
      ...comments.comments.map((c): Item => ({ at: c.createdAt, kind: "comment", c })),
    ].sort((a, b) => a.at - b.at);
    const items = timeline.map((item) => item.kind === "review"
      ? this.entry([h("strong", null, who(item.r.reviewer)), badge(item.r.state.replace("_", " "), item.r.state === "approved" ? "ok" : item.r.state === "changes_requested" ? "bad" : "neutral"), h("span", null, "at ", mono(short(item.r.headSha))), item.r.headSha !== p.headSha && badge("stale", "warn"), h("span", { class: "muted" }, when(item.at))], item.r.body)
      : this.entry([h("strong", null, who(item.c.author)), h("span", { class: "muted" }, when(item.at)), item.c.path && h("span", null, "on ", mono(`${item.c.path}${item.c.line ? `:${item.c.line}` : ""}`))], item.c.body));
    fill(this.conversation,
      p.body && this.entry([h("strong", null, who(p.author)), h("span", { class: "muted" }, `opened ${when(p.createdAt)}`)], p.body),
      ...items,
      statuses && statuses.statuses.length > 0 && panel(h("h3", null, "Checks on ", mono(short(p.headSha))), h("div", { class: "stack" }, ...statuses.statuses.map((s) => h("div", { class: "row small" }, badge(s.state, s.state === "success" ? "ok" : s.state === "pending" ? "warn" : "bad"), h("span", null, s.context), s.targetUrl && external(s.targetUrl, "details"), h("span", { class: "muted" }, s.description))))),
      this.context.session && this.actions(p),
      p.state === "merged" && h("p", { class: "merged-note" }, "Merged as ", link(`/${this.org}/${this.name}/commits/${p.mergedCommit}`, mono(short(p.mergedCommit))), p.mergeKind ? ` (${p.mergeKind})` : "", p.mergedBy ? ` by ${who(p.mergedBy)}` : "", "."),
    );
  }

  private entry(head: unknown[], body?: string | null) {
    return h("article", { class: "entry" }, h("header", { class: "row small" }, ...(head as never[])), body && h("div", { class: "entry-body" }, markdown(body, this.pr && { ...this.ref, rev: this.pr.headSha, dir: "" })));
  }

  private actions(p: PullRequest) {
    const text = textArea("Comment", { rows: 4, autogrow: true, placeholder: "Say what you see" });
    const comment = button("Comment", { variant: "default", onClick: () => void act(async () => { await api.pullRequests.comment({ ctx: ctx(), repo: this.ref, number: this.number, body: text.value }); clear(text); }, { done: "Comment posted.", after: () => void this.load() }) });
    const reviewState = comboBox("Review", [{ value: "approved", label: "Approve" }, { value: "changes_requested", label: "Request changes" }, { value: "comment", label: "Comment only" }], { value: "approved" });
    const review = button(["Submit review on ", mono(short(p.headSha))], { variant: "default", onClick: () => void act(async () => { await api.pullRequests.review({ ctx: ctx(), repo: this.ref, number: this.number, state: reviewState.value, headSha: p.headSha, body: text.value || undefined }); clear(text); }, { done: "Review submitted.", after: () => void this.load() }) });
    const parts: unknown[] = [panel(text, h("div", { class: "row" }, comment, p.state === "open" && [reviewState, review]))];
    // Merging needs write access; closing and reopening also suit the author.
    const writer = ["write", "maintain", "admin"].includes(this.repoRole ?? "");
    const me = this.context.session?.identity;
    const author = Boolean(me && me.userId === p.author.userId && me.domain === p.author.domain);
    if (p.state === "open" && !writer && author) {
      parts.push(button("Close", { variant: "danger", onClick: async () => {
        if (await confirmDanger("Close this pull request?", "You can reopen it later. On GitHub it closes too.", "Close pull request")) void act(() => api.pullRequests.close({ ctx: ctx(), repo: this.ref, number: this.number }), { done: "Pull request closed.", after: () => void this.load() });
      } }));
    }
    if (p.state === "open" && writer) {
      const kind = comboBox("Merge kind", [{ value: "merge", label: "Merge commit" }, { value: "squash", label: "Squash" }, { value: "rebase", label: "Rebase" }], { value: "merge" });
      const message = textField("Message", { placeholder: "optional", class: "w-lg" });
      const merge = button(["Merge ", mono(short(p.headSha))], { type: "submit", disabled: p.mergeable === "conflicts" });
      const close = button("Close", { variant: "danger", onClick: async () => {
        if (await confirmDanger("Close this pull request?", "You can reopen it later. On GitHub it closes too.", "Close pull request")) void act(() => api.pullRequests.close({ ctx: ctx(), repo: this.ref, number: this.number }), { done: "Pull request closed.", after: () => void this.load() });
      } });
      parts.push(panel(h("h3", null, "Merge"),
        form({ class: "form-row", onSubmit: () => void act(() => api.pullRequests.merge({ ctx: ctx(), repo: this.ref, number: this.number, expectedHeadSha: p.headSha, mergeKind: kind.value, message: message.value || undefined }), { done: "Merged.", after: () => void this.load() }) }, kind, message, merge, close),
        h("p", { class: "hint" }, "The merge is refused when the head moved since you saw it, or when a protection is not met. The reason is shown as a notice."),
      ));
    }
    if (p.state === "closed" && (writer || author)) parts.push(button("Reopen", { variant: "default", onClick: () => void act(() => api.pullRequests.reopen({ ctx: ctx(), repo: this.ref, number: this.number }), { done: "Reopened.", after: () => void this.load() }) }));
    return h("div", { class: "stack" }, ...(parts as never[]));
  }

  // --- Commits and files ----------------------------------------------------

  private async loadCommits(p: PullRequest) {
    const log = await api.repos.log({ ctx: ctx(), repo: this.ref, rev: p.headSha, page: { limit: 50 } }).catch(() => null);
    if (!this.isActive()) return;
    const base = p.mergeBaseSha ?? p.baseSha;
    const all = log?.commits ?? [];
    const idx = all.findIndex((c) => c.id === base);
    const commits = idx >= 0 ? all.slice(0, idx) : all;
    this.commitsLabel.setAttribute("label", `Commits (${commits.length})`);
    const t = table(null);
    fill(t.body, ...commits.map((c) => tr(
      td("", link(`/${this.org}/${this.name}/commits/${c.id}`, firstLine(c.message)), meta(c.authorName, when(c.committedAt), mono(short(c.id)))),
      td("", badge(c.signature.status.replace("_", " "), sigTone(c.signature.status))),
    )));
    fill(this.commits, t.wrap);
  }

  private async loadFiles(p: PullRequest) {
    // Every page of the diff (50 files a page), up to 2000 files.
    const base = p.mergeBaseSha ?? p.baseSha;
    let diff: { files: DiffFile[] } | null = null;
    try {
      const files: DiffFile[] = [];
      let cursor: string | undefined;
      do {
        const page = await api.repos.diff({ ctx: ctx(), repo: this.ref, base, head: p.headSha, page: { cursor, limit: 100 } });
        files.push(...page.files);
        cursor = page.nextCursor ?? undefined;
        if (cursor) fill(this.files, h("p", { class: "hint" }, `Loading files… ${files.length}`));
      } while (cursor && files.length < 2000 && this.isActive());
      diff = { files };
    } catch { diff = null; }
    if (!this.isActive()) return;
    this.filesLabel.setAttribute("label", `Files (${diff?.files.length ?? 0})`);
    fill(this.files, diff ? diffView(diff.files, { repo: this.ref, base, head: p.headSha }) : h("p", { class: "hint" }, "The diff is not available."));
  }
}

export function sigTone(status: string): Tone {
  return status === "verified" ? "ok" : status === "valid_unattested" || status === "missing" ? "neutral" : "bad";
}
