import { createResource, createSignal, For, Show } from "solid-js";
import { api, ctx, errorText, when, who } from "../api";

export function MePage() {
  const [me] = createResource(() => api.auth.whoami({ ctx: ctx() }).catch(() => null));
  const [keys, { refetch: refetchKeys }] = createResource(() => api.auth.listKeys({ ctx: ctx() }).catch(() => null));
  const [tokens, { refetch: refetchTokens }] = createResource(() => api.auth.listTokens({ ctx: ctx() }).catch(() => null));
  const [err, setErr] = createSignal("");
  const [newKey, setNewKey] = createSignal({ publicKey: "", usage: "auth" });
  const [newToken, setNewToken] = createSignal({ name: "", scopes: "repo:write,pr:write,pr:review,statuses:write,org:read" });
  const [minted, setMinted] = createSignal("");

  const run = async (f: () => Promise<unknown>, after?: () => void) => {
    setErr("");
    try {
      await f();
      after?.();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <h1>Account</h1>
      <Show when={me()} fallback={<p>Sign in to manage keys and tokens.</p>}>
        {(w) => (
          <div class="panel">
            <div>
              <strong>{who(w().identity)}</strong> <span class="muted">{w().identity.displayName}</span>
            </div>
            <div class="muted small">
              identity {w().identity.userId}@{w().identity.domain} · credential {w().credentialKind} · scopes {w().scopes.join(", ")}
            </div>
          </div>
        )}
      </Show>
      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>

      <Show when={keys()}>
        <h2>SSH keys</h2>
        <p class="muted small">
          Attested keys come from <code>vcsstar keys enroll</code> (LinkKeys application keys). Uploaded keys are unattested.
        </p>
        <table>
          <thead>
            <tr>
              <th>Key</th>
              <th>Usage</th>
              <th>Status</th>
              <th>Attested until</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <For each={keys()!.keys}>
              {(k) => (
                <tr>
                  <td class="mono small">{k.publicKey.split(" ").slice(0, 2).join(" ").slice(0, 60)}…</td>
                  <td>{k.usage}</td>
                  <td>
                    <span class={`badge ${k.status === "attested" ? "ok" : k.status === "revoked" ? "bad" : "warn"}`}>{k.status}</span>
                  </td>
                  <td class="muted">{k.attestedUntil ? new Date(k.attestedUntil).toISOString() : "—"}</td>
                  <td>
                    <Show when={k.status !== "revoked"}>
                      <button class="link danger" onClick={() => run(() => api.auth.revokeKey({ ctx: ctx(), keyId: k.keyId }), refetchKeys)}>
                        revoke
                      </button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.auth.registerKey({ ctx: ctx(), publicKey: newKey().publicKey, usage: newKey().usage }), () => {
              setNewKey({ publicKey: "", usage: "auth" });
              refetchKeys();
            });
          }}
        >
          <input placeholder="ssh-ed25519 AAAA… comment" size={60} value={newKey().publicKey} onInput={(e) => setNewKey({ ...newKey(), publicKey: e.currentTarget.value })} />
          <select value={newKey().usage} onChange={(e) => setNewKey({ ...newKey(), usage: e.currentTarget.value })}>
            <option value="auth">auth</option>
            <option value="signing">signing</option>
          </select>
          <button disabled={!newKey().publicKey}>Add unattested key</button>
        </form>
      </Show>

      <Show when={tokens()}>
        <h2>API tokens</h2>
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Scopes</th>
              <th>Label</th>
              <th>Created</th>
              <th>Expires</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <For each={tokens()!.tokens}>
              {(t) => (
                <tr>
                  <td>{t.name}</td>
                  <td class="small">{t.scopes.join(", ")}</td>
                  <td class="muted small">{t.clientLabel}</td>
                  <td class="muted">{when(t.createdAt)}</td>
                  <td class="muted">{t.expiresAt ? when(t.expiresAt) : "never"}</td>
                  <td>
                    <button class="link danger" onClick={() => run(() => api.auth.revokeToken({ ctx: ctx(), tokenId: t.id }), refetchTokens)}>
                      revoke
                    </button>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            run(
              async () => {
                const t = await api.auth.createToken({
                  ctx: ctx(),
                  name: newToken().name,
                  scopes: newToken().scopes.split(",").map((s) => s.trim()).filter(Boolean),
                });
                setMinted(t.secret ?? "");
              },
              () => {
                setNewToken({ ...newToken(), name: "" });
                refetchTokens();
              },
            );
          }}
        >
          <input placeholder="token name" value={newToken().name} onInput={(e) => setNewToken({ ...newToken(), name: e.currentTarget.value })} />
          <input size={50} value={newToken().scopes} onInput={(e) => setNewToken({ ...newToken(), scopes: e.currentTarget.value })} />
          <button disabled={!newToken().name}>Create token</button>
        </form>
        <Show when={minted()}>
          <div class="panel">
            <strong>Copy this token now; it is not shown again.</strong>
            <pre class="code">{minted()}</pre>
            <p class="muted small">
              Use it with <code>vcsstar login {location.origin} --token-stdin</code> or as the git HTTPS password.
            </p>
          </div>
        </Show>
      </Show>
    </div>
  );
}
