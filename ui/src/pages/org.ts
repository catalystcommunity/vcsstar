// An organization: its repositories, the GitHub repositories it can enable,
// members, teams, mirror credentials, service accounts, and webhooks.
import { api, ctx, when, who } from "../api";
import { button, comboBox, fill, form, h, link, external, show, textArea, textButton, textField, clear } from "../dom";
import { Page } from "../page";
import { act, badge, confirmDanger, csv, empty, Feedback, loading, meta, mono, pageHead, panel, section, showSecretOnce, table, td, tr } from "../ui";
import { hooksSection } from "./hooks";
import type { GithubRepoPage, Org } from "../generated/types.gen.ts";

export class OrgPage extends Page {
  private org = "";
  private info?: Org;
  private readonly feedback = new Feedback();
  private readonly titleText = document.createTextNode("");
  private readonly facts = h("div", { class: "meta" });
  private readonly repoList = h("ul", { class: "plain-list" });
  private readonly repoStatus = h("div");
  private readonly githubSection = h("div");
  private readonly members = table(["Person", "Role", ""]);
  private readonly membersBox = h("div");
  private readonly teamsBox = h("div");
  private readonly credsBox = h("div");
  private readonly accountsBox = h("div");
  private readonly hooksBox = h("div");
  private ghCursor: string | undefined;

  protected build() {
    this.org = this.context.path.split("/").filter(Boolean).map(decodeURIComponent)[0] ?? "";
    this.title = this.org;
    return [
      pageHead([["Repositories", "/"]], this.titleText, this.facts),
      this.feedback.element,
      section("Repositories", null, this.repoList, this.repoStatus),
      this.githubSection,
      this.membersBox,
      this.teamsBox,
      this.credsBox,
      this.accountsBox,
      this.hooksBox,
    ];
  }

  protected async start() {
    this.titleText.data = this.org;
    fill(this.repoStatus, loading());
    await this.loadInfo();
    await Promise.all([this.loadRepos(), this.loadGithub(), this.loadMembers(), this.loadTeams(), this.loadCreds(), this.loadAccounts()]);
    if (this.isActive()) fill(this.hooksBox, hooksSection(this.org));
  }

  private async loadInfo() {
    try {
      this.info = await api.orgs.get({ ctx: ctx(), org: this.org });
      if (!this.isActive()) return;
      this.titleText.data = this.info.displayName ?? this.org;
      fill(this.facts, this.info.displayName && h("span", null, mono(this.org)), this.info.githubOwner && h("span", null, "GitHub owner ", external(`https://github.com/${this.info.githubOwner}`, this.info.githubOwner)), h("span", { class: "muted" }, `created ${when(this.info.createdAt)}`));
    } catch (error) {
      if (this.isActive()) this.feedback.error(error);
    }
  }

  private async loadRepos() {
    try {
      const page = await api.repos.list({ ctx: ctx(), org: this.org, page: { limit: 200 } });
      if (!this.isActive()) return;
      fill(this.repoList, ...page.repos.map((r) => h("li", null, link(`/${r.org}/${r.name}`, h("strong", null, r.name)), " ", badge(r.visibility), r.mirrorEnabled && badge("mirrored", "info"), r.description && h("div", { class: "hint" }, r.description))));
      fill(this.repoStatus, page.repos.length === 0 && empty("No repositories in this organization yet."));
    } catch (error) {
      if (this.isActive()) { fill(this.repoStatus); this.feedback.error(error); }
    }
  }

  // --- GitHub repositories -------------------------------------------------

