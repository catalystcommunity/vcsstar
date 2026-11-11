// A diff: one disclosure per file, the patch coloured by line kind.
import { h } from "../dom";
import { badge } from "../ui";

export type DiffFile = { status: string; oldPath?: string; newPath?: string; additions: number; deletions: number; patch?: string };

export function diffView(files: DiffFile[]): HTMLElement {
  if (files.length === 0) return h("p", { class: "hint" }, "No changes.");
  return h("div", { class: "diff-list" }, ...files.map((f) =>
    h("details", { class: "diff-file", open: files.length <= 6 },
      h("summary", null, badge(f.status), h("span", { class: "mono path" }, f.newPath ?? f.oldPath ?? ""), h("span", { class: "add-count" }, `+${f.additions}`), h("span", { class: "del-count" }, `−${f.deletions}`)),
      f.patch
        ? h("pre", { class: "diff" }, ...f.patch.split("\n").map((line) => h("span", { class: lineClass(line) }, line || " ")))
        : h("p", { class: "hint" }, "No text diff."),
    ),
  ));
}

function lineClass(line: string): string {
  if (line.startsWith("+") && !line.startsWith("+++")) return "add";
  if (line.startsWith("-") && !line.startsWith("---")) return "del";
  if (line.startsWith("@@")) return "hunk";
  return "";
}
