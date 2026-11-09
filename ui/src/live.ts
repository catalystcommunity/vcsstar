// Live updates over the CSIL-Events WebSocket carrier (verbose profile):
// `$hello` / `$hello-ack`, then `subscribe` control events out and
// `StreamMessage` events in. Loss is acceptable: pages re-read state on
// reconnect, and the server keeps no cursor for the UI.

import { fromStreamMessageCbor, toStreamControlCbor } from "./generated/codec.gen.ts";
import type { Event, StreamMessage } from "./generated/types.gen.ts";
import { array, type Cbor, decode as decodeCbor, encode as encodeCbor } from "./csil/cbor.ts";
import { tag24, untag24, canonMap, intValue, textValue, mapGet } from "./csil/conventions.ts";

export type EventHandler = (e: Event) => void;

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/csil/v1/events`;
}

function frame(event: string, payload: Uint8Array, extra: Array<readonly [string, Cbor]> = []): Uint8Array {
  return encodeCbor(canonMap([["event", textValue(event)], ["payload", tag24(payload)], ...extra]));
}

export class LiveEvents {
  private ws?: WebSocket;
  private lanes = new Set<string>();
  private handlers = new Set<EventHandler>();
  private ready = false;
  private backoff = 1000;
  private closed = false;

  subscribe(lanes: string[], handler: EventHandler): () => void {
    for (const l of lanes) this.lanes.add(l);
    this.handlers.add(handler);
    this.ensure();
    if (this.ready) this.sendSubscribe();
    return () => {
      this.handlers.delete(handler);
      if (this.handlers.size === 0) this.lanes.clear();
    };
  }

  private ensure() {
    if (this.ws && this.ws.readyState <= WebSocket.OPEN) return;
    this.closed = false;
    const ws = new WebSocket(wsUrl());
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    ws.onopen = () => {
      ws.send(
        frame(
          "$hello",
          encodeCbor(
            canonMap([
              ["versions", array([intValue(1)])],
              ["profiles", array([textValue("verbose")])],
              ["service", textValue("EventStream")],
            ]),
          ),
        ),
      );
    };
    ws.onmessage = (m) => {
      const v = decodeCbor(new Uint8Array(m.data as ArrayBuffer));
      const eventField = mapGet(v, "event");
      const payloadField = mapGet(v, "payload");
      if (!eventField || eventField.t !== "text" || payloadField === undefined) return;
      const event = eventField.v;
      const payload = untag24(payloadField);
      if (event === "$hello-ack") {
        this.ready = true;
        this.backoff = 1000;
        this.sendSubscribe();
        return;
      }
      if (event === "$ping") {
        ws.send(frame("$pong", payload));
        return;
      }
      if (event.startsWith("$")) return;
      let msg: StreamMessage;
      try {
        msg = fromStreamMessageCbor(payload);
      } catch {
        return;
      }
      if ("lane" in msg && "kind" in msg) for (const h of this.handlers) h(msg as Event);
    };
    ws.onclose = () => {
      this.ready = false;
      if (this.closed || this.handlers.size === 0) return;
      setTimeout(() => this.ensure(), this.backoff);
      this.backoff = Math.min(this.backoff * 2, 30_000);
    };
    ws.onerror = () => ws.close();
  }

  private sendSubscribe() {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN || this.lanes.size === 0) return;
    this.ws.send(frame("subscribe", toStreamControlCbor({ lanes: [...this.lanes] })));
  }

  close() {
    this.closed = true;
    this.ws?.close();
  }
}

export const live = new LiveEvents();
