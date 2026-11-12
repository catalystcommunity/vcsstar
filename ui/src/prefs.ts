// Display preferences. A signed-in viewer's choice is kept on the server
// (`Auth/get-preferences`, `Auth/set-preferences`) and follows them to
// every browser; an anonymous viewer gets the defaults (side by side
// diffs), and a change lasts until the page is reloaded.
import { api, ctx } from "./api";

export type DiffLayout = "split" | "unified";

let layout: DiffLayout = "split";
let signedIn = false;
const listeners = new Set<(l: DiffLayout) => void>();

/** Call after the session is known (the shell does). */
export async function loadPreferences(session: boolean): Promise<void> {
  signedIn = session;
  layout = "split";
  if (!session) return;
  try {
    const p = await api.auth.getPreferences({ ctx: ctx() });
    layout = p.diffLayout === "unified" ? "unified" : "split";
  } catch {
    // Keep the default; the view still works.
  }
}

export function diffLayout(): DiffLayout {
  return layout;
}

export function setDiffLayout(next: DiffLayout): void {
  if (next === layout) return;
  layout = next;
  for (const l of listeners) l(next);
  if (signedIn) void api.auth.setPreferences({ ctx: ctx(), diffLayout: next }).catch(() => undefined);
}

/** Be told when the layout changes; returns the unsubscribe function. */
export function onDiffLayout(fn: (l: DiffLayout) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
