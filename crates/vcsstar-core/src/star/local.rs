//! Read and update the local collaboration records. Remote access settings
//! are never part of the synchronized representation.
use super::{store, text, Item};
use crate::services::{
    prs::{CommentRow, PrRow, ReviewRow},
    statuses::StatusRow,
};
use crate::{outbox, push, vcs, App, Error};
use diesel::prelude::*;
use diesel::sql_types::{Jsonb, Text, Uuid as SqlUuid};
use liblinkkeys::timestamp::Rfc3339 as _;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;
use vcsstar_db::{
    models::{Identity, NewIdentity, Org, Repo},
    schema::{identities, pr_comments, pr_reviews, pull_requests, repos, statuses},
};
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_vcs::RevId;

pub fn load_repo(conn: &mut PgConnection, id: Uuid) -> Result<(Org, Repo), Error> {
    Ok(repos::table
        .inner_join(vcsstar_db::schema::orgs::table)
        .filter(repos::id.eq(id))
        .select((Org::as_select(), Repo::as_select()))
        .first(conn)?)
}
fn author(conn: &mut PgConnection, id: Option<Uuid>) -> Result<String, Error> {
    match id {
        Some(id) => {
            let i: Identity = identities::table
                .find(id)
                .select(Identity::as_select())
                .first(conn)?;
            Ok(format!("{}@{}", i.handle, i.lk_domain))
        }
        None => Ok("vcsstar".into()),
    }
}
fn row(
    conn: &mut PgConnection,
    kind: &str,
    data: Value,
    actor: Option<Uuid>,
    at: i64,
) -> Result<Item, Error> {
    Ok(Item {
        kind: kind.into(),
        data,
        actor,
        author: author(conn, actor)?,
        created_at: at,
        url: None,
    })
}
pub fn snapshot(conn: &mut PgConnection, repo: &Repo) -> Result<BTreeMap<String, Item>, Error> {
    let mut items = BTreeMap::new();
    items.insert("settings".into(),row(conn,"settings",json!({"description":repo.description.clone().unwrap_or_default(),"default_branch":repo.default_branch,"archived":repo.archived}),None,crate::to_ms(repo.created_at))?);
    for (name, sha) in refs::load(conn, repo.id)? {
        if name.starts_with("refs/heads/") || name.starts_with("refs/tags/") {
            use vcsstar_db::schema::repo_refs;
            let actor: Option<Uuid> = repo_refs::table
                .find((repo.id, &name))
                .select(repo_refs::updated_by)
                .first(conn)?;
            items.insert(
                format!("ref:{name}"),
                row(conn, "ref", json!({"sha":sha.0}), actor, 0)?,
            );
        }
    }
    for pr in pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .select(PrRow::as_select())
        .load::<PrRow>(conn)?
    {
        let key = format!("pr:{}", pr.id);
        items.insert(key.clone(),row(conn,"pr",json!({"title":pr.title,"body":pr.body.unwrap_or_default(),"head_ref":pr.head_ref,"base_ref":pr.base_ref,"head_sha":pr.head_sha,"base_sha":pr.base_sha,"state":pr.state,"merge_sha":pr.merged_commit_sha.unwrap_or_default(),"head_remote":pr.head_remote.unwrap_or_default()}),Some(pr.author_identity_id),crate::to_ms(pr.created_at))?);
        for c in pr_comments::table
            .filter(pr_comments::pr_id.eq(pr.id))
            .select(CommentRow::as_select())
            .load::<CommentRow>(conn)?
        {
            items.insert(format!("comment:{}",c.id),row(conn,"comment",json!({"parent":key,"body":c.body,"path":c.path,"line":c.line,"commit_sha":c.commit_sha}),Some(c.author),crate::to_ms(c.created_at))?);
        }
        for r in pr_reviews::table
            .filter(pr_reviews::pr_id.eq(pr.id))
            .select(ReviewRow::as_select())
            .load::<ReviewRow>(conn)?
        {
            items.insert(format!("review:{}",r.id),row(conn,"review",json!({"parent":key,"body":r.body.unwrap_or_default(),"state":r.state,"head_sha":r.head_sha_at_review}),Some(r.reviewer),crate::to_ms(r.created_at))?);
        }
    }
    for s in statuses::table
        .filter(statuses::repo_id.eq(repo.id))
        .select(StatusRow::as_select())
        .load::<StatusRow>(conn)?
    {
        items.insert(format!("status:{}:{}",s.sha,super::encoded(&s.context)),row(conn,"status",json!({"sha":s.sha,"context":s.context,"state":s.state,"target_url":s.target_url,"description":s.description}),s.reporter,crate::to_ms(s.created_at))?);
    }
    // Keep imported attribution, timestamps, and source links. Content is
    // always read from the native tables so local edits remain authoritative.
    for saved in store::objects(conn, repo.id)? {
        let original: Item = serde_json::from_value(saved.payload).map_err(Error::internal)?;
        if original.kind == "history" && !saved.deleted {
            items.insert(saved.object_key, original);
            continue;
        }
        if let Some(item) = items.get_mut(&saved.object_key) {
            if item.actor == original.actor {
                item.author = original.author;
                item.created_at = original.created_at;
                item.url = original.url;
            }
        }
    }
    Ok(items)
}
pub fn identity(
    conn: &mut PgConnection,
    c: &store::Connection,
    user: &Value,
) -> Result<Identity, Error> {
    let remote_id = user
        .get("id")
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .ok_or_else(|| Error::invalid("remote author has no id"))?;
    #[derive(QueryableByName)]
    struct Linked {
        #[diesel(sql_type=SqlUuid)]
        identity_id: Uuid,
    }
    let linked: Option<Linked> = diesel::sql_query(
        "SELECT identity_id FROM sync_links WHERE provider=$1 AND host=$2 AND remote_user_id=$3",
    )
    .bind::<Text, _>(&c.provider)
    .bind::<Text, _>(&c.host)
    .bind::<Text, _>(&remote_id)
    .get_result(conn)
    .optional()?;
    if let Some(link) = linked {
        return Ok(identities::table
            .find(link.identity_id)
            .select(Identity::as_select())
            .first(conn)?);
    }
    if c.provider == "github" {
        return crate::github_link::identity_for_user(conn, user);
    }
    let uid = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("https://{}/users/{remote_id}", c.host).as_bytes(),
    );
    let login = user
        .get("username")
        .or_else(|| user.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("remote-user");
    diesel::insert_into(identities::table)
        .values(&NewIdentity {
            id: Uuid::now_v7(),
            lk_user_id: uid,
            lk_domain: &c.host,
            handle: login,
            display_name: Some(login),
            email: None,
            email_verified: false,
        })
        .on_conflict((identities::lk_user_id, identities::lk_domain))
        .do_update()
        .set(identities::handle.eq(login))
        .execute(conn)?;
    Ok(identities::table
        .filter(identities::lk_user_id.eq(uid))
        .filter(identities::lk_domain.eq(&c.host))
        .select(Identity::as_select())
        .first(conn)?)
}
fn key_id(key: &str) -> Result<Uuid, Error> {
    key.split_once(':')
        .and_then(|(_, id)| Uuid::parse_str(id).ok())
        .ok_or_else(|| Error::invalid("invalid sync object key"))
}

