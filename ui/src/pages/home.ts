// Repositories: every repository the viewer can see, the organizations, and
// the two create forms for a signed-in viewer.
import { api, ctx, when } from "../api";
import { button, comboBox, fill, form, h, link, textField, clear } from "../dom";
import { Page } from "../page";
import { act, badge, empty, Feedback, loading, meta, mono, pageHead, panel, section, sortKey, table, td, timeCell, tr } from "../ui";
import type { Org, Repo } from "../generated/types.gen.ts";

export class HomePage extends Page {
  title = "Repositories";
  private readonly repoTable = table(["Repository", "Visibility", "Default branch", "Mirror", "Last push", "Created"]);
  private readonly repoStatus = h("div");
  private readonly count = h("p", { class: "lead" });
  private readonly orgList = h("div", { class: "chips" });
  private readonly installBox = h("div");
  private readonly feedback = new Feedback();
  private orgBox!: ZestComboBox;
  private orgs: Org[] = [];

  protected build() {
    const signedIn = Boolean(this.context.session);
    return [
      pageHead([], "Repositories", this.count),
      this.feedback.element,
      this.repoTable.wrap,
      this.repoStatus,
      section("Organizations", null, this.orgList),
      this.installBox,
      signedIn && h("div", { class: "panel-grid" }, this.newRepoPanel(), this.newOrgPanel()),
    ];
  }

  protected async start() {
    fill(this.repoStatus, loading());
    await Promise.all([this.loadRepos(), this.loadOrgs()]);
    if (this.context.session) await this.loadInstallations();
  }

  // Where the instance's GitHub Apps are installed. An owner without an
  // organization here gets a button that makes one with that GitHub owner;
  // its page then lists the repositories to enable.
  private async loadInstallations() {
    let list;
    try {
      list = await api.mirror.listGithubInstallations({ ctx: ctx() });
    } catch {
      fill(this.installBox);
      return;
    }
    if (!this.isActive()) return;
    if (list.installations.length === 0 && list.errors.length === 0) { fill(this.installBox); return; }
    const t = table(["GitHub owner", "App", "Repositories", "Here"]);
    fill(t.body, ...list.installations.map((i) => {
      const here = h("td");
      if (i.vcsstarOrgs.length) fill(here, ...i.vcsstarOrgs.map((o) => link(`/${o}`, o)));
      else {
        const name = i.owner.toLowerCase().replace(/[^a-z0-9-]/g, "-");
        const make = button(`Create ${name}`, { variant: "default", onClick: async () => {
          make.disabled = true;
          const ok = await act(() => api.orgs.create({ ctx: ctx(), name, githubOwner: i.owner }), { done: `Organization ${name} created for ${i.owner}.`, after: () => { void this.loadOrgs(); void this.loadInstallations(); } });
          if (!ok) make.disabled = false;
        } });
        fill(here, make);
      }
      return tr(
        td("", mono(i.owner), " ", badge(i.ownerType === "Organization" ? "org" : "user")),
        td("", i.appSlug ? mono(i.appSlug) : mono(i.appId), h("div", { class: "hint" }, `installation ${i.installationId}`)),
        i.repositorySelection === "all" ? "all" : "selected",
        here,
      );
    }));
    fill(this.installBox, section("GitHub App installations", "Owners where this instance's GitHub Apps are installed. Open the organization to enable its repositories.", t.wrap, ...list.errors.map((e) => h("p", { class: "problem" }, e))));
  }

  private async loadRepos() {
    try {
      const page = await api.repos.list({ ctx: ctx(), page: { limit: 200 } });
      if (!this.isActive()) return;
      this.showRepos(page.repos);
    } catch (error) {
      if (this.isActive()) { fill(this.repoStatus); this.feedback.error(error); }
    }
  }

  private showRepos(repos: Repo[]) {
    this.count.textContent = repos.length === 1 ? "One repository" : `${repos.length} repositories`;
    fill(this.repoTable.body, ...repos.map((r) =>
      tr(
        sortKey(td("", link(`/${r.org}/${r.name}`, h("span", { class: "repo-org" }, `${r.org} / `), h("strong", null, r.name)), r.description && h("div", { class: "hint" }, r.description)), `${r.org}/${r.name}`),
        r.visibility,
        td("", mono(r.defaultBranch)),
        td("", r.mirrorEnabled ? badge("mirrored", "info") : badge("local")),
        timeCell(r.pushedAt, "never"),
        timeCell(r.createdAt),
      ),
    ));
    fill(this.repoStatus, repos.length === 0 && empty(this.context.session ? "No repositories you can see yet. Create one below, or enable a GitHub repository from an organization page." : "No public repositories yet. Sign in to see the ones you have access to."));
  }

  private async loadOrgs() {
    try {
      const page = await api.orgs.list({ ctx: ctx(), page: { limit: 200 } });
      if (!this.isActive()) return;
      this.orgs = page.orgs;
      fill(this.orgList, ...page.orgs.map((o) => link(`/${o.name}`, o.displayName ?? o.name)));
      if (page.orgs.length === 0) fill(this.orgList, h("p", { class: "hint" }, "No organizations yet."));
      if (this.orgBox) this.orgBox.options = page.orgs.map((o) => o.name);
    } catch (error) {
      if (this.isActive()) this.feedback.error(error);
    }
  }

  private newRepoPanel() {
    this.orgBox = comboBox("Organization", this.orgs.map((o) => o.name), { required: true, placeholder: "Select" });
    const name = textField("Name", { required: true, autocapitalize: "none", spellcheck: false });
    const visibility = comboBox("Visibility", [
      { value: "org", label: "Organization", description: "Members of the organization" },
      { value: "private", label: "Private", description: "Only people with a grant" },
      { value: "public", label: "Public", description: "Anyone, read only" },
    ], { value: "org" });
    const submit = button("Create repository", { type: "submit" });
    return panel(
      h("h3", null, "New repository"),
      form({ class: "form-row", onSubmit: () => void act(async () => {
        await api.repos.create({ ctx: ctx(), org: this.orgBox.value, name: name.value, visibility: visibility.value });
        clear(name);
      }, { done: "Repository created.", after: () => void this.loadRepos() }) }, this.orgBox, name, visibility, submit),
    );
  }

  private newOrgPanel() {
    const name = textField("Name", { required: true, autocapitalize: "none", spellcheck: false, helper: "Needs instance admin rights unless open creation is on." });
    return panel(
      h("h3", null, "New organization"),
      form({ class: "form-row", onSubmit: () => void act(async () => {
        await api.orgs.create({ ctx: ctx(), name: name.value });
        clear(name);
      }, { done: "Organization created.", after: () => void this.loadOrgs() }) }, name, button("Create organization", { type: "submit" })),
      meta(),
    );
  }
}
