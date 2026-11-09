import { Router, Route, A, useNavigate } from "@solidjs/router";
import { createResource, createSignal, Show, type JSX } from "solid-js";
import { api, ctx, who, isUnauthenticated } from "./api";
import { Home } from "./pages/Home";
import { OrgPage } from "./pages/Org";
import { RepoPage } from "./pages/Repo";
import { PullPage } from "./pages/Pull";
import { MePage } from "./pages/Me";
import type { WhoAmI } from "./generated/types.gen.ts";

export const [session, setSession] = createSignal<WhoAmI | null | undefined>(undefined);

async function loadSession(): Promise<WhoAmI | null> {
  try {
    const w = await api.auth.whoami({ ctx: ctx() });
    setSession(w);
    return w;
  } catch (e) {
    if (isUnauthenticated(e)) {
      setSession(null);
      return null;
    }
    throw e;
  }
}

function Layout(props: { children?: JSX.Element }) {
  const [info] = createResource(() => api.meta.serverInfo({ ctx: ctx() }));
  const [me] = createResource(loadSession);
  const navigate = useNavigate();
  const loginHref = () => `/auth/login?return_to=${encodeURIComponent(location.pathname)}`;
  return (
    <div class="shell">
      <header class="top">
        <A href="/" class="brand">
          vcsstar
        </A>
        <nav>
          <A href="/">Repositories</A>
          <A href="/me">Account</A>
        </nav>
        <div class="grow" />
        <Show when={me.state === "ready"}>
          <Show
            when={me()}
            fallback={
              <a class="btn" href={loginHref()}>
                Sign in
              </a>
            }
          >
            {(w) => (
              <span class="muted">
                {who(w().identity)}{" "}
                <button
                  class="link"
                  onClick={async () => {
                    await fetch("/auth/logout", { method: "POST", credentials: "same-origin" });
                    setSession(null);
                    navigate("/");
                  }}
                >
                  sign out
                </button>
              </span>
            )}
          </Show>
        </Show>
      </header>
      <main>{props.children}</main>
      <footer class="muted small">
        <Show when={info()}>
          {(i) => (
            <span>
              {i().instanceIdentity} · vcsstar {i().version} · contract {i().contractVersion}
            </span>
          )}
        </Show>
      </footer>
    </div>
  );
}

export function App() {
  return (
    <Router root={Layout}>
      <Route path="/" component={Home} />
      <Route path="/me" component={MePage} />
      <Route path="/:org" component={OrgPage} />
      <Route path="/:org/:repo/pulls/:number" component={PullPage} />
      <Route path="/:org/:repo/*rest" component={RepoPage} />
    </Router>
  );
}
