// A diff: one disclosure per file. Side by side ("split", the default) or
// in-line ("unified"), as the viewer's preference says; the choice is a
// toggle on the view and is saved for a signed-in viewer. Lines are
// highlighted for the file's language. A markdown or mermaid file can be
// shown rendered; an image (or other media) shows the old and the new
// version, when the revisions are known.
import { button, h } from "../dom";
import { badge } from "../ui";
import { diffLayout, onDiffLayout, setDiffLayout, type DiffLayout } from "../prefs";
import { drawMermaid, highlightLines, isMarkdown, isMermaid, languageOf, markdown, mediaType, rawUrl, readAll } from "../render";
import { bytesToText } from "../api";

export type DiffFile = { status: string; oldPath?: string; newPath?: string; additions: number; deletions: number; patch?: string };

/** Revisions of the two sides, to show rendered documents and media. */
export interface DiffSides {
  repo: { org: string; name: string };
  base: string;
  head: string;
}

type Row = { kind: "ctx" | "add" | "del" | "hunk"; old?: number; new?: number; text: string };

export function diffView(files: DiffFile[], sides?: DiffSides): HTMLElement {
  if (files.length === 0) return h("p", { class: "hint" }, "No changes.");
  const list = h("div", { class: "diff-list" });
  const split = button("Side by side", { variant: "flat", onClick: () => setDiffLayout("split") });
  const unified = button("In-line", { variant: "flat", onClick: () => setDiffLayout("unified") });
  const toolbar = h("div", { class: "row diff-toolbar", role: "group", "aria-label": "Diff layout" }, split, unified);
  const draw = (layout: DiffLayout) => {
    split.setAttribute("aria-pressed", String(layout === "split"));
    unified.setAttribute("aria-pressed", String(layout === "unified"));
    list.replaceChildren(...files.map((f) => fileView(f, layout, files.length <= 6, sides)));
  };
  draw(diffLayout());
  const root = h("div", { class: "diff-root" }, toolbar, list);
  const stop = onDiffLayout((l) => { if (!root.isConnected) { stop(); return; } draw(l); });
  return root;
}

function fileView(f: DiffFile, layout: DiffLayout, open: boolean, sides?: DiffSides): HTMLElement {
  const path = f.newPath ?? f.oldPath ?? "";
  const body = h("div", { class: "diff-body" });
  const showDiff = () => {
    if (!f.patch) {
      body.replaceChildren(media(f, sides) ?? h("p", { class: "hint" }, f.status === "binary" ? "Binary file." : "No text diff."));
      return;
    }
    const rows = parsePatch(f.patch);
    const table = layout === "split" ? splitTable(rows) : unifiedTable(rows);
    body.replaceChildren(table.element);
    void table.highlight(languageOf(path));
  };
  const renderable = sides && f.status !== "deleted" && (isMarkdown(path) || isMermaid(path));
  const rendered = renderable ? button("Rendered", { variant: "flat", onClick: () => void showRendered() }) : null;
  const source = renderable ? button("Diff", { variant: "flat", onClick: () => { mark(false); showDiff(); } }) : null;
  const mark = (r: boolean) => { rendered?.setAttribute("aria-pressed", String(r)); source?.setAttribute("aria-pressed", String(!r)); };
  const showRendered = async () => {
    drawn = true;
    if (!details.open) details.open = true;
    mark(true);
    body.replaceChildren(h("p", { class: "hint" }, "Loading…"));
    try {
      const { bytes } = await readAll(sides!.repo, sides!.head, path);
      const text = bytesToText(bytes) ?? "";
      if (isMermaid(path)) {
        const holder = h("div", { class: "mermaid-view" });
        body.replaceChildren(holder);
        await drawMermaid(holder, text);
      } else {
        const dir = path.split("/").slice(0, -1).join("/");
        body.replaceChildren(markdown(text, { ...sides!.repo, rev: sides!.head, dir }));
      }
    } catch (error) {
      body.replaceChildren(h("p", { class: "problem" }, String((error as Error).message ?? error)));
    }
  };
  mark(false);
  // A file's diff is drawn (and highlighted) when it is first opened, so
  // a pull request with hundreds of files stays quick.
  let drawn = false;
  const details = h("details", { class: "diff-file", open },
    h("summary", null, badge(f.status), h("span", { class: "mono path" }, f.oldPath && f.newPath && f.oldPath !== f.newPath ? `${f.oldPath} → ${f.newPath}` : path), h("span", { class: "add-count" }, `+${f.additions}`), h("span", { class: "del-count" }, `−${f.deletions}`), renderable && h("span", { class: "row view-toggle" }, source, rendered)),
    body,
  ) as HTMLDetailsElement;
  const firstDraw = () => { if (details.open && !drawn) { drawn = true; showDiff(); } };
  details.addEventListener("toggle", firstDraw);
  firstDraw();
  return details;
}

/** Old and new version of an image, video, audio, or PDF file. */
function media(f: DiffFile, sides?: DiffSides): HTMLElement | null {
  const path = f.newPath ?? f.oldPath ?? "";
  const type = mediaType(path);
  if (!sides || !type) return null;
  const side = (label: string, rev: string, p: string | undefined) => {
    const box = h("figure", { class: "media-side" }, h("figcaption", { class: "hint" }, label));
    if (!p) { box.append(h("p", { class: "hint" }, "none")); return box; }
    void rawUrl(sides.repo, rev, p).then((url) => { if (url) box.append(mediaElement(type, url, p)); else box.append(h("p", { class: "hint" }, "Not readable.")); });
    return box;
  };
  return h("div", { class: "media-diff" },
    side("Before", sides.base, f.status === "added" ? undefined : f.oldPath ?? path),
    side("After", sides.head, f.status === "deleted" ? undefined : f.newPath ?? path));
}

