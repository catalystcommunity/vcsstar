// Pieces every page shares: badges, tables, sections, meta rows, feedback,
// notices, confirmations, and the one-shot token dialog.
import { askConfirm } from "@catalystcommunity/containers";
import { showNotice } from "@catalystcommunity/menus";
import { errorText, when as whenText } from "./api";
import { button, fill, h, link, type Child } from "./dom";

export type Tone = "ok" | "warn" | "bad" | "info" | "neutral";

/** A small labelled state. The word carries the meaning; the colour helps. */
export function badge(text: Child, tone: Tone = "neutral"): HTMLSpanElement {
  return h("span", { class: "badge", "data-tone": tone }, text);
}

export function mono(text: Child, cls = ""): HTMLSpanElement {
  return h("span", { class: `mono ${cls}`.trim() }, text);
}

/** A row of short facts with space between them, no separators. */
export function meta(...items: Child[]): HTMLDivElement {
  return h("div", { class: "meta" }, ...items.filter((x) => x !== null && x !== undefined && x !== false && x !== "").map((x) => h("span", null, x)));
}

/** A page section with a heading and an optional lead sentence. */
export function section(title: string, lead: Child, ...children: Child[]): HTMLElement {
  return h("section", { class: "section" }, h("div", { class: "section-head" }, h("h2", null, title), lead && h("p", { class: "lead" }, lead)), ...children);
}

/** A quiet surface for a form or a group of actions. */
export function panel(...children: Child[]): HTMLDivElement {
  return h("div", { class: "panel" }, ...children);
}

/** A table. `rows` are `<tr>` elements; the body is returned so the page
 * can fill it again after a load. A header with text sorts the rows by
 * its column when clicked (again: the other direction). The key of a
 * cell is its `data-sort` attribute (see `sortKey`), else its text.
 * Numbers compare as numbers; empty keys go last in both directions.
 * The sort stays when the page fills the body again. */
export function table(headers: Child[] | null, cls = ""): { wrap: HTMLDivElement; body: HTMLTableSectionElement } {
  const body = h("tbody");
  let sorted: { col: number; desc: boolean } | null = null;
  const ths: HTMLTableCellElement[] = [];
  const apply = () => {
    if (!sorted) return;
    const { col, desc } = sorted;
    const rows = Array.from(body.rows).map((row, i) => ({ row, i, key: keyOf(row.cells[col]) }));
    rows.sort((a, b) => {
      if (a.key === null || b.key === null) return a.key === b.key ? a.i - b.i : a.key === null ? 1 : -1;
      const c = compareKeys(a.key, b.key);
      return (desc ? -c : c) || a.i - b.i;
    });
    body.append(...rows.map((r) => r.row));
    observer.takeRecords(); // our own moves are not a refill
  };
  const observer = new MutationObserver(apply);
  observer.observe(body, { childList: true });
  const head = headers ? h("thead", null, h("tr", null, ...headers.map((x, col) => {
    const label = typeof x === "string" ? x : "";
    const th = label ? h("th", { scope: "col" }) : h("th", { scope: "col" }, x);
    ths.push(th);
    if (!label) return th;
    const sort = h("button", { type: "button", class: "sort-head" }, label);
    sort.addEventListener("click", () => {
      // Dates and numbers start with the largest (newest) first.
      const first = Array.from(body.rows).map((r) => keyOf(r.cells[col])).find((k) => k !== null);
      const numeric = first !== undefined && first !== null && isNumber(first);
      sorted = sorted?.col === col ? { col, desc: !sorted.desc } : { col, desc: numeric };
      ths.forEach((t, i) => (i === col ? t.setAttribute("aria-sort", sorted!.desc ? "descending" : "ascending") : t.removeAttribute("aria-sort")));
      apply();
    });
    th.append(sort);
    return th;
  }))) : null;
  const wrap = h("div", { class: `table-wrap ${cls}`.trim() }, h("table", null, head, body));
  return { wrap, body };
}

/** Give a cell the key it sorts by (a time in ms, a number, or text);
 * null or undefined sorts last. Returns the cell. */
export function sortKey<T extends HTMLElement>(cell: T, key: number | string | null | undefined): T {
  cell.dataset.sort = key === null || key === undefined ? "" : String(key);
  return cell;
}

/** A table cell that shows a time ("3 h ago") and sorts by it. */
export function timeCell(ms: number | null | undefined, missing = "", ...rest: Child[]): HTMLTableCellElement {
  return sortKey(td("muted", ms ? whenText(ms) : missing, ...rest), ms || null);
}

