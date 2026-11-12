// A repository: code, commits, pull requests, the mirror, webhooks, and
// settings. The tabs are routes, so the page stays while the tab changes.
import { api, bytesToText, ctx, short, when, who } from "../api";
import { button, checkBox, comboBox, fill, form, h, link, spinBox, textArea, textButton, textField, clear } from "../dom";
import { live } from "../live";
import { Page } from "../page";
import { act, badge, confirmDanger, csv, empty, Feedback, firstLine, isoDate, loading, meta, mono, pageHead, panel, refShort, section, sortKey, table, td, timeCell, tr } from "../ui";
import { diffView, mediaElement } from "./diff";
import { codeView, drawMermaid, isMarkdown, isMermaid, languageOf, markdown, mediaType, rawUrl, readAll } from "../render";
import { hooksSection } from "./hooks";
import { sigTone } from "./pull";
import type { BranchProtection, Repo } from "../generated/types.gen.ts";

const TABS = [["code", "Code"], ["commits", "Commits"], ["pulls", "Pull requests"], ["mirror", "Mirror"], ["hooks", "Webhooks"], ["settings", "Settings"]] as const;
type TabId = (typeof TABS)[number][0];
/** What an anonymous viewer gets; the other tabs need a sign-in and are not shown. */
const PUBLIC_TABS: readonly TabId[] = ["code", "commits", "pulls", "mirror"];
/** Repository roles, lowest first, as `Repo.viewerRole` names them. */
const ROLES = ["read", "write", "maintain", "admin"] as const;
type Role = (typeof ROLES)[number];
/** The role a tab needs; the others need read. */
const TAB_ROLE: Partial<Record<TabId, Role>> = { hooks: "admin", settings: "admin" };

function allows(have: string | undefined, need: Role): boolean {
  return ROLES.indexOf((have ?? "") as Role) >= ROLES.indexOf(need);
}

interface Tab {
  element: HTMLElement;
  /** The route moved inside the tab. */
  update(rest: string[]): void;
  /** Something about the repository changed (a live event). */
  refresh(): void;
}

export class RepoPage extends Page {
  private org = "";
  private name = "";
  private repo?: Repo;
  private tabId: TabId = "code";
  private rest: string[] = [];
  private tab?: Tab;
  private readonly feedback = new Feedback();
  private readonly facts = h("div", { class: "meta" });
  private readonly description = h("p", { class: "lead" });
  private readonly clone = h("div", { class: "clone" });
  private readonly tabNav = h("nav", { class: "tabs", "aria-label": "Repository" });
  private readonly body = h("div", { class: "tab-body" });

  private get ref() { return { org: this.org, name: this.name }; }
  private get base() { return `/${this.org}/${this.name}`; }

  protected build() {
    this.readPath(this.context.path);
    this.title = `${this.org}/${this.name}`;
    return [
      pageHead([[this.org, `/${this.org}`]], [h("span", { class: "repo-org" }, `${this.org} / `), this.name], this.facts, this.description, this.clone),
      this.feedback.element,
      this.tabNav,
      this.body,
    ];
  }

  private readPath(path: string) {
    const parts = path.split("/").filter(Boolean).map(decodeURIComponent);
    this.org = parts[0] ?? ""; this.name = parts[1] ?? "";
    const tab = (parts[2] ?? "code") as TabId;
    const known = this.tabs().some(([id]) => id === tab);
    this.tabId = known ? tab : "code";
    this.rest = known ? parts.slice(3) : [];
  }

  /** The tabs the caller may use: by sign-in until the repository loads,
   * then by the caller's role on it. */
  private tabs() {
    if (!this.context.session) return TABS.filter(([id]) => PUBLIC_TABS.includes(id));
    if (!this.repo) return TABS.filter(([id]) => !TAB_ROLE[id]);
    return TABS.filter(([id]) => allows(this.repo!.viewerRole, TAB_ROLE[id] ?? "read"));
  }

  setPath(path: string): boolean {
    const before = this.tabId;
    this.readPath(path);
    this.renderTabNav();
    if (!this.repo) return true;
    if (before === this.tabId && this.tab) this.tab.update(this.rest);
    else this.showTab();
    return true;
  }

  protected connected() {
    this.own(live.subscribe([`${this.org}/${this.name}`], () => { void this.loadRepo(); this.tab?.refresh(); }));
  }

  protected async start() {
    this.renderTabNav();
    fill(this.body, loading());
    await this.loadRepo();
    if (!this.repo) return;
    // The role is known now: a tab the caller may not use opens Code.
    if (!this.tabs().some(([id]) => id === this.tabId)) { this.tabId = "code"; this.rest = []; }
    this.renderTabNav();
    this.showTab();
  }

