// Rendering of repository content: markdown (markdown-it, sanitized with
// DOMPurify), syntax highlighting (highlight.js with every language it
// has, loaded on first use), and mermaid diagrams (loaded on first use).
// The large libraries are dynamic imports, so a page without code or
// diagrams does not download them.
import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";
import { api, ctx } from "./api";
import { h } from "./dom";

type Hljs = typeof import("highlight.js").default;
let hljsLoad: Promise<Hljs> | undefined;
function hljs(): Promise<Hljs> {
  hljsLoad ??= import("highlight.js").then((m) => m.default);
  return hljsLoad;
}

type Mermaid = typeof import("mermaid").default;
let mermaidLoad: Promise<Mermaid> | undefined;
function mermaid(): Promise<Mermaid> {
  mermaidLoad ??= import("mermaid").then((m) => {
    const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
    // "strict": no scripts and no HTML labels from diagram text.
    m.default.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark ? "dark" : "default" });
    return m.default;
  });
  return mermaidLoad;
}

// --- File kinds ----------------------------------------------------------------

/** highlight.js language names by file name or extension, where the
 * extension alone is not one of its aliases. */
const BY_NAME: Record<string, string> = {
  dockerfile: "dockerfile", containerfile: "dockerfile", makefile: "makefile", gnumakefile: "makefile",
  "cmakelists.txt": "cmake", justfile: "makefile", gemfile: "ruby", rakefile: "ruby", podfile: "ruby",
  "cargo.lock": "toml", "go.mod": "go", "go.sum": "plaintext", ".gitignore": "bash", ".bashrc": "bash",
  ".zshrc": "bash", ".env": "bash", jenkinsfile: "groovy", vagrantfile: "ruby", "build.gradle": "gradle",
};
const BY_EXT: Record<string, string> = {
  rs: "rust", ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript", js: "javascript",
  mjs: "javascript", cjs: "javascript", jsx: "javascript", py: "python", pyi: "python", rb: "ruby",
  go: "go", java: "java", kt: "kotlin", kts: "kotlin", swift: "swift", c: "c", h: "c", cc: "cpp", cpp: "cpp",
  cxx: "cpp", hpp: "cpp", hh: "cpp", cs: "csharp", fs: "fsharp", php: "php", sh: "bash", bash: "bash",
  zsh: "bash", fish: "bash", ps1: "powershell", psm1: "powershell", yml: "yaml", yaml: "yaml", toml: "toml",
  ini: "ini", cfg: "ini", conf: "ini", json: "json", jsonc: "json", json5: "json", xml: "xml", svg: "xml",
  html: "xml", htm: "xml", vue: "xml", css: "css", scss: "scss", sass: "scss", less: "less", sql: "sql",
  md: "markdown", markdown: "markdown", mdx: "markdown", tf: "hcl", hcl: "hcl", nix: "nix", ex: "elixir",
  exs: "elixir", erl: "erlang", hs: "haskell", ml: "ocaml", mli: "ocaml", scala: "scala", clj: "clojure",
  lua: "lua", pl: "perl", pm: "perl", r: "r", dart: "dart", zig: "zig", proto: "protobuf", graphql: "graphql",
  gql: "graphql", diff: "diff", patch: "diff", dockerfile: "dockerfile", cmake: "cmake", vim: "vim",
  csil: "lisp", cddl: "plaintext", tex: "latex", elm: "elm", jl: "julia", groovy: "groovy", gradle: "gradle",
  sol: "solidity", v: "verilog", sv: "verilog", vhd: "vhdl", asm: "x86asm", s: "armasm", bat: "dos", cmd: "dos",
  mmd: "plaintext", mermaid: "plaintext", txt: "plaintext", lock: "plaintext", csv: "plaintext", tsv: "plaintext",
};

function baseName(path: string) { return (path.split("/").pop() ?? path).toLowerCase(); }
function ext(path: string) { const b = baseName(path); const i = b.lastIndexOf("."); return i > 0 ? b.slice(i + 1) : ""; }

/** The highlight.js language for a file, or undefined (detect). */
export function languageOf(path: string): string | undefined {
  return BY_NAME[baseName(path)] ?? BY_EXT[ext(path)] ?? (ext(path) || undefined);
}

