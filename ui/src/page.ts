// The base class of each page. A page is a Zest light-DOM component: it
// builds its elements one time when it first connects, then changes only
// the parts that change. The app shell makes a new page for each route and
// keeps it while the route stays on the same object.
import { BaseComponent, defineElements } from "@catalystcommunity/base-components";
import { nodes, type Child } from "./dom";
import type { WhoAmI } from "./generated/types.gen.ts";

/** What the app shell gives each page. */
export interface PageContext {
  navigate(path: string): void;
  session: WhoAmI | null;
  /** The path the page shows. The shell updates it through `setPath`. */
  path: string;
  signOut(): Promise<void>;
}

export abstract class Page extends BaseComponent {
  /** Set by the shell before the page connects. */
  context!: PageContext;
  /** The browser title of the page. */
  title = "vcsstar";
  private builtOnce = false;

  connectedCallback(): void {
    super.connectedCallback();
    if (!this.builtOnce) {
      this.builtOnce = true;
      this.replaceChildren(...nodes(this.build()));
      void this.start();
    }
    this.connected();
  }

  /** The page's elements. Runs one time. */
  protected abstract build(): Child;

  /** Work after the first build, for example the first load. After each
   * `await`, check `this.isActive()` before you change the page. */
  protected async start(): Promise<void> {}

  /** Runs on each connect, after the build. Make subscriptions here with
   * `this.own(...)`, because the element removes them when it disconnects. */
  protected connected(): void {}

  /** The route moved inside the same page (a tab, a path). Pages that
   * handle this return true; the shell makes a new page otherwise. */
  setPath(_path: string): boolean {
    return false;
  }
}

/** Define a page class under `tag` (once) and make an instance of it. */
export function createPage<T extends Page>(tag: string, pageClass: new () => T, context: PageContext): T {
  defineElements({ [tag]: pageClass });
  const page = document.createElement(tag) as T;
  page.classList.add("page");
  page.context = context;
  return page;
}
