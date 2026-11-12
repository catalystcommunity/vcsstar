// Account: who you are, the GitHub connection, SSH keys, and API tokens.
import { api, ctx, when, who } from "../api";
import { button, comboBox, fill, form, h, textButton, textField, clear } from "../dom";
import { Page } from "../page";
import { act, badge, confirmDanger, csv, empty, Feedback, isoDate, loading, meta, mono, pageHead, panel, section, showSecretOnce, table, td, tr } from "../ui";

export class MePage extends Page {
  title = "Account";
  private readonly feedback = new Feedback();
  private readonly github = h("div");
  private readonly keys = table(["Key", "Usage", "Status", "Attested until", ""]);
  private readonly keysStatus = h("div");
  private readonly tokens = table(["Name", "Scopes", "Label", "Created", "Expires", ""]);
  private readonly tokensStatus = h("div");
  private readonly rolesBox = h("div");

  protected build() {
    const me = this.context.session;
    if (!me) {
      return [pageHead([], "Account"), empty("Sign in to manage your keys and tokens.", h("a", { class: "button-link", href: "/auth/login?return_to=/me", rel: "external" }, "Sign in"))];
    }
    const githubError = new URLSearchParams(location.search).get("github_error");
    if (githubError) this.feedback.error(`GitHub did not connect: ${githubError}`);
    return [
      pageHead([], who(me.identity), meta(me.identity.displayName, h("span", null, "identity ", mono(`${me.identity.userId}@${me.identity.domain}`)), `signed in with ${me.credentialKind.replace("_", " ")}`, `scopes ${me.scopes.join(", ")}`)),
      this.feedback.element,
      section("GitHub", "With a connected GitHub account, what you do here on a mirrored repository is sent to GitHub as you, and what you do on GitHub is attributed to you here.", this.github),
      section("SSH keys", h("span", null, "Attested keys come from ", mono("vcsstar keys enroll"), " (LinkKeys application keys). An uploaded key works for access and is marked unattested."), this.keys.wrap, this.keysStatus, this.addKeyForm()),
      section("API tokens", "A token acts as you with the scopes you give it. Use it with the CLI or as the git HTTPS password.", this.tokens.wrap, this.tokensStatus, this.newTokenForm()),
      // Only an instance admin may see or change instance roles.
      me.instanceRole === "admin" && this.rolesBox,
    ];
  }

  protected async start() {
    if (!this.context.session) return;
    fill(this.github, loading());
    fill(this.keysStatus, loading());
    fill(this.tokensStatus, loading());
    await Promise.all([this.loadLink(), this.loadKeys(), this.loadTokens(), this.context.session.instanceRole === "admin" ? this.loadRoles() : Promise.resolve()]);
  }

  // --- GitHub -------------------------------------------------------------

  private async loadLink() {
    try {
      const l = await api.auth.githubLinkStatus({ ctx: ctx() });
      if (!this.isActive()) return;
      const connectHref = l.startUrl ? `${l.startUrl}?return_to=/me` : null;
      if (!l.linked) {
        const pat = textField("Personal access token", { type: "password", autocomplete: "off", placeholder: "github_pat_…", class: "w-lg" });
        fill(this.github, panel(
          h("div", { class: "row" }, badge("not connected"), connectHref && h("a", { class: "button-link primary", href: connectHref, rel: "external" }, l.appSlug ? `Connect GitHub (${l.appSlug})` : "Connect GitHub")),
          form({ class: "form-row", onSubmit: () => void act(() => api.auth.setGithubToken({ ctx: ctx(), token: pat.value }), { done: "GitHub token saved.", after: () => void this.loadLink() }) }, pat, button("Use this token", { type: "submit", variant: "default" })),
        ));
        return;
      }
      fill(this.github, panel(
        h("div", { class: "row" }, badge("connected", "ok"), h("span", null, "as ", h("strong", null, l.login)), h("span", { class: "muted" }, `${l.kind}, since ${when(l.linkedAt)}`), l.expiresAt && h("span", { class: "muted" }, `token renews before ${isoDate(l.expiresAt)}`)),
        l.brokenReason && h("p", { class: "problem", role: "alert" }, `GitHub refused the stored token: ${l.brokenReason}. Connect again.`),
        h("div", { class: "row" }, connectHref && h("a", { class: "button-link", href: connectHref, rel: "external" }, "Connect again"), textButton("Disconnect", async () => {
          if (await confirmDanger("Disconnect GitHub?", "Your actions on mirrored repositories are then sent through the instance App or the organization credential, not as you.", "Disconnect")) {
            void act(() => api.auth.unlinkGithub({ ctx: ctx() }), { done: "GitHub disconnected.", after: () => void this.loadLink() });
          }
        }, { danger: true })),
      ));
    } catch (error) {
      if (this.isActive()) { fill(this.github); this.feedback.error(error); }
    }
  }

  // --- SSH keys -----------------------------------------------------------