  private async loadGithub() {
    const owner = this.info?.githubOwner;
    if (!owner) { fill(this.githubSection); return; }
    const feedback = new Feedback();
    const t = table(["Repository", "Visibility", "Default branch", "Last push", "Here"]);
    const status = h("div", null, loading());
    const paging = h("div", { class: "row" });
    const lead = h("p", { class: "lead" }, "Repositories under ", mono(owner), " on GitHub. Enable one to import it and keep both sides in step.");
    fill(this.githubSection, h("section", { class: "section" }, h("div", { class: "section-head" }, h("h2", null, "GitHub repositories"), lead), feedback.element, t.wrap, status, paging));
    let page: GithubRepoPage;
    try {
      page = await api.mirror.listGithubRepos({ ctx: ctx(), org: this.org, page: { cursor: this.ghCursor, limit: 100 } });
    } catch (error) {
      if (this.isActive()) { fill(status); feedback.error(error); }
      return;
    }
    if (!this.isActive()) return;
    fill(lead, "Repositories under ", mono(owner), " on GitHub, as seen by ", mono(page.source), page.installationId ? ` (installation ${page.installationId})` : "", ". Enable one to import it and keep both sides in step.");
    fill(t.body, ...page.repos.map((r) => {
      const here = h("td");
      if (r.mirroredAs) fill(here, link(`/${this.org}/${r.mirroredAs}`, r.mirroredAs));
      else {
        const enable = button("Enable", { variant: "default", onClick: async () => {
          enable.disabled = true;
          const ok = await act(() => api.mirror.importRepo({ ctx: ctx(), org: this.org, name: r.fullName.split("/")[1]!, githubRepo: r.fullName, mode: "import_drift", visibility: r.private ? "private" : "public" }), { done: `${r.fullName} is being imported.` });
          if (ok) { void this.loadRepos(); void this.loadGithub(); } else enable.disabled = false;
        } });
        fill(here, enable);
      }
      return tr(
        td("", external(r.htmlUrl, r.fullName), r.description && h("div", { class: "hint" }, r.description)),
        td("", badge(r.private ? "private" : "public"), r.archived && badge("archived", "warn"), r.fork && badge("fork")),
        td("", mono(r.defaultBranch)),
        td("muted", r.pushedAt ? when(r.pushedAt) : "never"),
        here,
      );
    }));
    fill(status, page.repos.length === 0 && empty("GitHub lists no repositories for this owner with the current credential."));
    fill(paging, this.ghCursor && textButton("First page", () => { this.ghCursor = undefined; void this.loadGithub(); }), page.nextCursor && textButton("More", () => { this.ghCursor = page.nextCursor!; void this.loadGithub(); }));
  }

  // --- Members and teams ---------------------------------------------------

  private async loadMembers() {
    let page;
    try {
      page = await api.orgs.listMembers({ ctx: ctx(), org: this.org, page: { limit: 200 } });
    } catch {
      fill(this.membersBox);
      return;
    }
    if (!this.isActive()) return;
    fill(this.members.body, ...page.members.map((m) => tr(
      td("", who(m.identity), h("div", { class: "hint mono" }, `${m.identity.userId}@${m.identity.domain}`)),
      m.role,
      td("actions", textButton("Remove", async () => {
        if (await confirmDanger(`Remove ${who(m.identity)}?`, "They keep nothing in this organization except repository grants made to them directly.", "Remove member")) {
          void act(() => api.orgs.removeMember({ ctx: ctx(), org: this.org, identity: `${m.identity.userId}@${m.identity.domain}` }), { done: "Member removed.", after: () => void this.loadMembers() });
        }
      }, { danger: true })),
    )));
    if (this.membersBox.childElementCount) return;
    const identity = textField("Identity", { required: true, placeholder: "user_id@domain", class: "w-lg", autocapitalize: "none", spellcheck: false, helper: "A person appears after one sign-in. The key is user_id@domain, never the handle." });
    const role = comboBox("Role", ["member", "admin", "owner"], { value: "member" });
    fill(this.membersBox, section("Members", null, this.members.wrap,
      form({ class: "form-row", onSubmit: () => void act(async () => { await api.orgs.setMember({ ctx: ctx(), org: this.org, identity: identity.value, role: role.value }); clear(identity); }, { done: "Member set.", after: () => void this.loadMembers() }) }, identity, role, button("Add member", { type: "submit", variant: "default" })),
    ));
  }