export function isMarkdown(path: string) { return ["md", "markdown", "mdx", "mkd"].includes(ext(path)); }
export function isMermaid(path: string) { return ["mmd", "mermaid"].includes(ext(path)); }

/** Media types the browser can show inline, by extension. */
const MEDIA: Record<string, string> = {
  png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", avif: "image/avif",
  bmp: "image/bmp", ico: "image/x-icon", svg: "image/svg+xml", apng: "image/apng",
  mp4: "video/mp4", webm: "video/webm", ogv: "video/ogg", mov: "video/quicktime",
  mp3: "audio/mpeg", ogg: "audio/ogg", oga: "audio/ogg", wav: "audio/wav", flac: "audio/flac", m4a: "audio/mp4", opus: "audio/ogg",
  pdf: "application/pdf", woff: "font/woff", woff2: "font/woff2", ttf: "font/ttf", otf: "font/otf",
};
export function mediaType(path: string): string | undefined { return MEDIA[ext(path)]; }

// --- Highlighting --------------------------------------------------------------

/** Highlighted HTML of a text (escaped by highlight.js). */
export async function highlightHtml(text: string, language: string | undefined): Promise<string> {
  const lib = await hljs();
  if (language && lib.getLanguage(language)) return lib.highlight(text, { language, ignoreIllegals: true }).value;
  if (text.length > 200_000) return escapeHtml(text);
  return lib.highlightAuto(text).value;
}

/** Highlight each line on its own (for diffs, where lines are split). The
 * state of multi-line constructs is carried by highlighting the whole text
 * and splitting the HTML at line ends, closing and reopening spans. */
export async function highlightLines(lines: string[], language: string | undefined): Promise<string[]> {
  const html = await highlightHtml(lines.join("\n"), language);
  return splitHtmlLines(html);
}

function splitHtmlLines(html: string): string[] {
  const out: string[] = [];
  const open: string[] = [];
  let line = "";
  const re = /(<span[^>]*>)|(<\/span>)|(\n)|([^<\n]+|<)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html))) {
    if (m[1]) { open.push(m[1]); line += m[1]; }
    else if (m[2]) { open.pop(); line += m[2]; }
    else if (m[3]) { line += "</span>".repeat(open.length); out.push(line); line = open.join(""); }
    else line += m[4];
  }
  out.push(line + "</span>".repeat(open.length));
  return out;
}

export function escapeHtml(s: string) {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

/** A code block with line numbers, highlighted when the library is in. */
export function codeView(text: string, path: string): HTMLElement {
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const gutter = h("pre", { class: "code-gutter", "aria-hidden": "true" }, lines.map((_, i) => String(i + 1)).join("\n"));
  const body = h("pre", { class: "code hljs" }, h("code", null, lines.join("\n")));
  void highlightHtml(lines.join("\n"), languageOf(path)).then((html) => { (body.firstChild as HTMLElement).innerHTML = html; });
  return h("div", { class: "code-view" }, gutter, body);
}

// --- Markdown ------------------------------------------------------------------

/** Where relative links and images in a document point. */
export interface DocBase {
  org: string;
  name: string;
  rev: string;
  /** Directory of the document inside the repository ("" for the root). */
  dir: string;
}

const md = new MarkdownIt({ html: true, linkify: true, typographer: false });

/** Rendered markdown. HTML in the text is allowed and sanitized.
 * Fenced code is highlighted; ```mermaid fences become diagrams. Relative
 * links go to the code view; relative images load through `Repos/blob`. */
export function markdown(text: string, base?: DocBase): HTMLElement {
  const el = h("div", { class: "markdown" });
  el.innerHTML = DOMPurify.sanitize(md.render(text), { USE_PROFILES: { html: true }, FORBID_TAGS: ["style", "form", "input", "button"], ADD_ATTR: ["target"] });
  void enhance(el, base);
  return el;
}

async function enhance(el: HTMLElement, base?: DocBase) {
  for (const a of Array.from(el.querySelectorAll<HTMLAnchorElement>("a[href]"))) {
    const href = a.getAttribute("href")!;
    if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("//")) { a.rel = "noopener noreferrer external"; a.target = "_blank"; continue; }
    if (href.startsWith("#") || !base) continue;
    const [path, hash] = href.split("#");
    const target = resolvePath(base.dir, path!);
    a.href = `/${base.org}/${base.name}/code/${encodeURIComponent(base.rev)}/${target}${hash ? `#${hash}` : ""}`;
    a.dataset.nav = "";
  }
  if (base) {
    for (const img of Array.from(el.querySelectorAll<HTMLImageElement>("img[src]"))) {
      const src = img.getAttribute("src")!;
      if (/^[a-z][a-z0-9+.-]*:/i.test(src) || src.startsWith("//")) continue;
      const path = resolvePath(base.dir, src.split("?")[0]!);
      img.removeAttribute("src");
      void rawUrl({ org: base.org, name: base.name }, base.rev, path).then((url) => { if (url) img.src = url; });
    }
  }
  const blocks = Array.from(el.querySelectorAll<HTMLElement>("pre > code"));
  if (blocks.length === 0) return;
  const diagrams = blocks.filter((c) => c.classList.contains("language-mermaid"));
  for (const code of blocks.filter((c) => !diagrams.includes(c))) {
    const lang = Array.from(code.classList).find((c) => c.startsWith("language-"))?.slice(9);
    code.innerHTML = await highlightHtml(code.textContent ?? "", lang);
    code.classList.add("hljs");
  }
  for (const code of diagrams) {
    const holder = h("div", { class: "mermaid-view" });
    code.parentElement!.replaceWith(holder);
    await drawMermaid(holder, code.textContent ?? "");
  }
}

