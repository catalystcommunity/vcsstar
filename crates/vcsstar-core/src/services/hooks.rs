//! Outbound hook endpoints and their delivery log. The dispatcher and the
//! hook worker (vcsstar-workers) do the delivery; these ops manage the rows.

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use vcsstar_db::schema::{hook_deliveries, hook_endpoints, orgs, repos};
use vcsstar_db::DbConn;
use vcsstar_types::{
    CreateEndpointRequest, Delivery, DeliveryPage, DeliveryQuery, DeliveryRef, Endpoint,
    EndpointRef, ListEndpointsRequest, ListEndpointsResponse, Ok as WireOk, UpdateEndpointRequest,
};

use crate::authz::{self, OrgRole, RepoRole};
use crate::principal::ReqCtx;
use crate::{to_ms, App, Code, Error};

pub const EVENT_KINDS: &[&str] = &[
    "org_created",
    "repo_created",
    "repo_renamed",
    "repo_archived",
    "repo_deleted",
    "repo_settings_changed",
    "push",
    "ref_create",
    "ref_delete",
    "tag_create",
    "tag_delete",
    "release_created",
    "release_updated",
    "pr_opened",
    "pr_head_moved",
    "pr_retargeted",
    "pr_review",
    "pr_comment",
    "pr_merged",
    "pr_closed",
    "pr_reopened",
    "pr_updated",
    "status_set",
    "protection_changed",
    "grant_changed",
    "member_changed",
    "mirror_blocked",
    "mirror_recovered",
    "mirror_drift",
];

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = hook_endpoints)]
pub struct EndpointRow {
    pub id: uuid::Uuid,
    pub org_id: uuid::Uuid,
    pub repo_id: Option<uuid::Uuid>,
    pub kind: String,
    pub url: Option<String>,
    pub corndogs_queue: Option<String>,
    pub secret: Option<Vec<u8>>,
    pub event_filter: Vec<Option<String>>,
    pub content_type: String,
    pub active: bool,
    pub unordered: bool,
    pub created_by: Option<uuid::Uuid>,
    pub created_at: DateTime<Utc>,
}