  private async loadTeams() {
    let page;
    try {
      page = await api.orgs.listTeams({ ctx: ctx(), org: this.org });
    } catch {
      fill(this.teamsBox);
      return;
    }
    if (!this.isActive()) return;
    const list = h("div", { class: "stack" }, ...page.teams.map((t) => h("div", { class: "row team" },
      h("strong", null, t.name),
      h("span", { class: "muted" }, t.members.map(who).join(", ") || "no members"),
      h("span", { class: "grow" }),
      textButton("Delete", async () => {
        if (await confirmDanger(`Delete the team ${t.name}?`, "Grants made to the team end.", "Delete team")) {
          void act(() => api.orgs.deleteTeam({ ctx: ctx(), org: this.org, team: t.name }), { done: "Team deleted.", after: () => void this.loadTeams() });
        }
      }, { danger: true }),
    )), page.teams.length === 0 && empty("No teams yet."));
    const name = textField("Team name", { required: true, class: "w-md" });
    fill(this.teamsBox, section("Teams", null, list,
      form({ class: "form-row", onSubmit: () => void act(async () => { await api.orgs.createTeam({ ctx: ctx(), org: this.org, name: name.value }); clear(name); }, { done: "Team created.", after: () => void this.loadTeams() }) }, name, button("Create team", { type: "submit", variant: "default" })),
    ));
  }

  // --- Mirror credentials --------------------------------------------------

  private async loadCreds() {
    let page;
    try {
      page = await api.mirror.listCredentials({ ctx: ctx(), target: { org: this.org } });
    } catch {
      fill(this.credsBox);
      return;
    }
    if (!this.isActive()) return;
    const t = table(["Scope", "Purpose", "Method", "Secret", ""]);
    fill(t.body, ...page.credentials.map((c) => tr(
      c.repo ? `${c.org}/${c.repo}` : c.org,
      td("", mono(c.purpose)),
      c.authMethod === "github_app" ? "GitHub App" : "personal access token",
      td("", mono(c.secretRef)),
      td("actions", textButton("Remove", async () => {
        if (await confirmDanger("Remove this credential?", "Mirror actions that need it block until another member of the set works.", "Remove credential")) {
          void act(() => api.mirror.removeCredential({ ctx: ctx(), credentialId: c.id }), { done: "Credential removed.", after: () => void this.loadCreds() });
        }
      }, { danger: true })),
    )));
    const purpose = comboBox("Purpose", ["mirror_git_push", "mirror_api", "import_fetch", "webhook_inbound"], { value: "mirror_git_push" });
    const method = comboBox("Method", [{ value: "pat", label: "Personal access token" }, { value: "github_app", label: "GitHub App" }], { value: "pat" });
    const secretRef = textField("Secret reference", { value: "stored", helper: "stored, or env:VAR for a secret the server has in its environment", class: "w-md" });
    const token = textField("Token", { type: "password", autocomplete: "off", class: "w-lg" });
    const appJson = textArea("App credential", { rows: 4, placeholder: '{"app_id": "…", "installation_id": "…", "private_key": "-----BEGIN RSA PRIVATE KEY-----…"}', class: "w-xl" });
    const sync = () => { const stored = secretRef.value === "stored"; show(token, stored && method.value === "pat"); show(appJson, stored && method.value === "github_app"); };
    method.addEventListener("change", sync); secretRef.addEventListener("input", sync);
    appJson.hidden = true;
    fill(this.credsBox, section("Mirror credentials", "Credentials with the same purpose form a set; each action picks one. A stored secret is sealed with the instance key and never shown again.",
      t.wrap, page.credentials.length === 0 && empty("No credentials yet. The instance GitHub App and linked accounts still work without one."),
      panel(h("h3", null, "Set a credential"), form({ class: "form-stack", onSubmit: () => void act(async () => {
        const stored = secretRef.value === "stored";
        await api.mirror.setCredential({ ctx: ctx(), org: this.org, provider: "github", purpose: purpose.value, authMethod: method.value, host: "github.com", secretRef: secretRef.value, secret: stored ? (method.value === "pat" ? token.value : appJson.value) : undefined });
        clear(token); clear(appJson);
      }, { done: "Credential set.", after: () => void this.loadCreds() }) },
        h("div", { class: "form-row" }, purpose, method, secretRef),
        h("div", { class: "form-row" }, token, appJson),
        button("Set credential", { type: "submit", variant: "default" }),
      )),
    ));
  }

  // --- Service accounts ----------------------------------------------------

  private tokenFor: string | null = null;

