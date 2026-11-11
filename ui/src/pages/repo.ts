// A repository: code, commits, pull requests, the mirror, webhooks, and
// settings. The tabs are routes, so the page stays while the tab changes.
import { api, bytesToText, ctx, short, when, who } from "../api";
import { button, checkBox, comboBox, fill, form, h, link, spinBox, textArea, textButton, textField, clear } from "../dom";
import { live } from "../live";
import { Page } from "../page";
import { act, badge, confirmDanger, csv, empty, Feedback, firstLine, isoDate, loading, meta, mono, pageHead, panel, refShort, section, table, td, tr } from "../ui";
import { diffView } from "./diff";
import { hooksSection } from "./hooks";
import { sigTone } from "./pull";
import type { BranchProtection, Repo } from "../generated/types.gen.ts";

const TABS = [["code", "Code"], ["commits", "Commits"], ["pulls", "Pull requests"], ["mirror", "Mirror"], ["hooks", "Webhooks"], ["settings", "Settings"]] as const;
type TabId = (typeof TABS)[number][0];

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
    this.tabId = TABS.some(([id]) => id === tab) ? tab : "code";
    this.rest = parts.slice(3);
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
    if (this.repo) this.showTab();
  }

  private async loadRepo() {
    try {
      const r = await api.repos.get({ ctx: ctx(), repo: this.ref });
      if (!this.isActive()) return;
      this.repo = r;
      fill(this.facts, badge(r.visibility), r.archived && badge("archived", "warn"), r.mirrorEnabled && badge("mirrored", "info"), r.forkOf && h("span", null, "fork of ", link(`/${r.forkOf.org}/${r.forkOf.name}`, `${r.forkOf.org}/${r.forkOf.name}`)), h("span", { class: "muted" }, "default branch ", mono(r.defaultBranch)));
      this.description.textContent = r.description ?? "";
      this.description.hidden = !r.description;
      fill(this.clone, cloneField("HTTPS", r.httpsUrl), r.sshUrl && cloneField("SSH", r.sshUrl));
    } catch (error) {
      if (this.isActive()) { fill(this.body); this.feedback.error(error); }
    }
  }

  private renderTabNav() {
    fill(this.tabNav, ...TABS.map(([id, label]) => link(id === "code" ? this.base : `${this.base}/${id}`, label)).map((a, i) => { if (TABS[i]![0] === this.tabId) a.setAttribute("aria-current", "page"); return a; }));
  }

  private showTab() {
    const repo = this.repo!;
    const common = { org: this.org, name: this.name, repo, navigate: this.context.navigate, signedIn: Boolean(this.context.session), base: this.base, onRepoChange: () => void this.loadRepo() };
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

type Common = { org: string; name: string; repo: Repo; navigate(path: string): void; signedIn: boolean; base: string; onRepoChange(): void };

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
      fill(content, h("div", { class: "tree" },
        path && link(href(path.split("/").slice(0, -1).join("/")), h("span", { class: "tree-icon", "aria-hidden": "true" }, "↑"), ".."),
        ...tree.entries.map((e) => link(href(path ? `${path}/${e.name}` : e.name), h("span", { class: "tree-icon", "aria-hidden": "true" }, e.kind === "tree" ? "▸" : "·"), h("span", { class: e.kind === "tree" ? "tree-dir" : "" }, e.name), e.kind === "blob" && h("span", { class: "muted small size" }, formatSize(e.size ?? 0)))),
      ));
      return;
    } catch (treeError) {
      if (!path) { fill(content); feedback.error(treeError); return; }
    }
    try {
      const blob = await api.repos.blob({ ctx: ctx(), repo, rev: refName, path, limit: 512 * 1024 });
      const text = blob.binary ? null : bytesToText(blob.content);
      fill(content, meta(formatSize(blob.size), blob.truncated && "truncated"), text === null ? h("p", { class: "hint" }, "Binary file.") : h("pre", { class: "code" }, text));
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
        diff && diffView(diff.files),
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
  const list = table(null);
  const status = h("div");
  const feedback = new Feedback();
  const formBox = h("div");
  let formOpen = false;

  async function load() {
    try {
      const page = await api.pullRequests.list({ ctx: ctx(), repo, state: state.value, page: { limit: 50 } });
      fill(list.body, ...page.pullRequests.map((pr) => tr(
        td("", link(`${c.base}/pulls/${pr.number}`, h("span", { class: "muted pr-number" }, `#${pr.number} `), pr.title), meta(who(pr.author), h("span", null, mono(refShort(pr.head.ref)), " into ", mono(refShort(pr.baseRef))), when(pr.updatedAt))),
        td("", badge(pr.state, pr.state === "merged" ? "ok" : pr.state === "closed" ? "bad" : "info"), pr.state === "open" && badge(pr.mergeable === "clean" ? "can merge" : pr.mergeable, pr.mergeable === "clean" ? "ok" : pr.mergeable === "conflicts" ? "bad" : "neutral")),
      )));
      fill(status, page.pullRequests.length === 0 && empty(`No ${state.value === "all" ? "" : state.value + " "}pull requests.`));
    } catch (error) {
      fill(status);
      feedback.error(error);
    }
  }

  async function openForm() {
    const refs = await api.repos.listRefs({ ctx: ctx(), repo, prefix: "refs/heads/" }).catch(() => ({ refs: [] }));
    const branches = refs.refs.map((r) => refShort(r.name));
    const head = comboBox("Head branch", branches, { required: true, placeholder: "Select" });
    const base = comboBox("Base branch", branches, { value: c.repo.defaultBranch });
    const title = textField("Title", { required: true, class: "w-xl" });
    const body = textArea("Description", { rows: 5, class: "w-xl" });
    fill(formBox, panel(h("h3", null, "New pull request"), form({ class: "form-stack", onSubmit: () => void act(async () => {
      const pr = await api.pullRequests.open({ ctx: ctx(), repo, title: title.value, body: body.value || undefined, baseRef: base.value, head: { repo, ref: head.value } });
      c.navigate(`${c.base}/pulls/${pr.number}`);
    }) }, h("div", { class: "form-row" }, head, base), title, body, h("div", { class: "row" }, button("Open pull request", { type: "submit" }), textButton("Cancel", () => { formOpen = false; fill(formBox); })))));
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
      fill(statusBox, panel(
        h("div", { class: "row" }, h("strong", null, "GitHub"), s.githubRepo ? mono(s.githubRepo) : h("span", { class: "muted" }, "not configured"), badge(s.mode.replace(/_/g, " ")), badge(s.paused ? "paused" : "active", s.paused ? "warn" : "ok"), badge(`${s.pending} pending`), badge(`${s.blocked} blocked`, s.blocked ? "bad" : "neutral")),
        meta(h("span", null, `last success ${when(s.lastSuccessAt) || "never"}`), h("span", null, `last full reconcile ${when(s.lastFullReconcileAt) || "never"}`), h("span", null, `pull requests synced from GitHub up to ${s.lastPullSyncAt ? isoDate(s.lastPullSyncAt) : "never"}`)),
        s.lastError && h("p", { class: "problem" }, s.lastError),
        h("div", { class: "row" },
          button("Reconcile now", { variant: "default", onClick: () => void act(() => api.mirror.reconcile({ ctx: ctx(), repo }), { done: "Reconcile ran.", after: () => void load() }) }),
          s.paused
            ? button("Resume", { variant: "default", onClick: () => void act(() => api.mirror.resume({ ctx: ctx(), repo }), { done: "Mirror resumed.", after: () => void load() }) })
            : button("Pause", { variant: "default", onClick: () => void act(() => api.mirror.pause({ ctx: ctx(), repo }), { done: "Mirror paused.", after: () => void load() }) }),
          button("Replay all blocked", { variant: "default", disabled: !s.blocked, onClick: () => void act(() => api.mirror.replayBlocked({ ctx: ctx(), repo, actionIds: [] }), { done: "Blocked actions queued again.", after: () => void load() }) }),
        ),
      ));
    } catch (error) {
      fill(statusBox);
      feedback.error(error);
    }
    try {
      const b = await api.mirror.listBlocked({ ctx: ctx(), repo });
      if (b.actions.length === 0) { fill(blockedBox); return; }
      const t = table(["Object", "Kind", "Credential purpose", "Exact error", "Blocked", ""]);
      fill(t.body, ...b.actions.map((a) => tr(td("", mono(a.objectKey)), a.actionKind, a.credentialPurpose, td("small", a.error), td("muted", when(a.blockedAt)),
        td("actions", textButton("Replay", () => void act(() => api.mirror.replayBlocked({ ctx: ctx(), repo, actionIds: [a.id] }), { done: "Action queued again.", after: () => void load() }))))));
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
  const configure = c.signedIn && section("Configure", "Credentials are set on the organization page, with the purposes mirror_git_push and webhook_inbound.",
    form({ class: "form-row", onSubmit: () => void act(() => api.mirror.configure({ ctx: ctx(), repo, githubRepo: github.value, mode: mode.value }), { done: "Mirror configured.", after: () => void load() }) }, github, mode, button("Save", { type: "submit", variant: "default" })));

  fill(statusBox, loading());
  void load();
  return { element: h("div", null, feedback.element, statusBox, blockedBox, configure), update() {}, refresh() { void load(); } };
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