  private async loadRepo() {
    try {
      const r = await api.repos.get({ ctx: ctx(), repo: this.ref });
      if (!this.isActive()) return;
      this.repo = r;
      fill(this.facts, badge(r.visibility), r.archived && badge("archived", "warn"), r.mirrorEnabled && badge("mirrored", "info"), r.forkOf && h("span", null, "fork of ", link(`/${r.forkOf.org}/${r.forkOf.name}`, `${r.forkOf.org}/${r.forkOf.name}`)), h("span", { class: "muted" }, "default branch ", mono(r.defaultBranch)));
      this.description.textContent = r.description ?? "";
      this.description.hidden = !r.description;
      fill(this.clone, cloneField("HTTPS", r.httpsUrl), r.sshUrl && cloneField("SSH", r.sshUrl), this.context.session && h("div", { class: "row" }, this.forkButton()), this.forkBox);
    } catch (error) {
      if (this.isActive()) { fill(this.body); this.feedback.error(error); }
    }
  }

  private readonly forkBox = h("div");

  // A fork goes into an organization where the caller is an admin or
  // owner. Its pull requests can then come back here (New pull request).
  private forkButton() {
    return button("Fork", { variant: "default", onClick: async () => {
      if (this.forkBox.childElementCount) { fill(this.forkBox); return; }
      const orgs = await api.orgs.list({ ctx: ctx(), page: { limit: 200 } }).catch(() => ({ orgs: [] }));
      const mine = orgs.orgs.filter((o) => o.viewerRole === "admin" || o.viewerRole === "owner");
      if (mine.length === 0) { fill(this.forkBox, h("p", { class: "hint" }, "You are not an admin of any organization to fork into.")); return; }
      const org = comboBox("Into organization", mine.map((o) => o.name), { required: true, value: mine.find((o) => o.name !== this.org)?.name ?? mine[0]!.name });
      const name = textField("Name", { required: true, value: this.name, autocapitalize: "none", spellcheck: false });
      const feedback = new Feedback();
      fill(this.forkBox, panel(h("h3", null, "Fork ", mono(`${this.org}/${this.name}`)), feedback.element,
        form({ class: "form-row", onSubmit: () => void act(async () => {
          const r = await api.repos.fork({ ctx: ctx(), source: this.ref, org: org.value, name: name.value });
          this.context.navigate(`/${r.org}/${r.name}`);
        }, { feedback }) }, org, name, button("Fork", { type: "submit" }), textButton("Cancel", () => fill(this.forkBox)))));
    } });
  }

  private renderTabNav() {
    const tabs = this.tabs();
    fill(this.tabNav, ...tabs.map(([id, label]) => link(id === "code" ? this.base : `${this.base}/${id}`, label)).map((a, i) => { if (tabs[i]![0] === this.tabId) a.setAttribute("aria-current", "page"); return a; }));
  }

  private showTab() {
    const repo = this.repo!;
    const common = { org: this.org, name: this.name, repo, navigate: this.context.navigate, signedIn: Boolean(this.context.session), can: (need: Role) => allows(this.repo?.viewerRole, need), base: this.base, onRepoChange: () => void this.loadRepo() };
    switch (this.tabId) {
      case "code": this.tab = codeTab(common, this.rest); break;
      case "commits": this.tab = commitsTab(common, this.rest); break;
      case "pulls": this.tab = pullsTab(common); break;
      case "mirror": this.tab = mirrorTab(common); break;
      case "hooks": this.tab = { element: hooksSection(this.org, this.name), update() {}, refresh() {} }; break;
      case "settings": this.tab = settingsTab(common); break;
    }
    fill(this.body, this.tab.element);
  }
}

type Common = { org: string; name: string; repo: Repo; navigate(path: string): void; signedIn: boolean; can(need: Role): boolean; base: string; onRepoChange(): void };

function cloneField(label: string, url: string) {
  const copy = textButton("Copy", async () => { try { await navigator.clipboard.writeText(url); copy.textContent = "Copied"; setTimeout(() => (copy.textContent = "Copy"), 1500); } catch { /* the user can select the text */ } }, { "aria-label": `Copy the ${label} clone URL` });
  return h("div", { class: "clone-row" }, h("span", { class: "clone-label" }, label), mono(url, "clone-url"), copy);
}

// --- Code ------------------------------------------------------------------