impl EndpointRow {
    pub fn wants(&self, kind: &str) -> bool {
        let f: Vec<&str> = self
            .event_filter
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        f.is_empty() || f.contains(&kind)
    }
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = hook_deliveries)]
pub struct DeliveryRow {
    pub id: uuid::Uuid,
    pub endpoint_id: uuid::Uuid,
    pub event_lane: String,
    pub event_seq: i64,
    pub event_kind: String,
    pub state: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub last_status: Option<i32>,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub payload_cbor: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

fn to_wire(conn: &mut DbConn, e: &EndpointRow) -> Result<Endpoint, Error> {
    let org: String = orgs::table.find(e.org_id).select(orgs::name).first(conn)?;
    let repo: Option<String> = match e.repo_id {
        Some(id) => repos::table
            .find(id)
            .select(repos::name)
            .first(conn)
            .optional()?,
        None => None,
    };
    Ok(Endpoint {
        id: e.id.to_string(),
        org,
        repo,
        kind: e.kind.clone(),
        url: e.url.clone(),
        corndogs_queue: e.corndogs_queue.clone(),
        event_filter: e.event_filter.iter().flatten().cloned().collect(),
        content_type: e.content_type.clone(),
        active: e.active,
        unordered: e.unordered,
        created_at: to_ms(e.created_at),
    })
}

pub fn delivery_to_wire(d: &DeliveryRow) -> Delivery {
    Delivery {
        id: d.id.to_string(),
        endpoint_id: d.endpoint_id.to_string(),
        event_lane: d.event_lane.clone(),
        event_seq: d.event_seq.max(0) as u64,
        event_kind: d.event_kind.clone(),
        state: d.state.clone(),
        attempts: d.attempts.max(0) as u64,
        next_attempt_at: (d.state == "pending").then(|| to_ms(d.next_attempt_at)),
        last_status: d.last_status.map(|s| s.max(0) as u64),
        last_error: d.last_error.clone(),
        delivered_at: d.delivered_at.map(to_ms),
    }
}

/// Org admin, or repo admin for a repo-scoped endpoint.
fn authorize_target(
    app: &App,
    conn: &mut DbConn,
    ctx: &ReqCtx,
    org: &str,
    repo: Option<&str>,
) -> Result<(uuid::Uuid, Option<uuid::Uuid>), Error> {
    let p = ctx.require()?;
    p.require_scope("hooks:manage")?;
    let org_row = authz::load_org(conn, org)?;
    match repo {
        Some(r) => {
            let (_o, repo_row, role) = authz::load_repo_visible(app, conn, ctx, org, r)?;
            authz::require_repo_role(role, RepoRole::Admin)?;
            Ok((org_row.id, Some(repo_row.id)))
        }
        None => {
            authz::require_org_role(app, conn, &org_row, p, OrgRole::Admin)?;
            Ok((org_row.id, None))
        }
    }
}

fn check_filter(filter: &[String]) -> Result<(), Error> {
    for k in filter {
        if !EVENT_KINDS.contains(&k.as_str()) {
            return Err(Error::invalid(format!("unknown event kind {k}")));
        }
    }
    Ok(())
}

pub fn create_endpoint(
    app: &App,
    ctx: &ReqCtx,
    req: CreateEndpointRequest,
) -> Result<Endpoint, Error> {
    let mut conn = app.conn()?;
    let (org_id, repo_id) = authorize_target(app, &mut conn, ctx, &req.org, req.repo.as_deref())?;
    check_filter(&req.event_filter)?;
    let content_type = req.content_type.as_deref().unwrap_or("json");
    if !["json", "cbor"].contains(&content_type) {
        return Err(Error::invalid("content_type must be json or cbor"));
    }
    match req.kind.as_str() {
        "https" => {
            let url = req.url.as_deref().unwrap_or("");
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err(Error::invalid("https endpoints need an http(s) url"));
            }
            if req.secret.as_deref().unwrap_or("").len() < 16 {
                return Err(Error::invalid("secret must be at least 16 characters"));
            }
        }
        "corndogs" => {
            let q = req.corndogs_queue.as_deref().unwrap_or("");
            if q.is_empty() || q.contains(char::is_whitespace) {
                return Err(Error::invalid("corndogs endpoints need a corndogs_queue"));
            }
        }
        _ => return Err(Error::invalid("kind must be https or corndogs")),
    }
    let filter: Vec<Option<String>> = req.event_filter.iter().cloned().map(Some).collect();
    let row: EndpointRow = diesel::insert_into(hook_endpoints::table)
        .values((
            hook_endpoints::id.eq(vcsstar_db::new_id()),
            hook_endpoints::org_id.eq(org_id),
            hook_endpoints::repo_id.eq(repo_id),
            hook_endpoints::kind.eq(&req.kind),
            hook_endpoints::url.eq(req.url.as_deref()),
            hook_endpoints::corndogs_queue.eq(req.corndogs_queue.as_deref()),
            hook_endpoints::secret.eq(req.secret.as_deref().map(str::as_bytes)),
            hook_endpoints::event_filter.eq(&filter),
            hook_endpoints::content_type.eq(content_type),
            hook_endpoints::unordered.eq(req.unordered.unwrap_or(false)),
            hook_endpoints::created_by.eq(ctx.actor_id()),
        ))
        .returning(EndpointRow::as_returning())
        .get_result(&mut conn)?;
    to_wire(&mut conn, &row)
}

fn load_endpoint(
    app: &App,
    conn: &mut DbConn,
    ctx: &ReqCtx,
    id: &str,
) -> Result<EndpointRow, Error> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| Error::invalid("endpoint_id"))?;
    let row: EndpointRow = hook_endpoints::table
        .find(id)
        .select(EndpointRow::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("endpoint"))?;
    let org: String = orgs::table
        .find(row.org_id)
        .select(orgs::name)
        .first(conn)?;
    let repo: Option<String> = match row.repo_id {
        Some(r) => repos::table
            .find(r)
            .select(repos::name)
            .first(conn)
            .optional()?,
        None => None,
    };
    authorize_target(app, conn, ctx, &org, repo.as_deref()).map_err(|e| {
        if e.code == Code::Forbidden {
            Error::not_found("endpoint")
        } else {
            e
        }
    })?;
    Ok(row)
}

pub fn list_endpoints(
    app: &App,
    ctx: &ReqCtx,
    req: ListEndpointsRequest,
) -> Result<ListEndpointsResponse, Error> {
    let mut conn = app.conn()?;
    let (org_id, repo_id) = authorize_target(
        app,
        &mut conn,
        ctx,
        &req.target.org,
        req.target.repo.as_deref(),
    )?;
    let mut q = hook_endpoints::table
        .filter(hook_endpoints::org_id.eq(org_id))
        .into_boxed();
    if let Some(r) = repo_id {
        q = q.filter(hook_endpoints::repo_id.eq(r));
    }
    let rows: Vec<EndpointRow> = q
        .order(hook_endpoints::created_at.asc())
        .select(EndpointRow::as_select())
        .load(&mut conn)?;
    let mut endpoints = Vec::with_capacity(rows.len());
    for r in &rows {
        endpoints.push(to_wire(&mut conn, r)?);
    }
    Ok(ListEndpointsResponse { endpoints })
}