/** The element that shows a media file. SVG is drawn through `<img>` only,
 * so a script in it never runs. */
export function mediaElement(type: string, url: string, path: string): HTMLElement {
  if (type.startsWith("image/")) return h("img", { src: url, alt: path, class: "media" });
  if (type.startsWith("video/")) return h("video", { src: url, controls: true, class: "media" });
  if (type.startsWith("audio/")) return h("audio", { src: url, controls: true });
  // The browser draws a PDF in its own viewer, not as a page of ours.
  if (type === "application/pdf") return h("iframe", { src: url, title: path, class: "media pdf" });
  return h("a", { href: url, download: path.split("/").pop() ?? "file" }, "Download");
}

// --- Patch parsing and the two layouts ---------------------------------------

export function parsePatch(patch: string): Row[] {
  const rows: Row[] = [];
  let o = 0;
  let n = 0;
  let inHunk = false;
  for (const line of patch.split("\n")) {
    const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(line);
    if (m) {
      o = Number(m[1]); n = Number(m[2]); inHunk = true;
      rows.push({ kind: "hunk", text: line });
      continue;
    }
    if (!inHunk) continue; // diff --git, index, ---, +++
    if (line.startsWith("\\")) continue; // "\ No newline at end of file"
    if (line.startsWith("+")) rows.push({ kind: "add", new: n++, text: line.slice(1) });
    else if (line.startsWith("-")) rows.push({ kind: "del", old: o++, text: line.slice(1) });
    else if (line.length > 0 || rows.length > 0) rows.push({ kind: "ctx", old: o++, new: n++, text: line.slice(1) });
  }
  // A trailing empty context line comes from the final newline.
  const last = rows[rows.length - 1];
  if (last && last.kind === "ctx" && last.text === "") rows.pop();
  return rows;
}

type Drawn = { element: HTMLElement; highlight(language: string | undefined): Promise<void> };

/** Cells that hold code, by side, in line order, for highlighting. */
function highlighter(cells: { old: HTMLElement[]; new: HTMLElement[] }, texts: { old: string[]; new: string[] }) {
  return async (language: string | undefined) => {
    for (const side of ["old", "new"] as const) {
      if (texts[side].length === 0) continue;
      try {
        const html = await highlightLines(texts[side], language);
        cells[side].forEach((c, i) => { if (html[i] !== undefined) c.innerHTML = html[i] || " "; });
      } catch {
        // Plain text stays.
      }
    }
  };
}

function code(text: string) { return h("td", { class: "code-cell" }, text || " "); }
function num(n?: number) { return h("td", { class: "line-no" }, n === undefined ? "" : String(n)); }

function unifiedTable(rows: Row[]): Drawn {
  const cells = { old: [] as HTMLElement[], new: [] as HTMLElement[] };
  const texts = { old: [] as string[], new: [] as string[] };
  const body = h("tbody");
  for (const r of rows) {
    if (r.kind === "hunk") { body.append(h("tr", { class: "hunk" }, h("td", { colspan: 3 }, r.text))); continue; }
    const c = code(r.text);
    // Context lines are highlighted as part of the new side.
    if (r.kind === "del") { cells.old.push(c); texts.old.push(r.text); } else { cells.new.push(c); texts.new.push(r.text); }
    body.append(h("tr", { class: r.kind }, num(r.old), num(r.new), c));
  }
  return { element: h("table", { class: "diff-table diff-unified" }, body), highlight: highlighter(cells, texts) };
}

function splitTable(rows: Row[]): Drawn {
  const cells = { old: [] as HTMLElement[], new: [] as HTMLElement[] };
  const texts = { old: [] as string[], new: [] as string[] };
  const body = h("tbody");
  let i = 0;
  while (i < rows.length) {
    const r = rows[i]!;
    if (r.kind === "hunk") { body.append(h("tr", { class: "hunk" }, h("td", { colspan: 4 }, r.text))); i++; continue; }
    if (r.kind === "ctx") {
      const left = code(r.text); const right = code(r.text);
      cells.old.push(left); texts.old.push(r.text); cells.new.push(right); texts.new.push(r.text);
      body.append(h("tr", { class: "ctx" }, num(r.old), left, num(r.new), right));
      i++; continue;
    }
    // A run of deletions and additions: pair them line by line.
    const dels: Row[] = []; const adds: Row[] = [];
    while (i < rows.length && rows[i]!.kind === "del") dels.push(rows[i++]!);
    while (i < rows.length && rows[i]!.kind === "add") adds.push(rows[i++]!);
    for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
      const d = dels[k]; const a = adds[k];
      const left = d ? code(d.text) : h("td", { class: "code-cell blank" });
      const right = a ? code(a.text) : h("td", { class: "code-cell blank" });
      if (d) { cells.old.push(left); texts.old.push(d.text); }
      if (a) { cells.new.push(right); texts.new.push(a.text); }
      body.append(h("tr", null,
        h("td", { class: `line-no${d ? " del" : ""}` }, d ? String(d.old) : ""), d ? (left.classList.add("del"), left) : left,
        h("td", { class: `line-no${a ? " add" : ""}` }, a ? String(a.new) : ""), a ? (right.classList.add("add"), right) : right));
    }
  }
  return { element: h("table", { class: "diff-table diff-split" }, h("colgroup", null, h("col", { class: "no" }), h("col"), h("col", { class: "no" }), h("col")), body), highlight: highlighter(cells, texts) };
}