function keyOf(cell: HTMLTableCellElement | undefined): string | null {
  if (!cell) return null;
  const marked = cell.dataset.sort ?? cell.querySelector<HTMLElement>("[data-sort]")?.dataset.sort;
  const key = (marked ?? cell.textContent ?? "").trim();
  return key === "" ? null : key;
}

function isNumber(s: string): boolean {
  return /^-?\d+(\.\d+)?$/.test(s);
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function compareKeys(a: string, b: string): number {
  if (isNumber(a) && isNumber(b)) return Number(a) - Number(b);
  return collator.compare(a, b);
}

export function tr(...cells: Child[]): HTMLTableRowElement {
  return h("tr", null, ...cells.map((c) => (c instanceof HTMLTableCellElement ? c : h("td", null, c))));
}

export function td(cls: string, ...children: Child[]): HTMLTableCellElement {
  return h("td", { class: cls || undefined }, ...children);
}

/** A status line while something loads. */
export function loading(text = "Loading"): HTMLParagraphElement {
  return h("p", { class: "status", role: "status" }, text);
}

/** An empty state that says what to do next. */
export function empty(text: Child, ...actions: Child[]): HTMLDivElement {
  return h("div", { class: "empty" }, h("p", null, text), ...actions);
}

/** One place under a section where a problem shows. */
export class Feedback {
  readonly element = h("div", { class: "feedback" });
  error(error: unknown): void {
    fill(this.element, h("p", { class: "problem", role: "alert" }, typeof error === "string" ? error : errorText(error)));
  }
  clear(): void {
    fill(this.element);
  }
}

/** Run an action. A problem goes to the notice area (it stays until
 * dismissed); `done` is a short confirmation that goes away by itself. */
export async function act(work: () => Promise<unknown>, options: { done?: string; after?: () => void; feedback?: Feedback } = {}): Promise<boolean> {
  options.feedback?.clear();
  try {
    await work();
  } catch (error) {
    if (options.feedback) options.feedback.error(error);
    else showNotice({ text: errorText(error), kind: "error" });
    return false;
  }
  if (options.done) showNotice({ text: options.done, kind: "success" });
  options.after?.();
  return true;
}

/** Ask before an action that cannot be undone. */
export function confirmDanger(title: string, text: string, okLabel: string): Promise<boolean> {
  return askConfirm({ title, text, okLabel, kind: "warning", danger: true });
}

/** Show a secret one time, with a Copy button. */
export async function showSecretOnce(title: string, secret: string, hint: Child): Promise<void> {
  const code = h("pre", { class: "code secret" }, secret);
  const copy = button("Copy", { slot: "buttons", "data-keep-open": true, onClick: async () => { try { await navigator.clipboard.writeText(secret); showNotice({ text: "Copied.", kind: "success" }); } catch { showNotice({ text: "The browser refused the clipboard. Select the text and copy it.", kind: "warning" }); } } } as never);
  const done = button("Done", { slot: "buttons", value: "done", "data-default": true, variant: "primary" } as never);
  const dialog = h("dialog-window", { label: title, kind: "info", "no-close": true }, h("p", null, "Copy it now. It is not shown again."), code, h("p", { class: "hint" }, hint), copy, done) as ZestDialog;
  document.body.append(dialog);
  try {
    await dialog.showModal();
  } finally {
    dialog.remove();
  }
}

/** The head of a page: a breadcrumb line, a title, and facts under it. */
export function pageHead(crumbs: Array<[string, string] | string>, title: Child, ...rest: Child[]): HTMLElement {
  const trail = crumbs.length
    ? h("nav", { class: "crumbs", "aria-label": "You are here" }, ...crumbs.flatMap((c, i) => [i > 0 && h("span", { class: "sep", "aria-hidden": "true" }, "/"), typeof c === "string" ? h("span", null, c) : link(c[1], c[0])]))
    : null;
  return h("header", { class: "page-head" }, trail, h("h1", null, title), ...rest);
}

export function refShort(name: string): string {
  return name.replace(/^refs\/(heads|tags)\//, "");
}

export function firstLine(text: string): string {
  return text.split("\n")[0] ?? "";
}

export function isoDate(ms?: number | null): string {
  return ms ? new Date(ms).toISOString().replace("T", " ").slice(0, 16) + " UTC" : "";
}

export function csv(text: string): string[] {
  return text.split(",").map((s) => s.trim()).filter(Boolean);
}