/// Apply under a transaction. The engine locks the affected native row and
/// rechecks its value before calling this function.
pub fn apply(
    app: &App,
    conn: &mut PgConnection,
    org: &Org,
    repo: &Repo,
    key: &str,
    item: Option<&Item>,
    source: &str,
) -> Result<(), Error> {
    let kind = key.split(':').next().unwrap_or(key);
    outbox::with_origin(source, || {
        conn.transaction::<(),Error,_>(|conn| {
        crate::protection::lock(conn,repo.id)?;
        if kind=="history" {
            store::save_object(conn,repo.id,key,item,source)?;
            emit(conn,org,repo,"sync_applied",key,item.and_then(|i|i.actor))?;
            return Ok(());
        }
        if kind=="ref" {
            let name=key.strip_prefix("ref:").ok_or_else(||Error::invalid("ref key"))?;
            vcsstar_vcs::check_ref_name(name)?;
            if !name.starts_with("refs/heads/") && !name.starts_with("refs/tags/") { return Err(Error::invalid("only branches and tags can sync")); }
            let update=RefUpdate{name:name.into(),old:refs::get(conn,repo.id,name)?,new:item.map(|i|RevId(text(&i.data,"sha").into()))};
            let forced = push::check_sync_ref(app,conn,repo,&update)?;
            push::apply_ref_change(conn,org,repo,None,&update,forced,vec![],crate::protection::Source::Connection(source))?;
            crate::services::prs::on_push(app,conn,org,repo,None,std::slice::from_ref(&update))?;
        } else if let Some(item)=item {
            if item.kind != kind { return Err(Error::invalid("a destination summary cannot replace the full record")); }
            let mut data=item.data.clone();
            data["id"]=json!(if kind=="settings" {repo.id} else if kind=="status" {Uuid::now_v7()} else {key_id(key)?});
            data["actor"]=json!(item.actor);
            data["created_at"]=json!(crate::from_ms(item.created_at).to_rfc3339());
            if matches!(kind,"pr"|"comment"|"review") && item.actor.is_none() {return Err(Error::invalid("collaboration record has no author"));}
            let sql=match kind {
                "settings" => "UPDATE repos SET description=$2->>'description',default_branch=$2->>'default_branch',archived=($2->>'archived')::boolean WHERE id=$1",
                "pr" => {
                    let id=key_id(key)?;
                    let number:Option<i32>=pull_requests::table.find(id).select(pull_requests::number).first(conn).optional()?;
                    let number=match number {Some(n)=>n,None=>{
                        #[derive(QueryableByName)] struct N {#[diesel(sql_type=diesel::sql_types::Integer)] number:i32}
                        diesel::sql_query("INSERT INTO pr_counters(repo_id,next_number) VALUES($1,2) ON CONFLICT(repo_id) DO UPDATE SET next_number=pr_counters.next_number+1 RETURNING next_number-1 AS number").bind::<SqlUuid,_>(repo.id).get_result::<N>(conn)?.number
                    }};
                    data["number"]=json!(number);
                    "INSERT INTO pull_requests(id,repo_id,number,title,body,author_identity_id,head_repo_id,head_remote,head_ref,base_ref,head_sha,base_sha,state,merged_commit_sha,created_at,closed_at) VALUES(($2->>'id')::uuid,$1,($2->>'number')::integer,$2->>'title',$2->>'body',($2->>'actor')::uuid,$1,NULLIF($2->>'head_remote',''),$2->>'head_ref',$2->>'base_ref',$2->>'head_sha',$2->>'base_sha',$2->>'state',NULLIF($2->>'merge_sha',''),($2->>'created_at')::timestamptz,CASE WHEN $2->>'state'='open' THEN NULL ELSE now() END) ON CONFLICT(id) DO UPDATE SET title=EXCLUDED.title,body=EXCLUDED.body,head_ref=EXCLUDED.head_ref,base_ref=EXCLUDED.base_ref,head_sha=EXCLUDED.head_sha,base_sha=EXCLUDED.base_sha,state=EXCLUDED.state,merged_commit_sha=EXCLUDED.merged_commit_sha,merge_base_sha=CASE WHEN pull_requests.head_sha<>EXCLUDED.head_sha OR pull_requests.base_sha<>EXCLUDED.base_sha THEN NULL ELSE pull_requests.merge_base_sha END,updated_at=now(),closed_at=EXCLUDED.closed_at WHERE pull_requests.repo_id=$1"
                },
                "comment"|"review"=>{
                    let parent=key_id(text(&data,"parent"))?;
                    let valid=pull_requests::table.find(parent).filter(pull_requests::repo_id.eq(repo.id)).select(pull_requests::id).first::<Uuid>(conn).optional()?.is_some();
                    if !valid {return Err(Error::invalid("parent request is not in this repository"));}
                    data["parent_id"]=json!(parent);
                    if kind=="comment" {"INSERT INTO pr_comments(id,pr_id,author,body,path,line,commit_sha,created_at) VALUES(($2->>'id')::uuid,($2->>'parent_id')::uuid,($2->>'actor')::uuid,$2->>'body',$2->>'path',($2->>'line')::integer,$2->>'commit_sha',($2->>'created_at')::timestamptz) ON CONFLICT(id) DO UPDATE SET body=EXCLUDED.body,path=EXCLUDED.path,line=EXCLUDED.line,commit_sha=EXCLUDED.commit_sha,edited_at=now() WHERE pr_comments.pr_id IN (SELECT id FROM pull_requests WHERE repo_id=$1)"}
                    else {"INSERT INTO pr_reviews(id,pr_id,reviewer,state,head_sha_at_review,body,created_at) VALUES(($2->>'id')::uuid,($2->>'parent_id')::uuid,($2->>'actor')::uuid,$2->>'state',$2->>'head_sha',$2->>'body',($2->>'created_at')::timestamptz) ON CONFLICT(id) DO UPDATE SET state=EXCLUDED.state,body=EXCLUDED.body,head_sha_at_review=EXCLUDED.head_sha_at_review,base_sha_at_review=NULL,approval_scope=NULL WHERE pr_reviews.pr_id IN (SELECT id FROM pull_requests WHERE repo_id=$1)"}
                },
                "status"=>"INSERT INTO statuses(id,repo_id,sha,context,state,target_url,description,reporter,created_at) VALUES(($2->>'id')::uuid,$1,$2->>'sha',$2->>'context',$2->>'state',$2->>'target_url',$2->>'description',($2->>'actor')::uuid,($2->>'created_at')::timestamptz) ON CONFLICT(repo_id,sha,context) DO UPDATE SET state=EXCLUDED.state,target_url=EXCLUDED.target_url,description=EXCLUDED.description,reporter=EXCLUDED.reporter,created_at=EXCLUDED.created_at,base_sha=NULL",
                _=>return Err(Error::invalid("unsupported sync kind"))
            };
            diesel::sql_query(sql).bind::<SqlUuid,_>(repo.id).bind::<Jsonb,_>(data).execute(conn)?;
            if kind == "pr" {
                let pr: PrRow = pull_requests::table.find(key_id(key)?).select(PrRow::as_select()).first(conn)?;
                let dir = app.store.repo_dir(repo.id);
                let head = RevId(pr.head_sha.clone());
                if app.store.backend().object_exists(&dir,&head,&vec![]) {
                    let name = crate::services::prs::head_ref_name(pr.number);
                    let old = refs::get(conn,repo.id,&name)?;
                    refs::apply(conn,repo.id,&[RefUpdate { old, name, new: Some(head) }],item.actor)?;
                    if pr.state == "open" {
                        crate::services::prs::trial_merge(app,conn,repo,&dir,&pr)?;
                    } else {
                        crate::services::prs::settle_history_base(app,conn,&dir,&pr)?;
                    }
                }
            }
        } else {
            // Keep complete records before a native delete cascades to children.
            for (saved_key, saved) in snapshot(conn,repo)? {
                if saved_key == key || text(&saved.data,"parent") == key {
                    store::save_object(conn,repo.id,&saved_key,Some(&saved),source)?;
                    store::save_object(conn,repo.id,&saved_key,None,source)?;
                }
            }
            let sql=match kind {
                "comment"=>"DELETE FROM pr_comments WHERE id=$2 AND pr_id IN (SELECT id FROM pull_requests WHERE repo_id=$1)",
                "review"=>"DELETE FROM pr_reviews WHERE id=$2 AND pr_id IN (SELECT id FROM pull_requests WHERE repo_id=$1)",
                "pr"=>"DELETE FROM pull_requests WHERE repo_id=$1 AND id=$2",
                _=>return Ok(())
            };
            diesel::sql_query(sql).bind::<SqlUuid,_>(repo.id).bind::<SqlUuid,_>(key_id(key)?).execute(conn)?;
        }
        store::save_object(conn,repo.id,key,item,source)?;
        emit(conn,org,repo,"sync_applied",key,item.and_then(|i|i.actor))?;
        Ok(())
    })
    })
}
pub fn emit(
    conn: &mut PgConnection,
    org: &Org,
    repo: &Repo,
    kind: &str,
    key: &str,
    actor: Option<Uuid>,
) -> Result<(), Error> {
    outbox::emit(
        conn,
        outbox::Emit {
            lane: outbox::lane_for_repo(&org.name, &repo.name),
            kind,
            repo_id: Some(repo.id),
            org_id: Some(org.id),
            actor_id: actor,
            actor: None,
            repo: Some(vcs::repo_ref(org, repo)),
            org: Some(org.name.clone()),
            subject: Some(key.into()),
            payload: vcsstar_types::EventPayload::Variant5(vcsstar_types::SettingsPayload {
                changed: vec![key.into()],
            }),
        },
    )?;
    Ok(())
}
