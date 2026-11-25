// The browser speaks CSIL-RPC to vcsstar over the HTTP envelope-in-body
// carrier. The session cookie authenticates; the custom header is the CSRF
// check the server requires for cookie credentials. No JSON API exists.

import { AsyncApiClient, type AsyncServiceTransport } from "./generated/client.async.gen.ts";
import { ServiceErrorRefusal } from "./generated/codec.gen.ts";
import type { Ctx, Identity } from "./generated/types.gen.ts";
import { RpcRequest, RpcResponse } from "./csil/rpc.ts";

export class ApiError extends Error {
  constructor(
    public code: number,
    message: string,
  ) {
    super(message);
  }
}

export const RPC_PATH = "/csil/v1/rpc";

class FetchTransport implements AsyncServiceTransport {
  private nextId = 1;
  async call(service: string, op: string, req: Uint8Array): Promise<Uint8Array> {
    return (await this.callReply(service, op, req)).payload;
  }

  async callReply(service: string, op: string, req: Uint8Array): Promise<RpcResponse> {
    const envelope = new RpcRequest(service, op, req).withId(this.nextId++).encode();
    const res = await fetch(RPC_PATH, {
      method: "POST",
      headers: { "Content-Type": "application/cbor", "X-Vcsstar-Csrf": "1" },
      body: envelope as BodyInit,
      credentials: "same-origin",
    });
    if (!res.ok) throw new ApiError(0, `HTTP ${res.status} from ${RPC_PATH}`);
    const reply = RpcResponse.decode(new Uint8Array(await res.arrayBuffer()));
    if (reply.status !== 0) {
      const code = reply.status === 3 ? 10 : reply.status === 4 ? 11 : 0;
      throw new ApiError(code, reply.error ?? `transport status ${reply.status}`);
    }
    return reply;
  }
}

export const api = new AsyncApiClient(new FetchTransport());

export function ctx(): Ctx {
  return { requestId: crypto.randomUUID() };
}

export function who(i?: Identity | null): string {
  if (!i) return "";
  return `${i.handle ?? i.userId.slice(0, 8)}@${i.domain}`;
}

export function short(sha?: string | null): string {
  return (sha ?? "").slice(0, 10);
}

export function when(ms?: number | null): string {
  if (!ms) return "";
  const d = new Date(ms);
  const diff = Date.now() - ms;
  // A time to come (an expiry): the date, or "in N min/h" when close.
  if (diff < -60_000) {
    if (diff > -3_600_000) return `in ${Math.ceil(-diff / 60_000)} min`;
    if (diff > -86_400_000) return `in ${Math.ceil(-diff / 3_600_000)} h`;
    return d.toISOString().slice(0, 10);
  }
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} h ago`;
  return d.toISOString().slice(0, 10);
}

function apiErrorDetails(e: unknown) {
  if (e instanceof ServiceErrorRefusal) return e.value;
  if (e instanceof ApiError) return e;
}

export function errorText(e: unknown): string {
  const details = apiErrorDetails(e);
  if (details) return details.code ? `${details.message} (code ${details.code})` : details.message;
  return e instanceof Error ? e.message : String(e);
}

export function isUnauthenticated(e: unknown): boolean {
  const code = apiErrorDetails(e)?.code;
  return code === 10 || code === 12;
}

/** Decode a base64 blob payload to text, or null when not UTF-8. */
export function bytesToText(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}
