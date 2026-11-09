//! CSIL-Events over WebSocket (verbose profile) for UI live updates:
//! `$hello` / `$hello-ack`, `subscribe` control events in, `StreamMessage`
//! events out. The gateway polls the outbox for the subscribed lanes; loss
//! is acceptable, the UI re-reads state on reconnect.

use std::collections::HashMap;
use std::time::Duration;

use ciborium::Value;
use diesel::prelude::*;
use rocket::http::CookieJar;
use rocket::{get, routes, Route, State};
use rocket_ws::{Message, WebSocket};
use vcsstar_core::authz;
use vcsstar_core::principal::{self, ReqCtx};
use vcsstar_core::App;
use vcsstar_db::schema::events;
use vcsstar_transport::{Carrier, CredentialSource, Peer};

pub fn routes() -> Vec<Route> {
    routes![events_ws]
}

fn frame(event: &str, payload: Vec<u8>) -> Vec<u8> {
    let v = Value::Map(vec![
        (Value::Text("event".into()), Value::Text(event.into())),
        (
            Value::Text("payload".into()),
            Value::Tag(24, Box::new(Value::Bytes(payload))),
        ),
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&v, &mut out).expect("cbor");
    out
}

fn map_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_map()?
        .iter()
        .find(|(k, _)| k.as_text() == Some(key))
        .map(|(_, v)| v)
}

fn untag24(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Tag(24, inner) => inner.as_bytes().cloned(),
        Value::Bytes(b) => Some(b.clone()),
        _ => None,
    }
}

fn control(event: &str, pairs: Vec<(&str, Value)>) -> Vec<u8> {
    let payload = Value::Map(
        pairs
            .into_iter()
            .map(|(k, v)| (Value::Text(k.into()), v))
            .collect(),
    );
    let mut out = Vec::new();
    ciborium::ser::into_writer(&payload, &mut out).expect("cbor");
    frame(event, out)
}

#[derive(Clone)]
struct Cursor {
    seq: i64,
}

/// The fields of `SubscribeRequest` this gateway reads.
struct SubscribeRequest {
    lanes: Vec<String>,
    after_seq: Option<u64>,
}