pub fn update_endpoint(
    app: &App,
    ctx: &ReqCtx,
    req: UpdateEndpointRequest,
) -> Result<Endpoint, Error> {
    let mut conn = app.conn()?;
    let row = load_endpoint(app, &mut conn, ctx, &req.endpoint_id)?;
    if let Some(f) = &req.event_filter {
        check_filter(f)?;
    }
    let filter: Option<Vec<Option<String>>> = req
        .event_filter
        .as_ref()
        .map(|f| f.iter().cloned().map(Some).collect());
    let updated: EndpointRow = diesel::update(hook_endpoints::table.find(row.id))
        .set((
            req.url.as_ref().map(|u| hook_endpoints::url.eq(u)),
            filter.as_ref().map(|f| hook_endpoints::event_filter.eq(f)),
            req.active.map(|a| hook_endpoints::active.eq(a)),
            req.secret
                .as_ref()
                .map(|s| hook_endpoints::secret.eq(s.as_bytes())),
            req.unordered.map(|u| hook_endpoints::unordered.eq(u)),
        ))
        .returning(EndpointRow::as_returning())
        .get_result(&mut conn)?;
    to_wire(&mut conn, &updated)
}

pub fn delete_endpoint(app: &App, ctx: &ReqCtx, req: EndpointRef) -> Result<WireOk, Error> {
    let mut conn = app.conn()?;
    let row = load_endpoint(app, &mut conn, ctx, &req.endpoint_id)?;
    diesel::delete(hook_endpoints::table.find(row.id)).execute(&mut conn)?;
    Ok(WireOk { ok: true })
}

pub fn list_deliveries(app: &App, ctx: &ReqCtx, req: DeliveryQuery) -> Result<DeliveryPage, Error> {
    let mut conn = app.conn()?;
    let row = load_endpoint(app, &mut conn, ctx, &req.endpoint_id)?;
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(50)
        .clamp(1, 200) as i64;
    let mut q = hook_deliveries::table
        .filter(hook_deliveries::endpoint_id.eq(row.id))
        .into_boxed();
    if let Some(s) = &req.state {
        q = q.filter(hook_deliveries::state.eq(s));
    }
    if let Some(c) = req.page.as_ref().and_then(|p| p.cursor.as_deref()) {
        let cursor = uuid::Uuid::parse_str(c).map_err(|_| Error::invalid("cursor"))?;
        q = q.filter(hook_deliveries::id.lt(cursor));
    }
    let rows: Vec<DeliveryRow> = q
        .order(hook_deliveries::id.desc())
        .limit(limit + 1)
        .select(DeliveryRow::as_select())
        .load(&mut conn)?;
    let next_cursor = (rows.len() as i64 > limit).then(|| rows[limit as usize - 1].id.to_string());
    Ok(DeliveryPage {
        deliveries: rows
            .iter()
            .take(limit as usize)
            .map(delivery_to_wire)
            .collect(),
        next_cursor,
    })
}

/// Queue a delivery again: pending, due now. The dispatcher's reconcile
/// pass submits the wake-up task within a minute, or the worker picks it
/// up with the next event on the endpoint.
pub fn redeliver(app: &App, ctx: &ReqCtx, req: DeliveryRef) -> Result<WireOk, Error> {
    let mut conn = app.conn()?;
    let id = uuid::Uuid::parse_str(&req.delivery_id).map_err(|_| Error::invalid("delivery_id"))?;
    let d: DeliveryRow = hook_deliveries::table
        .find(id)
        .select(DeliveryRow::as_select())
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| Error::not_found("delivery"))?;
    load_endpoint(app, &mut conn, ctx, &d.endpoint_id.to_string())?;
    diesel::update(hook_deliveries::table.find(id))
        .set((
            hook_deliveries::state.eq("pending"),
            hook_deliveries::next_attempt_at.eq(Utc::now()),
            hook_deliveries::last_error.eq::<Option<String>>(None),
        ))
        .execute(&mut conn)?;
    Ok(WireOk { ok: true })
}