  private async loadKeys() {
    try {
      const page = await api.auth.listKeys({ ctx: ctx() });
      if (!this.isActive()) return;
      fill(this.keys.body, ...page.keys.map((k) => tr(
        td("", mono(k.publicKey.split(" ").slice(0, 2).join(" ").slice(0, 56) + "…", "small")),
        k.usage,
        td("", badge(k.status, k.status === "attested" ? "ok" : k.status === "revoked" ? "bad" : "warn")),
        td("muted", k.attestedUntil ? isoDate(k.attestedUntil) : "no attestation"),
        td("actions", k.status !== "revoked" && textButton("Revoke", async () => {
          if (await confirmDanger("Revoke this key?", "Git over SSH with this key stops at once.", "Revoke key")) {
            void act(() => api.auth.revokeKey({ ctx: ctx(), keyId: k.keyId }), { done: "Key revoked.", after: () => void this.loadKeys() });
          }
        }, { danger: true })),
      )));
      fill(this.keysStatus, page.keys.length === 0 && empty("No SSH keys yet."));
    } catch (error) {
      if (this.isActive()) { fill(this.keysStatus); this.feedback.error(error); }
    }
  }

  private addKeyForm() {
    const key = textField("Public key", { required: true, placeholder: "ssh-ed25519 AAAA… comment", class: "w-xl", autocapitalize: "none", spellcheck: false });
    const usage = comboBox("Usage", [{ value: "auth", label: "Access" }, { value: "signing", label: "Signing" }], { value: "auth" });
    return form({ class: "form-row", onSubmit: () => void act(async () => {
      await api.auth.registerKey({ ctx: ctx(), publicKey: key.value, usage: usage.value });
      clear(key);
    }, { done: "Key added.", after: () => void this.loadKeys() }) }, key, usage, button("Add key", { type: "submit", variant: "default" }));
  }

  // --- API tokens ---------------------------------------------------------

  private async loadTokens() {
    try {
      const page = await api.auth.listTokens({ ctx: ctx() });
      if (!this.isActive()) return;
      fill(this.tokens.body, ...page.tokens.map((t) => tr(
        td("", h("strong", null, t.name)),
        td("small", t.scopes.join(", ")),
        td("muted small", t.clientLabel),
        td("muted", when(t.createdAt)),
        td("muted", t.expiresAt ? when(t.expiresAt) : "never"),
        td("actions", textButton("Revoke", async () => {
          if (await confirmDanger(`Revoke the token "${t.name}"?`, "Anything that uses it stops at once.", "Revoke token")) {
            void act(() => api.auth.revokeToken({ ctx: ctx(), tokenId: t.id }), { done: "Token revoked.", after: () => void this.loadTokens() });
          }
        }, { danger: true })),
      )));
      fill(this.tokensStatus, page.tokens.length === 0 && empty("No API tokens yet."));
    } catch (error) {
      if (this.isActive()) { fill(this.tokensStatus); this.feedback.error(error); }
    }
  }

  private newTokenForm() {
    const name = textField("Token name", { required: true, class: "w-md" });
    const scopes = textField("Scopes", { value: "repo:write,pr:write,pr:review,statuses:write,org:read", class: "w-xl", helper: "Comma separated. A token can have at most your own scopes." });
    return form({ class: "form-row", onSubmit: () => void act(async () => {
      const t = await api.auth.createToken({ ctx: ctx(), name: name.value, scopes: csv(scopes.value) });
      clear(name);
      await showSecretOnce(`Token "${t.name}"`, t.secret ?? "", h("span", null, "Use it with ", mono(`vcsstar login ${location.origin} --token-stdin`), " or as the git HTTPS password."));
    }, { after: () => void this.loadTokens() }) }, name, scopes, button("Create token", { type: "submit" }));
  }

  // --- Instance roles (instance admins) -------------------------------------

  private async loadRoles() {
    const feedback = new Feedback();
    let list;
    try {
      list = await api.instance.listRoles({ ctx: ctx() });
    } catch (error) {
      if (this.isActive()) { fill(this.rolesBox, section("Instance roles", null, feedback.element)); feedback.error(error); }
      return;
    }
    if (!this.isActive()) return;
    const t = table(["Identity", "Role", "Source", "Granted", ""]);
    fill(t.body,
      ...list.configuredAdmins.map((k) => tr(td("", mono(k)), "admin", "configuration", td("muted", "INSTANCE_ADMINS"), td(""))),
      ...list.grants.map((g) => {
        const key = `${g.identity.userId}@${g.identity.domain}`;
        return tr(
          td("", who(g.identity), h("div", { class: "hint mono" }, key)),
          g.role,
          g.source,
          td("muted", when(g.grantedAt), g.grantedBy ? ` by ${who(g.grantedBy)}` : ""),
          td("actions", textButton("Remove", async () => {
            if (await confirmDanger(`Remove the ${g.role} role of ${who(g.identity)}?`, "They keep their organization and repository roles.", "Remove role")) {
              void act(() => api.instance.removeRole({ ctx: ctx(), identity: key }), { feedback, done: "Role removed.", after: () => void this.loadRoles() });
            }
          }, { danger: true })),
        );
      }));
    const identity = textField("Identity", { required: true, placeholder: "user_id@domain", class: "w-lg", autocapitalize: "none", spellcheck: false, helper: "A person appears after one sign-in." });
    const role = comboBox("Role", [
      { value: "viewer", label: "Viewer", description: "Reads every repository, private ones too" },
      { value: "admin", label: "Admin", description: "Manages everything on this instance" },
    ], { value: "viewer" });
    fill(this.rolesBox, section("Instance roles", "Roles on the whole instance. Admins from the configuration cannot be removed here.", feedback.element, t.wrap,
      form({ class: "form-row", onSubmit: () => void act(async () => { await api.instance.setRole({ ctx: ctx(), identity: identity.value, role: role.value }); clear(identity); }, { feedback, done: "Role set.", after: () => void this.loadRoles() }) }, identity, role, button("Set role", { type: "submit", variant: "default" })),
    ));
  }
}
