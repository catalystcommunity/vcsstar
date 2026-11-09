//! Organizations, membership, and teams.

use diesel::prelude::*;
use vscstar_db::models::{Identity, NewOrg, Org as OrgRow};
use vscstar_db::schema::{identities, org_members, orgs, team_members, teams};
use vscstar_db::DbConn;
use vscstar_types::{
    CreateOrgRequest, CreateTeamRequest, GetOrgRequest, ListOrgMembersRequest, ListOrgsRequest,
    ListTeamsResponse, Ok as WireOk, Org, OrgMember, OrgMemberPage, OrgPage,
    RemoveOrgMemberRequest, SetOrgMemberRequest, SetTeamMemberRequest, Team, TeamRef,
    UpdateOrgRequest,
};

use crate::authz::{self, OrgRole};
use crate::outbox::{self, Emit};
use crate::principal::{identity_to_wire, ReqCtx};
use crate::services::{check_name, parse_identity_key};
use crate::{to_ms, App, Error};

fn to_wire(o: &OrgRow) -> Org {
    Org {
        name: o.name.clone(),
        display_name: o.display_name.clone(),
        github_owner: o.github_owner.clone(),
        created_at: to_ms(o.created_at),
    }
}

fn load_identity(conn: &mut DbConn, key: &str) -> Result<Identity, Error> {
    let (user_id, domain) = parse_identity_key(key)?;
    identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq(&domain))
        .select(Identity::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("identity (the user must log in once first)"))
}

pub fn create(app: &App, ctx: &ReqCtx, req: CreateOrgRequest) -> Result<Org, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    if !(app.config.open_org_creation || authz::is_instance_admin(app, p)) {
        return Err(Error::forbidden(
            "only instance admins may create organizations",
        ));
    }
    check_name("organization", &req.name)?;
    let mut conn = app.conn()?;
    let org = conn.transaction::<OrgRow, Error, _>(|conn| {
        let row = NewOrg {
            id: vscstar_db::new_id(),
            name: &req.name,
            display_name: req.display_name.as_deref(),
            github_owner: req.github_owner.as_deref(),
        };
        let org: OrgRow = diesel::insert_into(orgs::table)
            .values(&row)
            .returning(OrgRow::as_returning())
            .get_result(conn)?;
        diesel::insert_into(org_members::table)
            .values((
                org_members::org_id.eq(org.id),
                org_members::identity_id.eq(p.identity.id),
                org_members::role.eq("owner"),
            ))
            .execute(conn)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_org(&org.name),
                kind: "org_created",
                repo_id: None,
                org_id: Some(org.id),
                actor_id: Some(p.identity.id),
                actor: Some(p.to_wire()),
                repo: None,
                org: Some(org.name.clone()),
                subject: None,
                payload: vscstar_types::EventPayload::Variant5(vscstar_types::SettingsPayload {
                    changed: vec!["created".into()],
                }),
            },
        )?;
        Ok(org)
    })?;
    Ok(to_wire(&org))
}

pub fn get(app: &App, _ctx: &ReqCtx, req: GetOrgRequest) -> Result<Org, Error> {
    let mut conn = app.conn()?;
    Ok(to_wire(&authz::load_org(&mut conn, &req.org)?))
}

pub fn list(app: &App, _ctx: &ReqCtx, req: ListOrgsRequest) -> Result<OrgPage, Error> {
    let mut conn = app.conn()?;
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(50)
        .clamp(1, 200) as i64;
    let mut q = orgs::table
        .into_boxed()
        .order(orgs::name.asc())
        .limit(limit + 1);
    if let Some(cursor) = req.page.as_ref().and_then(|p| p.cursor.clone()) {
        q = q.filter(orgs::name.gt(cursor));
    }
    let mut rows: Vec<OrgRow> = q.select(OrgRow::as_select()).load(&mut conn)?;
    let next_cursor = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|o| o.name.clone())
    } else {
        None
    };
    Ok(OrgPage {
        orgs: rows.iter().map(to_wire).collect(),
        next_cursor,
    })
}

pub fn update(app: &App, ctx: &ReqCtx, req: UpdateOrgRequest) -> Result<Org, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let updated: OrgRow = diesel::update(orgs::table.find(org.id))
        .set((
            req.display_name.as_ref().map(|v| orgs::display_name.eq(v)),
            req.github_owner.as_ref().map(|v| orgs::github_owner.eq(v)),
        ))
        .returning(OrgRow::as_returning())
        .get_result(&mut conn)?;
    Ok(to_wire(&updated))
}

pub fn set_member(app: &App, ctx: &ReqCtx, req: SetOrgMemberRequest) -> Result<OrgMember, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    let role = OrgRole::parse(&req.role)
        .ok_or_else(|| Error::invalid("role must be owner, admin, or member"))?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    let caller_role = authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    if role == OrgRole::Owner && caller_role < OrgRole::Owner {
        return Err(Error::forbidden("only owners may add owners"));
    }
    let target = load_identity(&mut conn, &req.identity)?;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::insert_into(org_members::table)
            .values((
                org_members::org_id.eq(org.id),
                org_members::identity_id.eq(target.id),
                org_members::role.eq(role.as_str()),
            ))
            .on_conflict((org_members::org_id, org_members::identity_id))
            .do_update()
            .set(org_members::role.eq(role.as_str()))
            .execute(conn)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_org(&org.name),
                kind: "member_changed",
                repo_id: None,
                org_id: Some(org.id),
                actor_id: Some(p.identity.id),
                actor: Some(p.to_wire()),
                repo: None,
                org: Some(org.name.clone()),
                subject: Some(req.identity.clone()),
                payload: vscstar_types::EventPayload::Variant7(vscstar_types::MembershipPayload {
                    identity: req.identity.clone(),
                    role: Some(role.as_str().to_string()),
                    team: None,
                }),
            },
        )?;
        Ok(())
    })?;
    Ok(OrgMember {
        identity: identity_to_wire(&target),
        role: role.as_str().to_string(),
    })
}

