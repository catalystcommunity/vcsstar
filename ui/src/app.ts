// The app shell: header, navigation, footer, session, and routing.
// SimpleRouter owns the browser history. The shell reads the path and
// picks the page, and keeps a page while the route stays on its object.
import { loadPreferences } from "./prefs";
import SimpleRouter from "@catalystcommunity/simple-router";
import { api, ctx, isUnauthenticated, who } from "./api";
import { fill, h, link } from "./dom";
import type { Page, PageContext } from "./page";
import { createPage } from "./page";
import type { WhoAmI } from "./generated/types.gen.ts";
import { HomePage } from "./pages/home";
import { MePage } from "./pages/me";
import { OrgPage } from "./pages/org";
import { RepoPage } from "./pages/repo";
import { PullPage } from "./pages/pull";

type Route = { key: string; make: () => Page };

export class VcsstarApp {
  private session: WhoAmI | null = null;
  private readonly router = new SimpleRouter();
  private readonly outlet = h("div", { id: "main-content", class: "outlet", tabindex: "-1" });
  private readonly nav = h("nav", { class: "site-nav", "aria-label": "Main" });
  private readonly account = h("div", { class: "account" });
  private readonly footer = h("footer", { class: "site-footer" });
  private current?: { key: string; page: Page; session: string };
  private ready = false;

  constructor(private readonly root: HTMLElement) {}

  async start(): Promise<void> {
    const brand = link("/", h("img", { class: "brand-mark", src: "/logo.svg", alt: "" }), h("span", { class: "brand-name" }, "vcsstar"));
    brand.classList.add("brand");
    this.root.replaceChildren(
      h("a", { class: "skip-link", href: "#main-content", on: { click: (e: MouseEvent) => { e.preventDefault(); this.outlet.focus(); } } }, "Skip to content"),
      h("header", { class: "site-header" }, brand, this.nav, this.account),
      this.outlet,
      this.footer,
    );
    document.addEventListener("click", this.onClick);
    this.router.eventSender = () => undefined;
    this.router.registerRoute("/", () => this.show());
    this.router.registerRoute("/me", () => this.show());
    this.router.registerRoute("/notfound", () => this.show(), [], true);
    await this.refreshSession();
    this.ready = true;
    this.renderNav();
    this.show();
    void this.loadFooter();
  }

  /** Go to `target`. A path the SPA does not own loads as a normal page. */
  navigate = (target: string): void => {
    const url = new URL(target, location.origin);
    if (url.origin !== location.origin) { location.assign(url); return; }
    if (url.pathname.startsWith("/auth/") || url.pathname.startsWith("/hooks/") || url.pathname.includes(".git")) { location.assign(url); return; }
    this.router.changePage(url.pathname + url.search + url.hash);
  };

  private readonly onClick = (event: MouseEvent): void => {
    const anchor = (event.target as Element).closest<HTMLAnchorElement>("a[data-nav]");
    if (!anchor || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    this.navigate(anchor.href);
  };

  private async refreshSession(): Promise<void> {
    try {
      this.session = await api.auth.whoami({ ctx: ctx() });
    } catch (error) {
      this.session = null;
      if (!isUnauthenticated(error)) console.warn("whoami failed", error);
    }
    await loadPreferences(Boolean(this.session));
  }

  private signOut = async (): Promise<void> => {
    await fetch("/auth/logout", { method: "POST", credentials: "same-origin" });
    this.session = null;
    this.current = undefined;
    this.renderNav();
    this.navigate("/");
  };

  private async loadFooter(): Promise<void> {
    try {
      const info = await api.meta.serverInfo({ ctx: ctx() });
      fill(this.footer, h("span", null, info.instanceIdentity), h("span", null, `vcsstar ${info.version}`), h("span", null, `contract ${info.contractVersion}`));
    } catch {
      fill(this.footer, h("span", null, "vcsstar"));
    }
  }

  private renderNav(): void {
    const path = location.pathname;
    const item = (href: string, label: string, current: boolean) => h("a", { href, "data-nav": true, "aria-current": current ? "page" : null }, label);
    fill(this.nav, item("/", "Repositories", path === "/" || (!path.startsWith("/me") && path !== "/")), this.session && item("/me", "Account", path.startsWith("/me")));
    if (this.session) {
      const menu = h("menu-button", { label: "Account menu", class: "account-button", placement: "bottom-end" }, h("span", { class: "account-name" }, who(this.session.identity))) as ZestMenuButton;
      menu.items = [
        { id: "account", label: "Account", mnemonic: false, run: () => this.navigate("/me") },
        { separator: true },
        { id: "signout", label: "Sign out", mnemonic: false, run: () => void this.signOut() },
      ];
      fill(this.account, menu);
    } else {
      fill(this.account, h("a", { class: "sign-in", href: `/auth/login?return_to=${encodeURIComponent(location.pathname)}`, rel: "external" }, "Sign in"));
    }
  }

  /** Show the page for the current path. The same page object stays when
   * the path still belongs to it (a repository tab, for example). */
  private show(): void {
    // The router also fires on window load, before the session is known.
    if (!this.ready) return;
    const path = location.pathname;
    const route = this.routeFor(path);
    const session = this.session ? `${this.session.identity.userId}@${this.session.identity.domain}` : "";
    if (this.current && this.current.key === route.key && this.current.session === session && this.current.page.setPath(path)) {
      document.title = `${this.current.page.title} - vcsstar`;
      this.renderNav();
      return;
    }
    const page = route.make();
    this.current = { key: route.key, page, session };
    this.outlet.replaceChildren(page);
    document.title = `${page.title} - vcsstar`;
    this.renderNav();
    window.scrollTo(0, 0);
    requestAnimationFrame(() => {
      const active = document.activeElement;
      if (active && active !== document.body && this.outlet.contains(active)) return;
      const heading = this.outlet.querySelector<HTMLElement>("h1");
      heading?.setAttribute("tabindex", "-1");
      heading?.focus({ preventScroll: true });
    });
  }

  private context(path: string): PageContext {
    return { navigate: this.navigate, session: this.session, path, signOut: this.signOut };
  }

  private routeFor(path: string): Route {
    const parts = path.split("/").filter(Boolean).map(decodeURIComponent);
    const context = this.context(path);
    if (parts.length === 0) return { key: "home", make: () => createPage("vcs-home-page", HomePage, context) };
    if (parts[0] === "me") return { key: "me", make: () => createPage("vcs-me-page", MePage, context) };
    if (parts.length === 1) return { key: `org:${parts[0]}`, make: () => createPage("vcs-org-page", OrgPage, context) };
    if (parts.length === 4 && parts[2] === "pulls" && /^\d+$/.test(parts[3]!)) {
      return { key: `pull:${parts[0]}/${parts[1]}#${parts[3]}`, make: () => createPage("vcs-pull-page", PullPage, context) };
    }
    return { key: `repo:${parts[0]}/${parts[1]}`, make: () => createPage("vcs-repo-page", RepoPage, context) };
  }
}
