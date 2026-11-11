// Webhooks of an organization or a repository: endpoints, their deliveries,
// and the form that adds one. Shared by the Org page and the Repo page.
import { api, ctx, when } from "../api";
import { button, checkBox, comboBox, fill, form, h, show, textButton, textField, clear } from "../dom";
import { act, badge, confirmDanger, csv, empty, Feedback, loading, mono, panel, section, table, td, tr } from "../ui";

export function hooksSection(org: string, repo?: string): HTMLElement {
  const target = { org, repo };
  const feedback = new Feedback();
  const endpoints = table(["Endpoint", "Kind", "Events", "State", ""]);
  const status = h("div");
  const deliveriesBox = h("div");
  let selected: string | null = null;

  async function loadDeliveries() {
    if (!selected) { fill(deliveriesBox); return; }
    const id = selected;
    try {
      const page = await api.hooks.listDeliveries({ ctx: ctx(), endpointId: id, page: { limit: 50 } });
      if (selected !== id) return;
      const t = table(["Event", "State", "Attempts", "Last result", ""]);
      fill(t.body, ...page.deliveries.map((x) => tr(
        td("", x.eventKind, " ", mono(`${x.eventLane}#${x.eventSeq}`, "small muted")),
        td("", badge(x.state, x.state === "delivered" ? "ok" : x.state === "dead" ? "bad" : "warn")),
        String(x.attempts),
        td("small", [x.lastStatus, x.lastError].filter(Boolean).join(" "), x.nextAttemptAt && h("div", { class: "muted" }, `next attempt ${when(x.nextAttemptAt)}`)),
        td("actions", x.state !== "pending" && textButton("Redeliver", () => void act(() => api.hooks.redeliver({ ctx: ctx(), deliveryId: x.id }), { done: "Delivery queued again.", after: () => void loadDeliveries() }))),
      )));
      fill(deliveriesBox, panel(h("h3", null, "Deliveries"), t.wrap, page.deliveries.length === 0 && empty("No deliveries yet.")));
    } catch (error) {
      feedback.error(error);
    }
  }

  let loadedOnce = false;
  async function load() {
    try {
      const page = await api.hooks.listEndpoints({ ctx: ctx(), target });
      loadedOnce = true;
      root.hidden = false;
      fill(endpoints.body, ...page.endpoints.map((e) => tr(
        td("", mono(e.url ?? e.corndogsQueue ?? "", "small"), e.repo && h("div", { class: "hint" }, `repository ${e.repo}`)),
        e.kind,
        td("small", e.eventFilter.length ? e.eventFilter.join(", ") : "all events"),
        td("", badge(e.active ? "active" : "off", e.active ? "ok" : "neutral"), e.unordered ? badge("unordered") : null),
        td("actions",
          textButton(selected === e.id ? "Hide deliveries" : "Deliveries", () => { selected = selected === e.id ? null : e.id; void load(); void loadDeliveries(); }),
          textButton(e.active ? "Disable" : "Enable", () => void act(() => api.hooks.updateEndpoint({ ctx: ctx(), endpointId: e.id, active: !e.active }), { done: e.active ? "Endpoint disabled." : "Endpoint enabled.", after: () => void load() })),
          textButton("Delete", async () => {
            if (await confirmDanger("Delete this endpoint?", "Its delivery history goes with it.", "Delete endpoint")) {
              void act(() => api.hooks.deleteEndpoint({ ctx: ctx(), endpointId: e.id }), { done: "Endpoint deleted.", after: () => { if (selected === e.id) selected = null; void load(); void loadDeliveries(); } });
            }
          }, { danger: true }),
        ),
      )));
      fill(status, page.endpoints.length === 0 && empty("No webhook endpoints yet. Add one below to receive events over HTTPS or in a corndogs queue."));
    } catch (error) {
      fill(status);
      if (!loadedOnce) { root.hidden = true; return; }
      feedback.error(error);
    }
  }

  const kind = comboBox("Kind", [{ value: "https", label: "HTTPS" }, { value: "corndogs", label: "corndogs queue" }], { value: "https" });
  const url = textField("URL", { type: "url", placeholder: "https://…", class: "w-lg" });
  const secret = textField("HMAC secret", { type: "password", autocomplete: "off", helper: "16 characters or more", class: "w-md" });
  const queue = textField("Queue", { class: "w-md" });
  const filter = textField("Event filter", { placeholder: "push, pr_merged", helper: "Comma separated. Empty sends every event.", class: "w-lg" });
  const unordered = checkBox("Unordered delivery");
  const httpsFields = h("div", { class: "form-row" }, url, secret);
  const queueFields = h("div", { class: "form-row" }, queue);
  const syncKind = () => { show(httpsFields, kind.value === "https"); show(queueFields, kind.value === "corndogs"); };
  kind.addEventListener("change", syncKind);
  queueFields.hidden = true;
  const add = form({ class: "form-stack", onSubmit: () => void act(async () => {
    await api.hooks.createEndpoint({
      ctx: ctx(), org, repo, kind: kind.value,
      url: kind.value === "https" ? url.value : undefined,
      corndogsQueue: kind.value === "corndogs" ? queue.value : undefined,
      secret: kind.value === "https" ? secret.value : undefined,
      eventFilter: csv(filter.value),
      unordered: unordered.checked,
    });
    clear(url); clear(secret); clear(queue);
  }, { done: "Endpoint added.", after: () => void load() }) },
    h("div", { class: "form-row" }, kind, filter, unordered),
    httpsFields,
    queueFields,
    button("Add endpoint", { type: "submit", variant: "default" }),
  );

  const root = section("Webhooks", "Each endpoint receives the events of the outbox, in order, with retries.", feedback.element, endpoints.wrap, status, deliveriesBox, panel(h("h3", null, "New endpoint"), add));
  fill(status, loading());
  void load();
  return root;
}