let diagramSeq = 0;
/** Draw a mermaid diagram into `holder`; on a syntax error show the text. */
export async function drawMermaid(holder: HTMLElement, source: string) {
  try {
    const m = await mermaid();
    const { svg } = await m.render(`mmd-${++diagramSeq}`, source);
    holder.innerHTML = DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true } });
  } catch (error) {
    holder.replaceChildren(h("pre", { class: "code" }, source), h("p", { class: "problem" }, `Diagram error: ${(error as Error).message ?? error}`));
  }
}

function resolvePath(dir: string, rel: string): string {
  const parts = rel.startsWith("/") ? [] : dir.split("/").filter(Boolean);
  for (const seg of rel.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") parts.pop(); else parts.push(decodeURIComponent(seg));
  }
  return parts.map(encodeURIComponent).join("/");
}

// --- Raw content -----------------------------------------------------------------

/** Largest file the browser view loads (in pieces of CHUNK through CSIL). */
export const RAW_LIMIT = 64 * 1024 * 1024;
const CHUNK = 2 * 1024 * 1024;

/** The whole file as bytes, read through `Repos/blob` in pieces. */
export async function readAll(repo: { org: string; name: string }, rev: string, path: string): Promise<{ bytes: Uint8Array; size: number; binary: boolean }> {
  const first = await api.repos.blob({ ctx: ctx(), repo, rev, path, limit: CHUNK });
  if (first.size > RAW_LIMIT) throw new Error(`The file is ${first.size} bytes; the browser view loads files up to ${RAW_LIMIT} bytes. Clone the repository to get it.`);
  const out = new Uint8Array(first.size);
  out.set(first.content, 0);
  let offset = first.content.length;
  while (offset < first.size) {
    const part = await api.repos.blob({ ctx: ctx(), repo, rev, path, offset, limit: CHUNK });
    if (part.content.length === 0) break;
    out.set(part.content, offset);
    offset += part.content.length;
  }
  return { bytes: out, size: first.size, binary: first.binary };
}

const rawCache = new Map<string, Promise<string | null>>();
/** An object URL with the file's bytes and its media type (images in
 * documents, raw views). `null` when it cannot be read. */
export function rawUrl(repo: { org: string; name: string }, rev: string, path: string): Promise<string | null> {
  const key = `${repo.org}/${repo.name}@${rev}:${path}`;
  let hit = rawCache.get(key);
  if (!hit) {
    hit = readAll(repo, rev, path)
      .then(({ bytes }) => URL.createObjectURL(new Blob([bytes as BlobPart], { type: mediaType(path) ?? "application/octet-stream" })))
      .catch(() => null);
    rawCache.set(key, hit);
  }
  return hit;
}