pub fn remove_member(
    app: &App,
    ctx: &ReqCtx,
    req: RemoveOrgMemberRequest,
) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let target = load_identity(&mut conn, &req.identity)?;
    let owners: i64 = org_members::table
        .filter(org_members::org_id.eq(org.id))
        .filter(org_members::role.eq("owner"))
        .count()
        .get_result(&mut conn)?;
    let target_role: Option<String> = org_members::table
        .filter(org_members::org_id.eq(org.id))
        .filter(org_members::identity_id.eq(target.id))
        .select(org_members::role)
        .first(&mut conn)
        .optional()?;
    if target_role.as_deref() == Some("owner") && owners <= 1 {
        return Err(Error::new(
            crate::Code::PreconditionFailed,
            "an organization keeps at least one owner",
        ));
    }
    let n = diesel::delete(
        org_members::table
            .filter(org_members::org_id.eq(org.id))
            .filter(org_members::identity_id.eq(target.id)),
    )
    .execute(&mut conn)?;
    if n == 0 {
        return Err(Error::not_found("membership"));
    }
    Ok(WireOk { ok: true })
}

pub fn list_members(
    app: &App,
    ctx: &ReqCtx,
    req: ListOrgMembersRequest,
) -> Result<OrgMemberPage, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Member)?;
    let rows: Vec<(String, Identity)> = org_members::table
        .inner_join(identities::table)
        .filter(org_members::org_id.eq(org.id))
        .order(identities::handle.asc())
        .select((org_members::role, Identity::as_select()))
        .load(&mut conn)?;
    Ok(OrgMemberPage {
        members: rows
            .into_iter()
            .map(|(role, i)| OrgMember {
                identity: identity_to_wire(&i),
                role,
            })
            .collect(),
        next_cursor: None,
    })
}

fn team_to_wire(
    conn: &mut DbConn,
    org: &str,
    team_id: uuid::Uuid,
    name: &str,
) -> Result<Team, Error> {
    let members: Vec<Identity> = team_members::table
        .inner_join(identities::table)
        .filter(team_members::team_id.eq(team_id))
        .select(Identity::as_select())
        .load(conn)?;
    Ok(Team {
        org: org.to_string(),
        name: name.to_string(),
        members: members.iter().map(identity_to_wire).collect(),
    })
}

pub fn create_team(app: &App, ctx: &ReqCtx, req: CreateTeamRequest) -> Result<Team, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    check_name("team", &req.name)?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let id = vscstar_db::new_id();
    diesel::insert_into(teams::table)
        .values((
            teams::id.eq(id),
            teams::org_id.eq(org.id),
            teams::name.eq(&req.name),
        ))
        .execute(&mut conn)?;
    team_to_wire(&mut conn, &org.name, id, &req.name)
}

fn load_team(conn: &mut DbConn, org_id: uuid::Uuid, name: &str) -> Result<uuid::Uuid, Error> {
    teams::table
        .filter(teams::org_id.eq(org_id))
        .filter(teams::name.eq(name))
        .select(teams::id)
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("team"))
}

pub fn delete_team(app: &App, ctx: &ReqCtx, req: TeamRef) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let id = load_team(&mut conn, org.id, &req.team)?;
    diesel::delete(teams::table.find(id)).execute(&mut conn)?;
    Ok(WireOk { ok: true })
}

pub fn set_team_member(app: &App, ctx: &ReqCtx, req: SetTeamMemberRequest) -> Result<Team, Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let team_id = load_team(&mut conn, org.id, &req.team)?;
    let target = load_identity(&mut conn, &req.identity)?;
    if req.member {
        diesel::insert_into(team_members::table)
            .values((
                team_members::team_id.eq(team_id),
                team_members::identity_id.eq(target.id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)?;
    } else {
        diesel::delete(
            team_members::table
                .filter(team_members::team_id.eq(team_id))
                .filter(team_members::identity_id.eq(target.id)),
        )
        .execute(&mut conn)?;
    }
    team_to_wire(&mut conn, &org.name, team_id, &req.team)
}

pub fn list_teams(app: &App, ctx: &ReqCtx, req: GetOrgRequest) -> Result<ListTeamsResponse, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Member)?;
    let rows: Vec<(uuid::Uuid, String)> = teams::table
        .filter(teams::org_id.eq(org.id))
        .order(teams::name.asc())
        .select((teams::id, teams::name))
        .load(&mut conn)?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, name) in rows {
        out.push(team_to_wire(&mut conn, &org.name, id, &name)?);
    }
    Ok(ListTeamsResponse { teams: out })
}
