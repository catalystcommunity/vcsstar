//! Instance roles (`Instance/*`, table `instance_roles`). Only instance
//! admins manage them. "admin" is what `INSTANCE_ADMINS` gives; "viewer"
//! reads every repository. Rows with `source = 'claims'` are reserved for
//! a LinkKeys claim policy (not built yet); an admin sets `manual` rows.
//! Every change writes an event on the lane `@instance` (no organization
//! name can start with `@`).

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_db::models::Identity;
use vcsstar_db::schema::{identities, instance_roles};
use vcsstar_db::DbConn;
use vcsstar_types::{
    Empty, EventPayload, InstanceRoleGrant, InstanceRoleList, Ok as WireOk,
    RemoveInstanceRoleRequest, SetInstanceRoleRequest, SettingsPayload,
};

use crate::authz;
use crate::outbox::{self, Emit};
use crate::principal::{identity_to_wire, InstanceRole, Principal, ReqCtx};
use crate::services::parse_identity_key;
use crate::{to_ms, App, Error};

/// Outbox lane of instance-wide changes.
pub const LANE: &str = "@instance";

fn require_admin<'a>(app: &App, ctx: &'a ReqCtx) -> Result<&'a Principal, Error> {
    let p = ctx.require()?;
    p.require_scope("admin")?;
    if !authz::is_instance_admin(app, p) {
        return Err(Error::forbidden("requires an instance admin"));
    }
    Ok(p)
}

fn load_identity(conn: &mut DbConn, key: &str) -> Result<Identity, Error> {
    let (user_id, domain) = parse_identity_key(key)?;
    identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq(&domain))
        .select(Identity::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("identity (it must sign in once first)"))
}

type GrantRow = (
    uuid::Uuid,
    String,
    String,
    Option<uuid::Uuid>,
    chrono::DateTime<Utc>,
);

fn grant_wire(conn: &mut DbConn, row: &GrantRow) -> Result<InstanceRoleGrant, Error> {
    let (identity_id, role, source, granted_by, granted_at) = row;
    let load = |conn: &mut DbConn, id: uuid::Uuid| -> Result<Identity, Error> {
        Ok(identities::table
            .find(id)
            .select(Identity::as_select())
            .first(conn)?)
    };
    let identity = load(conn, *identity_id)?;
    let by = match granted_by {
        Some(id) => Some(identity_to_wire(&load(conn, *id)?)),
        None => None,
    };
    Ok(InstanceRoleGrant {
        identity: identity_to_wire(&identity),
        role: role.clone(),
        source: source.clone(),
        granted_by: by,
        granted_at: to_ms(*granted_at),
    })
}

fn emit(conn: &mut PgConnection, p: &Principal, kind: &str, subject: String) -> Result<(), Error> {
    outbox::emit(
        conn,
        Emit {
            lane: LANE.into(),
            kind,
            repo_id: None,
            org_id: None,
            actor_id: Some(p.identity.id),
            actor: Some(p.to_wire()),
            repo: None,
            org: None,
            subject: Some(subject.clone()),
            payload: EventPayload::Variant5(SettingsPayload {
                changed: vec![subject],
            }),
        },
    )?;
    Ok(())
}

pub fn list_roles(app: &App, ctx: &ReqCtx, _req: Empty) -> Result<InstanceRoleList, Error> {
    require_admin(app, ctx)?;
    let mut conn = app.conn()?;
    let rows: Vec<GrantRow> = instance_roles::table
        .order(instance_roles::granted_at.asc())
        .select((
            instance_roles::identity_id,
            instance_roles::role,
            instance_roles::source,
            instance_roles::granted_by,
            instance_roles::granted_at,
        ))
        .load(&mut conn)?;
    let mut grants = Vec::with_capacity(rows.len());
    for r in &rows {
        grants.push(grant_wire(&mut conn, r)?);
    }
    Ok(InstanceRoleList {
        grants,
        configured_admins: app.config.instance_admins.clone(),
    })
}

pub fn set_role(
    app: &App,
    ctx: &ReqCtx,
    req: SetInstanceRoleRequest,
) -> Result<InstanceRoleGrant, Error> {
    let p = require_admin(app, ctx)?;
    let role = InstanceRole::parse(&req.role)
        .ok_or_else(|| Error::invalid("role must be \"admin\" or \"viewer\""))?;
    let mut conn = app.conn()?;
    let identity = load_identity(&mut conn, &req.identity)?;
    let now = Utc::now();
    let row: GrantRow = conn.transaction::<GrantRow, Error, _>(|conn| {
        let row = diesel::insert_into(instance_roles::table)
            .values((
                instance_roles::identity_id.eq(identity.id),
                instance_roles::role.eq(role.as_str()),
                instance_roles::source.eq("manual"),
                instance_roles::granted_by.eq(p.identity.id),
                instance_roles::granted_at.eq(now),
            ))
            .on_conflict(instance_roles::identity_id)
            .do_update()
            .set((
                instance_roles::role.eq(role.as_str()),
                instance_roles::source.eq("manual"),
                instance_roles::granted_by.eq(p.identity.id),
                instance_roles::granted_at.eq(now),
            ))
            .returning((
                instance_roles::identity_id,
                instance_roles::role,
                instance_roles::source,
                instance_roles::granted_by,
                instance_roles::granted_at,
            ))
            .get_result(conn)?;
        emit(
            conn,
            p,
            "instance_role_set",
            format!(
                "{}@{}:{}",
                identity.lk_user_id,
                identity.lk_domain,
                role.as_str()
            ),
        )?;
        Ok(row)
    })?;
    grant_wire(&mut conn, &row)
}

pub fn remove_role(
    app: &App,
    ctx: &ReqCtx,
    req: RemoveInstanceRoleRequest,
) -> Result<WireOk, Error> {
    let p = require_admin(app, ctx)?;
    let mut conn = app.conn()?;
    let identity = load_identity(&mut conn, &req.identity)?;
    if identity.id == p.identity.id
        && !authz::is_configured_admin(app, &p.key())
        && p.instance_role == Some(InstanceRole::Admin)
    {
        return Err(Error::invalid(
            "you cannot remove your own admin role; ask another admin",
        ));
    }
    let removed = conn.transaction::<usize, Error, _>(|conn| {
        let n = diesel::delete(instance_roles::table.find(identity.id)).execute(conn)?;
        if n > 0 {
            emit(
                conn,
                p,
                "instance_role_removed",
                format!("{}@{}", identity.lk_user_id, identity.lk_domain),
            )?;
        }
        Ok(n)
    })?;
    if removed == 0 {
        return Err(Error::not_found("instance role"));
    }
    Ok(WireOk { ok: true })
}
