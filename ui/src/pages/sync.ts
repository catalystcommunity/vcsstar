import { api, ctx, when } from "../api";
import { button, checkBox, comboBox, fill, form, h, textField } from "../dom";
import { act, badge, empty, Feedback, mono, panel, section, showSecretOnce } from "../ui";

export function syncTab(c: { org: string; name: string; can(role: "read" | "write" | "maintain" | "admin"): boolean }) {
  const repo = { org: c.org, name: c.name };
  const feedback = new Feedback();
  const connections = h("div"), policyBox = h("div"), conflicts = h("div"), records = h("div");
  const provider = comboBox("Provider", [{ value: "github", label: "GitHub" }, { value: "gitlab", label: "GitLab" }], { value: "gitlab" });
  const host = textField("Host", { value: "gitlab.com", required: true, helper: "Hostname and optional port, without https://" });
  provider.addEventListener("change", () => { host.value = provider.value === "gitlab" ? "gitlab.com" : "github.com"; });
  const project = textField("Project path", { required: true, placeholder: "group/project" });
  const create = checkBox("Create the remote repository", { checked: false });
  const automatic = checkBox("Configure the webhook automatically", { checked: false });
  const run = (work: () => Promise<unknown>) => void act(work, { feedback, after: () => void load() });
  async function load() {
    try {
      const [list, policy, blocked] = await Promise.all([
        api.mirror.listConnections({ ctx: ctx(), repo }), api.mirror.getPolicy({ ctx: ctx(), repo }), api.mirror.listConflicts({ ctx: ctx(), repo }),
      ]);
      fill(connections, ...list.connections.map(s => panel(
        h("div", { class: "row" }, h("strong", null, `${s.host}/${s.project}`), badge(s.paused ? "paused" : "active"), badge(`${s.pending} pending`), badge(`${s.blocked} blocked`), badge(`${s.skipped} omitted`)),
        h("p", null, `Last complete pass: ${when(s.lastSuccessAt) || "never"}. Webhook: ${s.automaticWebhook ? "automatic" : "manual"}.`),
        s.lastError && h("p", { class: "problem", role: "alert" }, s.lastError),
        h("div", { class: "row" },
          c.can("maintain") && button("Sync now", { onClick: () => run(() => api.mirror.syncNow({ ctx: ctx(), repo, connectionId: s.id })) }),
          c.can("admin") && button(s.paused ? "Resume" : "Pause", { onClick: () => run(() => api.mirror.updateConnection({ ctx: ctx(), repo, connectionId: s.id, paused: !s.paused })) }),
          c.can("admin") && button("Webhook setup", { onClick: () => run(async () => {
            const secret = await api.mirror.webhookSecret({ ctx: ctx(), repo, connectionId: s.id });
            await showSecretOnce("Webhook secret", secret.secret, h("div", null, h("p", null, "Create a webhook at the destination. Use this URL and secret. Select push, request, comment, review, and status events that the provider supports."), mono(secret.url)));
          }) }),
          c.can("admin") && button(s.automaticWebhook ? "Use manual webhook setup" : "Set up webhook automatically", { onClick: () => run(() => api.mirror.updateConnection({ ctx: ctx(), repo, connectionId: s.id, automaticWebhook: !s.automaticWebhook })) }),
        ),
      )), list.connections.length === 0 && empty("No remote connections. Add a connection to start sync."));
      if (list.connections.length === 0) {
        const old = await api.mirror.status({ ctx: ctx(), repo });
        if (old.importState && old.importState !== "done") {
          connections.append(panel(`Repository import: ${old.importState}`, old.lastError && h("p", { class: "problem" }, old.lastError)));
        }
      }
      if (c.can("admin") && !policyBox.childElementCount) {
        const source = comboBox("Conflict policy", [{ value: "resolver", label: "Require a resolver" }, { value: "local", label: "Prefer VCSStar" }, ...list.connections.map(s => ({ value: s.id, label: `Prefer ${s.host}/${s.project}` }))], { value: policy.preferredSource ?? "resolver" });
        const identity = textField("Resolver identity (user_id@domain)", { value: policy.resolverIdentity ?? "" });
        const team = textField("Resolver team ID", { value: policy.resolverTeam ?? "" });
        fill(policyBox, section("Conflict policy", "Only the affected item stops. Other items continue. Select a preferred source, or set a resolver identity or team.",
          form({ onSubmit: () => run(() => api.mirror.setPolicy({ ctx: ctx(), repo, policy: source.value === "resolver" ? { resolverIdentity: identity.value || undefined, resolverTeam: team.value || undefined } : { preferredSource: source.value } })) }, source, identity, team, button("Save policy", { type: "submit" }))));
      }
      fill(conflicts, section("Conflicts", "The assigned resolver can select a version. The selected version will be sent to the other connections.",
        ...blocked.conflicts.map(f => panel(mono(f.objectKey), h("details", null, h("summary", null, "Compare versions"), h("h4", null, "VCSStar"), h("pre", null, f.localJson), h("h4", null, "Remote"), h("pre", null, f.remoteJson)),
          button("Keep VCSStar version", { onClick: () => run(() => api.mirror.resolveConflict({ ctx: ctx(), repo, conflictId: f.id, choice: "local" })) }),
          button("Use remote version", { onClick: () => run(() => api.mirror.resolveConflict({ ctx: ctx(), repo, conflictId: f.id, choice: "remote" })) }))),
        blocked.conflicts.length === 0 && empty("No conflicts.")));
    } catch (error) { feedback.error(error); }
  }
  const add = c.can("admin") && section("Add a connection", "Connect an existing repository, or request a new one. Set credentials on the organization page. Manual webhook setup is the default. VCSStar reports missing permissions; a repository administrator must grant them or complete setup.",
    form({ onSubmit: () => run(async () => {
      await api.mirror.connect({ ctx: ctx(), repo, provider: provider.value, host: host.value, project: project.value, createRemote: create.checked, automaticWebhook: automatic.checked });
      fill(policyBox);
    }) }, provider, host, project, create, automatic, button("Connect", { type: "submit" })));
  async function loadRecords(cursor?: string) {
    try {
      const page = await api.mirror.listSyncRecords({ ctx: ctx(), repo, page: { cursor, limit: 50 } });
      fill(records, ...page.records.map(r => h("details", null, h("summary", null, `${r.objectKey}${r.deleted ? " (deleted)" : ""}`), h("pre", null, r.payloadJson))),
        page.nextCursor && button("Next records", { onClick: () => void loadRecords(page.nextCursor) }));
    } catch (error) { feedback.error(error); }
  }
  void load();
  return { element: h("div", null, feedback.element, section("Remote connections", "Changes can start in VCSStar or any connected repository. Visibility, permissions, and protection rules stay local to each system.", connections), add, policyBox, conflicts,
    section("Stored records", "VCSStar keeps the full record when a destination can store only a summary or link.", button("Load records", { onClick: () => void loadRecords() }), records)), update() {}, refresh() { void load(); } };
}

export function providerLinks() {
  const root = h("div"), list = h("div"), feedback = new Feedback();
  const host = textField("GitLab host", { value: "gitlab.com", required: true });
  const token = textField("Personal access token", { type: "password", autocomplete: "off", required: true });
  async function load() {
    try {
      const result = await api.mirror.listProviderLinks({ ctx: ctx() });
      fill(list, ...result.links.map(l => panel(`${l.login}@${l.host}`, l.brokenReason && h("p", { class: "problem" }, l.brokenReason), button("Disconnect", { onClick: () => void act(() => api.mirror.unlinkProvider({ ctx: ctx(), provider: l.provider, host: l.host }), { feedback, after: () => void load() }) }))));
    } catch (error) { feedback.error(error); }
  }
  fill(root, feedback.element, list, form({ onSubmit: () => void act(async () => {
    await api.mirror.linkProvider({ ctx: ctx(), provider: "gitlab", host: host.value, token: token.value }); token.value = "";
  }, { feedback, after: () => void load() }) }, host, token, button("Link GitLab account", { type: "submit" })));
  void load();
  return section("GitLab accounts", "Use your linked account for remote changes. If that account is not available, VCSStar uses a bot and includes attribution.", root);
}