  private async loadAccounts() {
    let page;
    try {
      page = await api.serviceAccounts.list({ ctx: ctx(), org: this.org });
    } catch {
      fill(this.accountsBox);
      return;
    }
    if (!this.isActive()) return;
    const t = table(["Account", "Identity", "Tokens", "State", ""]);
    fill(t.body, ...page.accounts.map((a) => tr(
      td("", h("strong", null, a.name), a.description && h("div", { class: "hint" }, a.description)),
      td("", mono(`${a.identity.userId}@${a.identity.domain}`, "small")),
      td("", String(a.activeTokens), " ", textButton(this.tokenFor === a.name ? "Hide tokens" : "Tokens", () => { this.tokenFor = this.tokenFor === a.name ? null : a.name; void this.loadAccounts(); })),
      td("", badge(a.disabledAt ? "disabled" : "active", a.disabledAt ? "bad" : "ok")),
      td("actions", a.disabledAt
        ? textButton("Enable", () => void act(() => api.serviceAccounts.enable({ ctx: ctx(), org: this.org, name: a.name }), { done: "Account enabled.", after: () => void this.loadAccounts() }))
        : textButton("Disable", async () => {
          if (await confirmDanger(`Disable ${a.name}?`, "Its tokens are refused until you enable it again.", "Disable account")) {
            void act(() => api.serviceAccounts.disable({ ctx: ctx(), org: this.org, name: a.name }), { done: "Account disabled.", after: () => void this.loadAccounts() });
          }
        }, { danger: true })),
    )));
    const tokensBox = h("div");
    if (this.tokenFor) void this.loadAccountTokens(this.tokenFor, tokensBox);
    const name = textField("Account name", { required: true, class: "w-md", autocapitalize: "none", spellcheck: false });
    const description = textField("Description", { class: "w-lg" });
    fill(this.accountsBox, section("Service accounts", "A service account never signs in. It holds API tokens for integrations that do not come through LinkKeys. Grant it roles like any identity, by its identity key.",
      t.wrap, page.accounts.length === 0 && empty("No service accounts yet."), tokensBox,
      form({ class: "form-row", onSubmit: () => void act(async () => { await api.serviceAccounts.create({ ctx: ctx(), org: this.org, name: name.value, description: description.value || undefined }); clear(name); clear(description); }, { done: "Service account created.", after: () => void this.loadAccounts() }) }, name, description, button("Create service account", { type: "submit", variant: "default" })),
    ));
  }

  private async loadAccountTokens(name: string, box: HTMLElement) {
    let page;
    try {
      page = await api.serviceAccounts.listTokens({ ctx: ctx(), org: this.org, name });
    } catch (error) {
      this.feedback.error(error);
      return;
    }
    if (!this.isActive() || this.tokenFor !== name) return;
    const t = table(["Name", "Scopes", "Created", "Expires", ""]);
    fill(t.body, ...page.tokens.map((k) => tr(
      k.name, td("small", k.scopes.join(", ")), td("muted", when(k.createdAt)), td("muted", k.expiresAt ? when(k.expiresAt) : "never"),
      td("actions", textButton("Revoke", async () => {
        if (await confirmDanger(`Revoke the token "${k.name}"?`, "Anything that uses it stops at once.", "Revoke token")) {
          void act(() => api.serviceAccounts.revokeToken({ ctx: ctx(), org: this.org, name, tokenId: k.id }), { done: "Token revoked.", after: () => void this.loadAccounts() });
        }
      }, { danger: true })),
    )));
    const tokenName = textField("Token name", { required: true, class: "w-md" });
    const scopes = textField("Scopes", { value: "repo:read,pr:read", class: "w-lg", helper: "Comma separated, at most your own scopes." });
    fill(box, panel(h("h3", null, `Tokens of ${name}`), t.wrap, page.tokens.length === 0 && empty("No tokens yet."),
      form({ class: "form-row", onSubmit: () => void act(async () => {
        const created = await api.serviceAccounts.createToken({ ctx: ctx(), org: this.org, name, tokenName: tokenName.value, scopes: csv(scopes.value) });
        clear(tokenName);
        await showSecretOnce(`Token "${created.name}" of ${name}`, created.secret ?? "", h("span", null, "Send it as ", mono("Authorization: Bearer"), " on ", mono("/csil/v1/rpc"), ", or ", mono(`vcsstar login ${location.origin} --token-stdin`), "."));
      }, { after: () => void this.loadAccounts() }) }, tokenName, scopes, button("Create token", { type: "submit" })),
    ));
  }
}

// Keep `meta` imported for pages that extend this one later.
void meta;