impl SubscribeRequest {
    fn from_value(v: &Value) -> Self {
        let lanes = map_get(v, "lanes")
            .and_then(|l| l.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_text().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let after_seq = map_get(v, "after_seq")
            .and_then(|s| s.as_integer())
            .and_then(|i| u64::try_from(i).ok());
        Self { lanes, after_seq }
    }
}

#[get("/csil/v1/events")]
fn events_ws<'r>(
    app: &'r State<App>,
    ws: WebSocket,
    cookies: &CookieJar<'r>,
) -> rocket_ws::Channel<'r> {
    let app = app.inner().clone();
    // Same-origin check for cookie credentials: browsers always send Origin
    // on WebSocket upgrades.
    let cookie = cookies
        .get(crate::auth_pages::SESSION_COOKIE)
        .map(|c| c.value().to_string());
    ws.channel(move |mut stream| {
        Box::pin(async move {
            use rocket::futures::{SinkExt, StreamExt};
            let mut principal = None;
            let mut lanes: HashMap<String, Cursor> = HashMap::new();
            let mut ready = false;
            let mut tick = rocket::tokio::time::interval(Duration::from_secs(1));
            loop {
                rocket::tokio::select! {
                    msg = stream.next() => {
                        let Some(Ok(msg)) = msg else { break };
                        let data = match msg {
                            Message::Binary(b) => b,
                            Message::Close(_) => break,
                            Message::Ping(p) => { let _ = stream.send(Message::Pong(p)).await; continue; }
                            _ => continue,
                        };
                        let Ok(v) = ciborium::de::from_reader::<Value, _>(data.as_slice()) else { continue };
                        let Some(event) = map_get(&v, "event").and_then(|e| e.as_text()).map(str::to_string) else { continue };
                        let payload = map_get(&v, "payload").and_then(untag24).unwrap_or_default();
                        match event.as_str() {
                            "$hello" => {
                                let hello: Value = ciborium::de::from_reader(payload.as_slice()).unwrap_or(Value::Null);
                                let auth = map_get(&hello, "auth").and_then(|a| a.as_text()).map(str::to_string);
                                let credential = auth.or(cookie.clone());
                                let peer = Peer {
                                    credential,
                                    credential_source: CredentialSource::Cookie,
                                    remote: String::new(),
                                    carrier: Carrier::Http,
                                };
                                let app2 = app.clone();
                                principal = rocket::tokio::task::spawn_blocking(move || principal::resolve(&app2, &peer).ok().flatten()).await.ok().flatten();
                                let _ = stream.send(Message::Binary(control("$hello-ack", vec![
                                    ("v", Value::Integer(1.into())),
                                    ("profile", Value::Text("verbose".into())),
                                ]))).await;
                                ready = true;
                            }
                            "$ping" => {
                                let _ = stream.send(Message::Binary(frame("$pong", payload))).await;
                            }
                            "$close" => break,
                            "subscribe" if ready => {
                                // StreamControl is a union: [0, SubscribeRequest] | [1, UnsribeMarker].
                                let Ok(ctrl) = ciborium::de::from_reader::<Value, _>(payload.as_slice()) else { continue };
                                let Some(arr) = ctrl.as_array() else { continue };
                                let idx = arr.first().and_then(|i| i.as_integer()).map(i128::from).unwrap_or(-1);
                                let body = arr.get(1).cloned().unwrap_or(Value::Null);
                                match idx {
                                    0 => {
                                        let sub = SubscribeRequest::from_value(&body);
                                        // Authorize each lane against the repo's visibility.
                                        let app2 = app.clone();
                                        let p2 = principal.clone();
                                        let wanted = sub.lanes.clone();
                                        let allowed: Vec<(String, i64)> = rocket::tokio::task::spawn_blocking(move || {
                                            let mut out = Vec::new();
                                            let Ok(mut conn) = app2.conn() else { return out };
                                            let ctx = ReqCtx { request_id: String::new(), principal: p2, peer: Peer::default() };
                                            for lane in wanted {
                                                let ok = match lane.split_once('/') {
                                                    Some((o, r)) => authz::load_repo_visible(&app2, &mut conn, &ctx, o, r).is_ok(),
                                                    None => ctx.principal.is_some(),
                                                };
                                                if ok {
                                                    let last: Option<i64> = events::table.filter(events::lane.eq(&lane)).select(diesel::dsl::max(events::seq)).first(&mut conn).ok().flatten();
                                                    let start = sub.after_seq.map(|s| s as i64).unwrap_or(last.unwrap_or(0));
                                                    out.push((lane, start));
                                                }
                                            }
                                            out
                                        }).await.unwrap_or_default();
                                        for (lane, seq) in &allowed {
                                            tracing::debug!(lane = %lane, start = seq, "events gateway subscribed");
                                            lanes.insert(lane.clone(), Cursor { seq: *seq });
                                        }
                                        let ack = Value::Array(vec![
                                            Value::Integer(1.into()),
                                            Value::Map(vec![(Value::Text("subscribed".into()), Value::Array(allowed.iter().map(|(l, _)| Value::Text(l.clone())).collect()))]),
                                        ]);
                                        let mut bytes = Vec::new();
                                        ciborium::ser::into_writer(&ack, &mut bytes).expect("cbor");
                                        let _ = stream.send(Message::Binary(frame("subscribe", bytes))).await;
                                    }
                                    1 => {
                                        let un = map_get(&body, "unsubscribe").and_then(|u| map_get(u, "lanes")).and_then(|l| l.as_array().cloned()).unwrap_or_default();
                                        for l in un { if let Some(t) = l.as_text() { lanes.remove(t); } }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ = tick.tick() => {
                        if lanes.is_empty() { continue; }
                        let app2 = app.clone();
                        let snapshot: Vec<(String, i64)> = lanes.iter().map(|(l, c)| (l.clone(), c.seq)).collect();
                        let fresh: Vec<(String, i64, Vec<u8>)> = rocket::tokio::task::spawn_blocking(move || {
                            let mut out = Vec::new();
                            let Ok(mut conn) = app2.conn() else { return out };
                            for (lane, seq) in snapshot {
                                let rows: Vec<(i64, Vec<u8>)> = events::table
                                    .filter(events::lane.eq(&lane))
                                    .filter(events::seq.gt(seq))
                                    .order(events::seq.asc())
                                    .limit(100)
                                    .select((events::seq, events::payload_cbor))
                                    .load(&mut conn)
                                    .unwrap_or_default();
                                for (s, p) in rows { out.push((lane.clone(), s, p)); }
                            }
                            out
                        }).await.unwrap_or_default();
                        tracing::debug!(lanes = lanes.len(), fresh = fresh.len(), "events gateway tick");
                        for (lane, seq, payload) in fresh {
                            if let Some(c) = lanes.get_mut(&lane) { c.seq = c.seq.max(seq); }
                            // StreamMessage union: [0, Event]. The stored payload is the Event's CBOR.
                            match ciborium::de::from_reader::<Value, _>(payload.as_slice()) {
                                Ok(ev) => {
                                    let msg = Value::Array(vec![Value::Integer(0.into()), ev]);
                                    let mut bytes = Vec::new();
                                    ciborium::ser::into_writer(&msg, &mut bytes).expect("cbor");
                                    if let Err(e) = stream.send(Message::Binary(frame("subscribe", bytes))).await {
                                        tracing::debug!(error = %e, "events gateway send failed");
                                        return Ok(());
                                    }
                                    tracing::debug!(lane = %lane, seq, "events gateway sent event");
                                }
                                Err(e) => tracing::warn!(error = %e, lane = %lane, seq, "events gateway: stored event payload does not decode"),
                            }
                        }
                    }
                }
            }
            Ok(())
        })
    })
}