function codeTab(c: Common, rest: string[]): Tab {
  const refBox = comboBox("Branch or tag", [], { "aria-label": "Branch or tag", class: "ref-box" });
  const refCount = h("span", { class: "muted small" });
  const crumbs = h("nav", { class: "crumbs code-crumbs", "aria-label": "Path" });
  const content = h("div");
  const feedback = new Feedback();
  let refName = "";
  let path = "";
  let refsLoaded = false;

  function read(parts: string[]) {
    refName = parts[0] || c.repo.defaultBranch;
    path = parts.slice(1).join("/");
  }
  const href = (p: string) => `${c.base}/code/${encodeURIComponent(refName)}${p ? `/${p}` : ""}`;

  async function loadRefs() {
    try {
      const page = await api.repos.listRefs({ ctx: ctx(), repo: { org: c.org, name: c.name } });
      const names = page.refs.filter((r) => r.name.startsWith("refs/heads/") || r.name.startsWith("refs/tags/"));
      refBox.options = names.map((r) => ({ value: refShort(r.name), label: refShort(r.name), description: r.name.startsWith("refs/tags/") ? "tag" : undefined }));
      refBox.value = refName;
      refCount.textContent = page.refs.length === 1 ? "1 ref" : `${page.refs.length} refs`;
      refsLoaded = true;
      if (page.refs.length === 0) fill(content, empty(h("span", null, "Empty repository. Push with ", mono(`git push ${c.repo.httpsUrl} main`), ".")));
    } catch (error) {
      feedback.error(error);
    }
  }

  async function load() {
    fill(crumbs, link(href(""), c.name), ...(path ? path.split("/") : []).map((seg, i, all) => [h("span", { class: "sep", "aria-hidden": "true" }, "/"), link(href(all.slice(0, i + 1).join("/")), seg)]));
    if (refsLoaded) refBox.value = refName;
    const repo = { org: c.org, name: c.name };
    try {
      const tree = await api.repos.tree({ ctx: ctx(), repo, rev: refName, path: path || undefined });
      // The directory's README, rendered under the listing.
      const readme = tree.entries.filter((e) => e.kind === "blob" && /^readme(\.(md|markdown|mdx|txt|rst))?$/i.test(e.name)).sort((x, y) => Number(isMarkdown(y.name)) - Number(isMarkdown(x.name)))[0];
      const readmeBox = h("div");
      if (readme) {
        const rpath = path ? `${path}/${readme.name}` : readme.name;
        void api.repos.blob({ ctx: ctx(), repo, rev: refName, path: rpath, limit: 1024 * 1024 }).then((b) => {
          const text = b.binary ? null : bytesToText(b.content);
          if (text === null) return;
          fill(readmeBox, panel(h("h3", { class: "mono" }, readme.name), isMarkdown(readme.name) ? markdown(text, { org: c.org, name: c.name, rev: refName, dir: path }) : h("pre", { class: "code" }, text)));
        }).catch(() => undefined);
      }
      fill(content, readmeBox);
      content.prepend(h("div", { class: "tree" },
        path && link(href(path.split("/").slice(0, -1).join("/")), h("span", { class: "tree-icon", "aria-hidden": "true" }, "↑"), ".."),
        ...tree.entries.map((e) => link(href(path ? `${path}/${e.name}` : e.name), h("span", { class: "tree-icon", "aria-hidden": "true" }, e.kind === "tree" ? "▸" : "·"), h("span", { class: e.kind === "tree" ? "tree-dir" : "" }, e.name), e.kind === "blob" && h("span", { class: "muted small size" }, formatSize(e.size ?? 0)))),
      ));
      return;
    } catch (treeError) {
      if (!path) { fill(content); feedback.error(treeError); return; }
    }
    try {
      fill(content, await fileView(repo, refName, path));
    } catch (error) {
      fill(content);
      feedback.error(error);
    }
  }

  refBox.addEventListener("change", () => c.navigate(`${c.base}/code/${encodeURIComponent(refBox.value)}`));
  read(rest);
  fill(content, loading());
  void loadRefs().then(load);
  return {
    element: h("div", null, h("div", { class: "row toolbar" }, refBox, refCount), crumbs, feedback.element, content),
    update(parts) { read(parts); void load(); },
    refresh() { void loadRefs().then(load); },
  };
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// --- Commits ---------------------------------------------------------------

function commitsTab(c: Common, rest: string[]): Tab {
  const list = table(null);
  const detail = h("div", { class: "commit-detail" });
  const feedback = new Feedback();
  let selected = rest[0] ?? "";
  const repo = { org: c.org, name: c.name };

  async function load() {
    try {
      const log = await api.repos.log({ ctx: ctx(), repo, rev: selected || c.repo.defaultBranch, page: { limit: 50 } });
      fill(list.body, ...log.commits.map((x) => {
        const row = tr(
          td("", link(`${c.base}/commits/${x.id}`, firstLine(x.message)), meta(x.authorName, when(x.committedAt), mono(short(x.id)))),
          td("", badge(x.signature.status.replace("_", " "), sigTone(x.signature.status))),
        );
        if (x.id === selected) row.setAttribute("aria-current", "true");
        return row;
      }));
    } catch (error) {
      feedback.error(error);
    }
    if (!selected) { fill(detail); return; }
    try {
      const x = await api.repos.getCommit({ ctx: ctx(), repo, rev: selected });
      const diff = x.parents.length ? await api.repos.diff({ ctx: ctx(), repo, base: x.parents[0]!, head: x.id }).catch(() => null) : null;
      fill(detail, panel(
        h("h3", null, firstLine(x.message)),
        x.message.includes("\n") && h("pre", { class: "code message" }, x.message.split("\n").slice(1).join("\n").trim()),
        meta(h("span", null, `author ${x.authorName} <${x.authorEmail}>`), h("span", null, `committer ${x.committerName}`), isoDate(x.committedAt)),
        meta(h("span", null, "signature ", badge(x.signature.status.replace("_", " "), sigTone(x.signature.status))), x.signature.signer && `by ${who(x.signature.signer)}`, x.signature.keyFingerprint && mono(x.signature.keyFingerprint)),
        meta(mono(x.id), ...x.parents.map((p) => h("span", null, "parent ", link(`${c.base}/commits/${p}`, mono(short(p)))))),
        diff && diffView(diff.files, { repo, base: x.parents[0]!, head: x.id }),
      ));
    } catch (error) {
      fill(detail);
      feedback.error(error);
    }
  }

  void load();
  return {
    element: h("div", null, feedback.element, h("div", { class: "split" }, list.wrap, detail)),
    update(parts) { selected = parts[0] ?? ""; void load(); },
    refresh() { void load(); },
  };
}

// --- Pull requests -----------------------------------------------------------

function pullsTab(c: Common): Tab {
  const repo = { org: c.org, name: c.name };
  const state = comboBox("Show", [{ value: "open", label: "Open" }, { value: "merged", label: "Merged" }, { value: "closed", label: "Closed" }, { value: "all", label: "All" }], { value: "open" });
  const list = table(["Pull request", "State", "Updated"]);
  const status = h("div");
  const feedback = new Feedback();
  const formBox = h("div");
  let formOpen = false;

  async function load() {
    try {
      const page = await api.pullRequests.list({ ctx: ctx(), repo, state: state.value, page: { limit: 50 } });
      fill(list.body, ...page.pullRequests.map((pr) => tr(
        sortKey(td("", link(`${c.base}/pulls/${pr.number}`, h("span", { class: "muted pr-number" }, `#${pr.number} `), pr.title), meta(who(pr.author), h("span", null, mono(refShort(pr.head.ref)), " into ", mono(refShort(pr.baseRef))))), pr.number),
        td("", badge(pr.state, pr.state === "merged" ? "ok" : pr.state === "closed" ? "bad" : "info"), pr.state === "open" && badge(pr.mergeable === "clean" ? "can merge" : pr.mergeable, pr.mergeable === "clean" ? "ok" : pr.mergeable === "conflicts" ? "bad" : "neutral")),
        timeCell(pr.updatedAt),
      )));
      fill(status, page.pullRequests.length === 0 && empty(`No ${state.value === "all" ? "" : state.value + " "}pull requests.`));
    } catch (error) {
      fill(status);
      feedback.error(error);
    }
  }

  // A pull request goes into a base repository (this one, or its parent
  // when this is a fork) from a head repository: the base itself or any
  // fork of it the caller can see, in any organization.
  async function openForm() {
    type Ref = { org: string; name: string };
    const key = (r: Ref) => `${r.org}/${r.name}`;
    const parse = (k: string): Ref => { const [org, name] = k.split("/"); return { org: org!, name: name! }; };
    const bases: Ref[] = [repo, ...(c.repo.forkOf ? [{ org: c.repo.forkOf.org, name: c.repo.forkOf.name }] : [])];
    const baseRepo = comboBox("Into repository", bases.map((r) => ({ value: key(r), label: key(r), description: key(r) === key(repo) ? "this repository" : "the repository this one forks" })), { value: key(c.repo.forkOf ?? repo) });
    const base = comboBox("Base branch", [], { required: true });
    const headRepo = comboBox("From repository", [], { required: true });
    const head = comboBox("Head branch", [], { required: true, placeholder: "Select" });
    const title = textField("Title", { required: true, class: "w-xl" });
    const body = textArea("Description", { rows: 5, class: "w-xl" });
    const compare = h("div");
    const formFeedback = new Feedback();
    const branchesOf = async (r: Ref) => (await api.repos.listRefs({ ctx: ctx(), repo: r, prefix: "refs/heads/" }).catch(() => ({ refs: [] }))).refs.map((x) => refShort(x.name));
    const loadBase = async () => {
      const target = parse(baseRepo.value);
      const [branches, forks, info] = await Promise.all([
        branchesOf(target),
        api.repos.listForks({ ctx: ctx(), repo: target }).catch(() => ({ repos: [] as Repo[] })),
        key(target) === key(repo) ? Promise.resolve(c.repo) : api.repos.get({ ctx: ctx(), repo: target }).catch(() => null),
      ]);
      base.options = branches;
      base.value = info?.defaultBranch ?? branches[0] ?? "";
      const heads: Ref[] = [target, ...forks.repos.map((f) => ({ org: f.org, name: f.name }))];
      headRepo.options = heads.map((r) => ({ value: key(r), label: key(r), description: key(r) === key(target) ? "same repository" : "fork" }));
      headRepo.value = heads.some((r) => key(r) === key(repo)) ? key(repo) : key(target);
      await loadHead();
    };
    const loadHead = async () => {
      const branches = await branchesOf(parse(headRepo.value));
      head.options = branches;
      if (!branches.includes(head.value)) head.value = "";
      await showCompare();
    };
    // What the pull request would bring, when head and base are in one
    // repository (a fork's objects are copied in when it opens).
    const showCompare = async () => {
      fill(compare);
      if (!head.value || !base.value || headRepo.value !== baseRepo.value || head.value === base.value) return;
      const target = parse(baseRepo.value);
      const diff = await api.repos.diff({ ctx: ctx(), repo: target, base: base.value, head: head.value }).catch(() => null);
      if (!diff) return;
      const added = diff.files.reduce((n, f) => n + f.additions, 0);
      const removed = diff.files.reduce((n, f) => n + f.deletions, 0);
      fill(compare, h("p", { class: "hint" }, `${diff.files.length} file${diff.files.length === 1 ? "" : "s"} changed, +${added} −${removed}`), diffView(diff.files, { repo: target, base: base.value, head: head.value }));
    };
    baseRepo.addEventListener("change", () => void loadBase());
    headRepo.addEventListener("change", () => void loadHead());
    head.addEventListener("change", () => void showCompare());
    base.addEventListener("change", () => void showCompare());
    fill(formBox, panel(h("h3", null, "New pull request"), formFeedback.element, form({ class: "form-stack", onSubmit: () => void act(async () => {
      const target = parse(baseRepo.value);
      const pr = await api.pullRequests.open({ ctx: ctx(), repo: target, title: title.value, body: body.value || undefined, baseRef: base.value, head: { repo: parse(headRepo.value), ref: head.value } });
      c.navigate(`/${target.org}/${target.name}/pulls/${pr.number}`);
    }, { feedback: formFeedback }) },
      h("div", { class: "form-row" }, baseRepo, base),
      h("div", { class: "form-row" }, headRepo, head),
      title, body,
      h("div", { class: "row" }, button("Open pull request", { type: "submit" }), textButton("Cancel", () => { formOpen = false; fill(formBox); })),
      compare)));
    await loadBase();
  }

  state.addEventListener("change", () => void load());
  fill(status, loading());
  void load();
  const toolbar = h("div", { class: "row toolbar" }, state, h("span", { class: "grow" }), c.signedIn && button("New pull request", { variant: "primary", onClick: () => { formOpen = !formOpen; if (formOpen) void openForm(); else fill(formBox); } }));
  return { element: h("div", null, toolbar, formBox, feedback.element, list.wrap, status), update() {}, refresh() { void load(); } };
}

// --- Mirror ------------------------------------------------------------------

function mirrorTab(c: Common): Tab {
  const repo = { org: c.org, name: c.name };
  const statusBox = h("div");
  const blockedBox = h("div");
  const feedback = new Feedback();

  async function load() {
    try {
      const s = await api.mirror.status({ ctx: ctx(), repo });
      if (c.can("admin")) { pullEvery.value = Math.max(1, Math.round(s.pullSyncIntervalS / 60)); refsEvery.value = Math.max(1, Math.round(s.refReconcileIntervalS / 60)); }
      fill(statusBox, panel(
        h("div", { class: "row" }, h("strong", null, "GitHub"), s.githubRepo ? mono(s.githubRepo) : h("span", { class: "muted" }, "not configured"), badge(s.mode.replace(/_/g, " ")), badge(s.paused ? "paused" : "active", s.paused ? "warn" : "ok"), badge(`${s.pending} pending`), badge(`${s.blocked} blocked`, s.blocked ? "bad" : "neutral")),
        meta(h("span", null, `last success ${when(s.lastSuccessAt) || "never"}`), h("span", null, `last full reconcile ${when(s.lastFullReconcileAt) || "never"}`), h("span", null, `pull requests synced from GitHub up to ${s.lastPullSyncAt ? isoDate(s.lastPullSyncAt) : "never"}`)),
        s.backfillState && meta(h("span", null, "pull request history from GitHub: ", badge(s.backfillState, s.backfillState === "done" ? "ok" : "info"), ` ${s.backfillPulls ?? 0} pull requests`, s.backfillDoneAt ? `, done ${when(s.backfillDoneAt)}` : s.backfillStartedAt ? `, started ${when(s.backfillStartedAt)}` : "")),
        s.importState && s.importState !== "done" && meta(h("span", null, "import of the branches and tags: ", badge(s.importState === "failed" ? "failed" : "queued", s.importState === "failed" ? "bad" : "info"))),
        meta(h("span", null, `pull request pass every ${duration(s.pullSyncIntervalS)}`), h("span", null, `ref reconcile every ${duration(s.refReconcileIntervalS)}`)),
        s.lastError && h("p", { class: "problem" }, s.lastError),
        c.can("maintain") && h("div", { class: "row" },
          c.can("admin") && button("Import history again", { variant: "default", disabled: !s.githubRepo || s.backfillState !== "done", onClick: () => void act(() => api.mirror.backfill({ ctx: ctx(), repo }), { done: "The pull request history import starts again from the first pull request.", after: () => void load() }) }),
          button("Reconcile now", { variant: "default", onClick: () => void act(() => api.mirror.reconcile({ ctx: ctx(), repo }), { done: "Reconcile ran.", after: () => void load() }) }),
          c.can("admin") && (s.paused
            ? button("Resume", { variant: "default", onClick: () => void act(() => api.mirror.resume({ ctx: ctx(), repo }), { done: "Mirror resumed.", after: () => void load() }) })
            : button("Pause", { variant: "default", onClick: () => void act(() => api.mirror.pause({ ctx: ctx(), repo }), { done: "Mirror paused.", after: () => void load() }) })),
          button("Replay all blocked", { variant: "default", disabled: !s.blocked, onClick: () => void act(() => api.mirror.replayBlocked({ ctx: ctx(), repo, actionIds: [] }), { done: "Blocked actions queued again.", after: () => void load() }) }),
        ),
      ));
    } catch (error) {
      fill(statusBox);
      feedback.error(error);
    }
    // The blocked list needs write access; the actions need maintain or
    // admin. Not requested for a caller without them.
    if (!c.can("write")) return;
    try {
      const b = await api.mirror.listBlocked({ ctx: ctx(), repo });
      if (b.actions.length === 0) { fill(blockedBox); return; }
      const t = table(["Object", "Kind", "Credential purpose", "Exact error", "Blocked", ""]);
      fill(t.body, ...b.actions.map((a) => tr(td("", mono(a.objectKey)), a.actionKind, a.credentialPurpose, td("small", a.error), timeCell(a.blockedAt),
        td("actions", c.can("maintain") && textButton("Replay", () => void act(() => api.mirror.replayBlocked({ ctx: ctx(), repo, actionIds: [a.id] }), { done: "Action queued again.", after: () => void load() }))))));
      fill(blockedBox, section("Blocked actions", "Each row holds the latest desired state for its object. Fix the credential, then replay.", t.wrap));
    } catch {
      fill(blockedBox);
    }
  }

  const github = textField("GitHub repository", { placeholder: "owner/name", required: true, class: "w-md", autocapitalize: "none", spellcheck: false });
  const mode = comboBox("Mode", [
    { value: "authoritative_vcsstar", label: "vcsstar is authoritative", description: "GitHub-only branches are removed" },
    { value: "import_drift", label: "Import drift", description: "Branches pushed to GitHub are imported; needed for both-ways pull requests" },
    { value: "flag_drift", label: "Flag drift", description: "Differences are reported, not changed" },
  ], { value: "import_drift" });
  const pullEvery = spinBox("Pull requests every (minutes)", { min: 1, max: 10080, value: 15 });
  const refsEvery = spinBox("Branches and tags every (minutes)", { min: 1, max: 10080, value: 15 });
  const schedule = c.can("admin") && section("Schedule", "How often this repository asks GitHub for changes that webhooks missed. Webhooks apply at once either way.",
    form({ class: "form-row", onSubmit: () => void act(() => api.mirror.setSchedule({ ctx: ctx(), repo, pullSyncIntervalS: Math.round(Number(pullEvery.value) * 60), refReconcileIntervalS: Math.round(Number(refsEvery.value) * 60) }), { done: "Schedule saved.", after: () => void load() }) },
      pullEvery, refsEvery, button("Save", { type: "submit", variant: "default" }),
      textButton("Use the defaults", () => void act(() => api.mirror.setSchedule({ ctx: ctx(), repo, pullSyncIntervalS: 0, refReconcileIntervalS: 0 }), { done: "Schedule set to the defaults.", after: () => void load() }))));
  const configure = c.can("admin") && section("Configure", "Credentials are set on the organization page, with the purposes mirror_git_push and webhook_inbound.",
    form({ class: "form-row", onSubmit: () => void act(() => api.mirror.configure({ ctx: ctx(), repo, githubRepo: github.value, mode: mode.value }), { done: "Mirror configured.", after: () => void load() }) }, github, mode, button("Save", { type: "submit", variant: "default" })));

  fill(statusBox, loading());
  void load();
  return { element: h("div", null, feedback.element, statusBox, blockedBox, schedule, configure), update() {}, refresh() { void load(); } };
}

// --- Settings ----------------------------------------------------------------

function settingsTab(c: Common): Tab {
  const repo = { org: c.org, name: c.name };
  const feedback = new Feedback();

  // General
  const description = textField("Description", { value: c.repo.description ?? "", class: "w-xl" });
  const visibility = comboBox("Visibility", [{ value: "public", label: "Public" }, { value: "org", label: "Organization" }, { value: "private", label: "Private" }], { value: c.repo.visibility });
  const archive = button(c.repo.archived ? "Unarchive" : "Archive", { variant: "default", onClick: () => void act(() => api.repos.update({ ctx: ctx(), repo, archived: !c.repo.archived }), { done: c.repo.archived ? "Repository unarchived." : "Repository archived.", after: () => c.onRepoChange() }) });
  const general = section("General", null, form({ class: "form-row", onSubmit: () => void act(() => api.repos.update({ ctx: ctx(), repo, description: description.value, visibility: visibility.value }), { done: "Saved.", after: () => c.onRepoChange() }) }, description, visibility, button("Save", { type: "submit", variant: "default" }), archive));

  // Protections
  const prots = table(["Pattern", "Approvals", "Checks", "Signatures", "Merge kinds", "Force / delete", ""]);
  const protStatus = h("div");
  async function loadProts() {
    try {
      const page = await api.repos.listProtections({ ctx: ctx(), repo });
      fill(prots.body, ...page.protections.map((x) => tr(
        td("", mono(x.refPattern)),
        `${x.requiredApprovals}${x.dismissStale ? ", stale dismissed" : ""}`,
        td("small", x.requiredChecks.join(", ") || "none"),
        x.requiredSignatureLevel,
        td("small", x.allowedMergeKinds.join(", ") || "any"),
        td("small", `${x.allowForcePush ? "force allowed" : "no force"}, ${x.allowDelete ? "delete allowed" : "no delete"}`),
        td("actions", textButton("Remove", async () => {
          if (await confirmDanger(`Remove the protection of ${x.refPattern}?`, "Pushes and merges to these branches are then unguarded.", "Remove protection")) void act(() => api.repos.removeProtection({ ctx: ctx(), repo, refPattern: x.refPattern }), { done: "Protection removed.", after: () => void loadProts() });
        }, { danger: true })),
      )));
      fill(protStatus, page.protections.length === 0 && empty("No branch protections yet."));
    } catch (error) {
      fill(protStatus);
      feedback.error(error);
    }
  }
  const pattern = textField("Ref pattern", { value: c.repo.defaultBranch, required: true, placeholder: "main, release/*", class: "w-md" });
  const approvals = spinBox("Approvals", { value: 1, min: 0, max: 20 });
  const dismiss = checkBox("Dismiss stale approvals", { checked: true });
  const signatures = comboBox("Signatures", [{ value: "none", label: "Not required" }, { value: "any", label: "Any valid signature" }, { value: "attested", label: "Attested key" }], { value: "none" });
  const checks = textField("Required checks", { placeholder: "ci/test, ci/lint", class: "w-lg" });
  const kinds = [["merge", "Merge commit"], ["squash", "Squash"], ["rebase", "Rebase"]].map(([v, l]) => ({ value: v!, box: checkBox(l, { checked: true }) }));
  const force = checkBox("Allow force push");
  const del = checkBox("Allow delete");
  const protForm = panel(h("h3", null, "Set a protection"), form({ class: "form-stack", onSubmit: () => {
    const protection: BranchProtection = { refPattern: pattern.value, requiredApprovals: approvals.value ?? 0, dismissStale: dismiss.checked, requiredSignatureLevel: signatures.value, requiredChecks: csv(checks.value), allowedMergeKinds: kinds.filter((k) => k.box.checked).map((k) => k.value), allowForcePush: force.checked, allowDelete: del.checked };
    void act(() => api.repos.setProtection({ ctx: ctx(), repo, protection }), { done: "Protection saved.", after: () => void loadProts() });
  } },
    h("div", { class: "form-row" }, pattern, approvals, signatures, checks),
    h("div", { class: "form-row checks" }, dismiss, ...kinds.map((k) => k.box), force, del),
    button("Save protection", { type: "submit", variant: "default" }),
  ));
  const protections = section("Branch protections", "A protection applies to every ref that matches its pattern.", prots.wrap, protStatus, protForm);

  // Access
  const accessBox = h("div");
  async function loadGrants() {
    let page;
    try {
      page = await api.repos.listGrants({ ctx: ctx(), repo });
    } catch {
      fill(accessBox);
      return;
    }
    const t = table(["Grantee", "Role", ""]);
    fill(t.body, ...page.grants.map((g) => tr(td("", h("span", { class: "muted" }, `${g.granteeKind} `), mono(g.grantee)), g.role,
      td("actions", textButton("Remove", async () => {
        if (await confirmDanger(`Remove the grant of ${g.grantee}?`, "They keep what the organization gives them.", "Remove grant")) void act(() => api.repos.removeGrant({ ctx: ctx(), repo, granteeKind: g.granteeKind, grantee: g.grantee }), { done: "Grant removed.", after: () => void loadGrants() });
      }, { danger: true })))));
    const kind = comboBox("Grantee kind", [{ value: "identity", label: "Identity" }, { value: "team", label: "Team" }], { value: "identity" });
    const grantee = textField("Grantee", { required: true, placeholder: "user_id@domain or team name", class: "w-lg", autocapitalize: "none", spellcheck: false });
    const role = comboBox("Role", ["read", "write", "maintain", "admin"], { value: "write" });
    fill(accessBox, section("Access", "Grants on this repository, on top of the organization roles.", t.wrap, page.grants.length === 0 && empty("No repository grants."),
      form({ class: "form-row", onSubmit: () => void act(async () => { await api.repos.setGrant({ ctx: ctx(), repo, grant: { granteeKind: kind.value, grantee: grantee.value, role: role.value } }); clear(grantee); }, { done: "Grant set.", after: () => void loadGrants() }) }, kind, grantee, role, button("Grant", { type: "submit", variant: "default" }))));
  }

  fill(protStatus, loading());
  void loadProts();
  void loadGrants();
  return { element: h("div", null, feedback.element, general, protections, accessBox), update() {}, refresh() { void loadProts(); void loadGrants(); } };
}

function duration(seconds: number): string {
  if (seconds % 3600 === 0) return seconds === 3600 ? "hour" : `${seconds / 3600} hours`;
  if (seconds % 60 === 0) return seconds === 60 ? "minute" : `${seconds / 60} minutes`;
  return `${seconds} s`;
}

/** One file of the code view: highlighted source, a rendered document or
 * diagram (with the source one click away), media shown inline, and raw
 * and download for every file. Raw opens text as plain text (an HTML or
 * SVG file never runs as a page of this site). */
async function fileView(repo: { org: string; name: string }, rev: string, path: string): Promise<HTMLElement> {
  const blob = await api.repos.blob({ ctx: ctx(), repo, rev, path, limit: 1024 * 1024 });
  const type = mediaType(path);
  const name = path.split("/").pop() ?? "file";
  const body = h("div", { class: "file-body" });
  const status = h("span", { class: "muted small" });
  const open = async (asText: boolean) => {
    status.textContent = "Loading…";
    try {
      const { bytes } = await readAll(repo, rev, path);
      const shown = asText ? "text/plain;charset=utf-8" : type && type !== "image/svg+xml" ? type : "text/plain;charset=utf-8";
      window.open(URL.createObjectURL(new Blob([bytes as BlobPart], { type: shown })), "_blank", "noopener");
      status.textContent = "";
    } catch (error) { status.textContent = String((error as Error).message ?? error); }
  };
  const download = async () => {
    status.textContent = "Loading…";
    try {
      const { bytes } = await readAll(repo, rev, path);
      const a = h("a", { href: URL.createObjectURL(new Blob([bytes as BlobPart], { type: "application/octet-stream" })), download: name }) as HTMLAnchorElement;
      document.body.append(a); a.click(); a.remove();
      status.textContent = "";
    } catch (error) { status.textContent = String((error as Error).message ?? error); }
  };
  const tools: unknown[] = [];
  const text = blob.binary ? null : bytesToText(blob.content);
  if (type && (blob.binary || type.startsWith("image/"))) {
    // Media: shown inline from the full bytes.
    fill(body, loading());
    void rawUrl(repo, rev, path).then((url) => fill(body, url ? h("div", { class: "media-view" }, mediaElement(type, url, path)) : h("p", { class: "problem" }, "The file cannot be read.")));
    if (type !== "image/svg+xml") tools.push(textButton("Open", () => void open(false)));
  } else if (text === null) {
    fill(body, h("p", { class: "hint" }, "Binary file. Download it, or open it if your browser can show it."));
    tools.push(textButton("Open as text", () => void open(true)));
  } else {
    const renderable = !blob.truncated && (isMarkdown(path) || isMermaid(path));
    const showSource = () => fill(body, codeView(text, path));
    const showRendered = () => {
      if (isMermaid(path)) { const holder = h("div", { class: "mermaid-view" }); fill(body, holder); void drawMermaid(holder, text); }
      else fill(body, markdown(text, { ...repo, rev, dir: path.split("/").slice(0, -1).join("/") }));
    };
    if (renderable) {
      const rendered = textButton("Rendered", () => { rendered.setAttribute("aria-pressed", "true"); source.setAttribute("aria-pressed", "false"); showRendered(); });
      const source = textButton("Source", () => { rendered.setAttribute("aria-pressed", "false"); source.setAttribute("aria-pressed", "true"); showSource(); });
      rendered.setAttribute("aria-pressed", "true");
      tools.push(rendered, source);
      showRendered();
    } else showSource();
    tools.push(textButton("Raw", () => void open(true)));
  }
  tools.push(textButton("Download", () => void download()));
  return h("div", { class: "file-view" },
    h("div", { class: "row file-head" }, meta(formatSize(blob.size), blob.truncated && "first 1 MB shown", languageOf(path) && mono(languageOf(path)!)), h("span", { class: "grow" }), ...(tools as never[]), status),
    body);
}
