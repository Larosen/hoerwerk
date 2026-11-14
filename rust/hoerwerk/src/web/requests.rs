//! Book requests, statistics, notifications, sources and downloads.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::requests::{Requests, may_search, sees_all, summary};
use crate::text::t;

type Params = HashMap<String, String>;

fn id_of(v: &str) -> Result<i64, Reply> {
    v.trim().parse().map_err(|_| error(404, t("Nicht gefunden", &[])))
}

fn user(req: &Req) -> Value {
    req.user.clone().unwrap_or(Value::Null)
}

fn is_admin(req: &Req) -> bool {
    req.user.as_ref().is_some_and(|u| u["role"] == "admin")
}

/// The request – only if this account may see it (others get 404, not 403: no guessing of ids).
fn visible(app: &App, req: &Req, rid: &str) -> Result<Value, Reply> {
    let id = id_of(rid)?;
    match app.requests.get(app, id) {
        Some(item) if Requests::can_see(&user(req), &item) => Ok(item),
        _ => Err(error(404, t("Nicht gefunden", &[]))),
    }
}

fn out(req: &Req, item: &Value) -> Value {
    let u = user(req);
    summary(item, &u, sees_all(&u))
}

macro_rules! tryr {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(r) => return r,
        }
    };
}

macro_rules! call {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return error(e.status, e.message),
        }
    };
}

/// Blocking work (indexers, SABnzbd, ntfy) off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let lang = crate::text::language();
    tokio::task::spawn_blocking(move || {
        crate::text::set_language(&lang);
        f()
    })
    .await
    .expect("worker thread")
}

/// SABnzbd's queue, asked at most every few seconds however many browsers poll the request list.
static SAB_SEEN: std::sync::Mutex<Option<(std::time::Instant, Value)>> = std::sync::Mutex::new(None);
const SAB_FRESH: std::time::Duration = std::time::Duration::from_secs(4);
const SAB_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

async fn sab_activity(app: &Arc<App>) -> Option<Value> {
    if let Some((when, seen)) = SAB_SEEN.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
        && when.elapsed() < SAB_FRESH
    {
        return Some(seen.clone());
    }
    let a = app.clone();
    // a slow SABnzbd must not hold up the list: without an answer in time the cards just show no progress
    let fresh = tokio::time::timeout(SAB_WAIT, blocking(move || a.usenet.activity(15)))
        .await
        .ok()?
        .ok()?;
    *SAB_SEEN.lock().unwrap_or_else(|p| p.into_inner()) = Some((std::time::Instant::now(), fresh.clone()));
    Some(fresh)
}

/// What a running request is doing right now: SABnzbd's progress while it loads, the pipeline's while it
/// is turned into an M4B. Only for requests in these states; nothing when SABnzbd does not answer.
async fn live(app: &Arc<App>, reqs: &[Value]) -> HashMap<i64, Value> {
    let mut out = HashMap::new();
    let sab = if reqs.iter().any(|r| r["status"] == "downloading") && app.usenet.has_sab() {
        sab_activity(app).await
    } else {
        None
    };
    for r in reqs {
        let Some(id) = r["id"].as_i64() else { continue };
        match r["status"].as_str() {
            Some("downloading") => {
                let Some(a) = &sab else { continue };
                let Some(i) = a["items"].as_array().and_then(|items| {
                    items
                        .iter()
                        .find(|i| i["id"].as_str().is_some_and(|x| !x.is_empty()) && i["id"] == r["nzo_id"])
                }) else {
                    continue;
                };
                let loading = i["done"] == false && i["status"] == "Downloading";
                out.insert(
                    id,
                    json!({"kind": "download", "state": i["status"], "done": i["done"], "progress": i["progress"],
                           "mb": i["mb"], "mbleft": i["mbleft"], "left": if loading { i["left"].clone() } else { Value::Null },
                           "speed": if loading { a["kbpersec"].clone() } else { Value::Null },
                           "paused": a["paused"]}),
                );
            }
            Some("processing") => {
                // no job yet: SABnzbd is done, the folder is not (yet) picked up from the inbox
                let (state, progress) = app.job_for_request(id).map_or((json!("inbox"), json!(0)), |j| {
                    (j["status"].clone(), j["progress"].clone())
                });
                if matches!(
                    state.as_str(),
                    Some("inbox" | "queued" | "matching" | "converting" | "tagging" | "review")
                ) {
                    out.insert(id, json!({"kind": "import", "state": state, "progress": progress}));
                }
            }
            _ => {}
        }
    }
    out
}

pub async fn list(app: Arc<App>, req: Req, _: Params) -> Reply {
    let u = user(&req);
    let all = sees_all(&u);
    let raw = app.requests.list(&app, &u, all);
    let now = live(&app, &raw).await;
    let items: Vec<Value> = raw
        .iter()
        .map(|r| {
            let mut o = out(&req, r);
            o["live"] = r["id"]
                .as_i64()
                .and_then(|id| now.get(&id))
                .cloned()
                .unwrap_or(Value::Null);
            o
        })
        .collect();
    json_reply(
        json!({"items": items, "quota": app.requests.quota(&app, &u), "sees_all": all, "may_search": may_search(&u),
                      "pending": if is_admin(&req) { json!(app.requests.pending_count(&app)) } else { Value::Null },
                      "interval": crate::requests::interval_hours(&app)}),
    )
}

/// A better release for a book of the library (admins): replaces it once it is there.
pub async fn upgrade(app: Arc<App>, req: Req, _: Params) -> Reply {
    let rel = match &req.get("path") {
        Value::String(s) => s.clone(),
        _ => return error(400, t("Kein Hörbuch angegeben", &[])),
    };
    if let Err(r) = super::library::library_path(&app, &rel) {
        return r;
    }
    let (a, u) = (app.clone(), user(&req));
    let item = call!(blocking(move || a.requests.create_upgrade(&a, &u, &rel)).await);
    json_reply(out(&req, &item))
}

pub async fn keys(app: Arc<App>, _: Req, _: Params) -> Reply {
    json_reply(json!({"keys": app.requests.status_by_key(&app)}))
}

pub async fn create(app: Arc<App>, req: Req, _: Params) -> Reply {
    if !req.get("book").is_object() {
        return error(400, t("Ungültige Daten", &[]));
    }
    let (u, book) = (user(&req), req.get("book").clone());
    let a = app.clone();
    let item = call!(blocking(move || a.requests.create(&a, &u, &book)).await);
    json_reply(out(&req, &item))
}

pub async fn create_series(app: Arc<App>, req: Req, _: Params) -> Reply {
    let (u, series, books) = (user(&req), req.get("series").clone(), req.get("books").clone());
    let a = app.clone();
    let res = call!(blocking(move || a.requests.create_series(&a, &u, &series, &books)).await);
    let items: Vec<Value> = res["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| out(&req, r))
        .collect();
    json_reply(json!({"group": res["group"], "status": res["status"], "skipped": res["skipped"], "items": items}))
}

pub async fn one(app: Arc<App>, req: Req, p: Params) -> Reply {
    let item = tryr!(visible(&app, &req, &p["rid"]));
    json_reply(out(&req, &item))
}

pub async fn withdraw(app: Arc<App>, req: Req, p: Params) -> Reply {
    let item = tryr!(visible(&app, &req, &p["rid"]));
    let whole = req.query.get("group").map(String::as_str) == Some("1");
    let n = call!(
        app.requests
            .withdraw(&app, item["id"].as_i64().unwrap_or(0), &user(&req), whole)
    );
    if is_admin(&req) && item["user_id"] != user(&req)["id"] {
        let more = if n > 1 { format!(" (+{})", n - 1) } else { String::new() };
        app.audit(
            &req.user_name(),
            &req.ip,
            "request_deleted",
            &format!("{} {}{more}", item["id"], item["book"]["title"].as_str().unwrap_or("")),
        );
    }
    json_reply(json!({"ok": true, "deleted": n}))
}

pub async fn approve(app: Arc<App>, req: Req, p: Params) -> Reply {
    let id = tryr!(id_of(&p["rid"]));
    let item = call!(app.requests.approve(&app, id, &user(&req)));
    app.audit(
        &req.user_name(),
        &req.ip,
        "request_approved",
        &format!("{} {}", item["id"], item["book"]["title"].as_str().unwrap_or("")),
    );
    json_reply(out(&req, &item))
}

pub async fn decline(app: Arc<App>, req: Req, p: Params) -> Reply {
    let id = tryr!(id_of(&p["rid"]));
    let item = call!(app.requests.decline(&app, id, &user(&req), &req.str("reason")));
    app.audit(
        &req.user_name(),
        &req.ip,
        "request_declined",
        &format!("{} {}", item["id"], item["book"]["title"].as_str().unwrap_or("")),
    );
    json_reply(out(&req, &item))
}

pub async fn retry(app: Arc<App>, req: Req, p: Params) -> Reply {
    let id = tryr!(id_of(&p["rid"]));
    let item = call!(app.requests.retry(&app, id));
    json_reply(out(&req, &item))
}

/// Interactive search: admins, and accounts with the right.
fn searcher(app: &App, req: &Req, rid: &str) -> Result<Value, Reply> {
    let item = visible(app, req, rid)?;
    if !may_search(&user(req)) {
        crate::authlog::event(
            "forbidden",
            &[
                ("user", &req.user_name()),
                ("ip", &req.ip),
                ("path", &format!("/api/requests/{rid}/releases")),
            ],
        );
        return Err(error(403, t("Dafür fehlen die Rechte", &[])));
    }
    Ok(item)
}

pub async fn releases(app: Arc<App>, req: Req, p: Params) -> Reply {
    let item = tryr!(searcher(&app, &req, &p["rid"]));
    let (a, id, q) = (app.clone(), item["id"].as_i64().unwrap_or(0), req.str("query"));
    let res = call!(blocking(move || a.requests.releases(&a, id, &q)).await);
    let results: Vec<Value> = res["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let mut r = r.clone();
            if let Some(o) = r.as_object_mut() {
                o.remove("link");
            }
            r
        })
        .collect();
    json_reply(
        json!({"query": res["query"], "queries": res["queries"], "errors": res["errors"], "resting": res["resting"],
               "results": results}),
    )
}

pub async fn grab(app: Arc<App>, req: Req, p: Params) -> Reply {
    let item = tryr!(searcher(&app, &req, &p["rid"]));
    let (a, id, rel, u) = (
        app.clone(),
        item["id"].as_i64().unwrap_or(0),
        req.str("release_id"),
        user(&req),
    );
    let title = json!({"title": req.str("title")});
    let res = call!(blocking(move || a.requests.grab(&a, id, &rel, Some(&u), Some(&title))).await);
    app.audit(
        &req.user_name(),
        &req.ip,
        "request_grab",
        &format!("{} {}", item["id"], res["release"].as_str().unwrap_or("")),
    );
    json_reply(out(&req, &res))
}

pub async fn block(app: Arc<App>, req: Req, p: Params) -> Reply {
    let item = tryr!(searcher(&app, &req, &p["rid"]));
    let title: String = req.str("title").chars().take(300).collect();
    let res = call!(app.requests.block(&app, item["id"].as_i64().unwrap_or(0), &title));
    json_reply(out(&req, &res))
}

// -- statistics -----------------------------------------------------------------------------------
fn period(req: &Req) -> Result<String, Reply> {
    let p = req
        .query
        .get("period")
        .filter(|p| !p.is_empty())
        .cloned()
        .unwrap_or_else(|| "365".into());
    if crate::stats::period_days(&p).is_none() {
        return Err(error(400, t("Ungültige Daten", &[])));
    }
    Ok(p)
}

pub async fn stats(app: Arc<App>, req: Req, _: Params) -> Reply {
    let p = tryr!(period(&req));
    let (a, admin, uid) = (app.clone(), is_admin(&req), req.user_id());
    let res = blocking(move || {
        if admin {
            a.stats.overview(&a, &p)
        } else {
            a.stats.personal(&a, uid, &p)
        }
    })
    .await;
    res.map(json_reply)
        .unwrap_or_else(|| error(400, t("Ungültige Daten", &[])))
}

pub async fn stats_account(app: Arc<App>, req: Req, p: Params) -> Reply {
    let wanted = tryr!(id_of(&p["uid"]));
    if !is_admin(&req) && wanted != req.user_id() {
        return error(404, t("Nicht gefunden", &[]));
    }
    let period = tryr!(period(&req));
    let a = app.clone();
    blocking(move || a.stats.account(&a, wanted, &period))
        .await
        .map(json_reply)
        .unwrap_or_else(|| error(404, t("Nicht gefunden", &[])))
}

// -- notifications --------------------------------------------------------------------------------
pub async fn notify_get(app: Arc<App>, _: Req, _: Params) -> Reply {
    json_reply(app.notifier.public_config())
}

pub async fn notify_put(app: Arc<App>, req: Req, _: Params) -> Reply {
    if !req.body.is_object() {
        return error(400, t("Ungültige Daten", &[]));
    }
    match app.notifier.update_config(&req.body) {
        Ok(v) => json_reply(v),
        Err(e) => error(400, e),
    }
}

pub async fn notify_test(app: Arc<App>, _: Req, _: Params) -> Reply {
    let a = app.clone();
    match blocking(move || a.notifier.test()).await {
        Ok(m) => json_reply(json!({"ok": true, "message": m})),
        Err(e) => json_reply(json!({"ok": false, "message": e})),
    }
}

/// Test message to the own ntfy topic (the saved one, or the one typed in).
pub async fn prefs_test(app: Arc<App>, req: Req, _: Params) -> Reply {
    let mut topic = req.str("ntfy_topic");
    if topic.is_empty() {
        let prefs: Option<String> = app
            .db
            .lock()
            .query_row("SELECT prefs FROM users WHERE id=?", [req.user_id()], |r| r.get(0))
            .ok();
        topic = prefs
            .and_then(|p| serde_json::from_str::<Value>(&p).ok())
            .and_then(|p| p["ntfy_topic"].as_str().map(str::to_string))
            .unwrap_or_default();
    }
    let (a, topic) = (app.clone(), topic.trim().to_string());
    match blocking(move || a.notifier.test_to(&topic)).await {
        Ok(m) => json_reply(json!({"ok": true, "message": m})),
        Err(e) => json_reply(json!({"ok": false, "message": e})),
    }
}

// -- sources and downloads --------------------------------------------------------------------------
fn with_servers(app: &App, mut v: Value) -> Value {
    v["abs"] = app.abs.public_config();
    v["plex"] = app.plex.public_config();
    v
}

pub async fn sources_get(app: Arc<App>, _: Req, _: Params) -> Reply {
    json_reply(with_servers(&app, app.usenet.public_config()))
}

pub async fn sources_put(app: Arc<App>, req: Req, _: Params) -> Reply {
    if !req.body.is_object() {
        return error(400, t("Ungültige Daten", &[]));
    }
    if req.body["abs"].is_object() {
        app.abs.update_config(&req.body["abs"]);
    }
    if req.body["plex"].is_object()
        && let Err(e) = app.plex.update_config(&req.body["plex"])
    {
        return error(400, e.message);
    }
    json_reply(with_servers(&app, app.usenet.update_config(&req.body)))
}

pub async fn sources_test(app: Arc<App>, req: Req, _: Params) -> Reply {
    let (a, kind, id) = (app.clone(), req.str("kind"), req.str("id"));
    let res = blocking(move || match kind.as_str() {
        "abs" => a.abs.test().map_err(|e| e.message),
        "plex" => a.plex.test().map_err(|e| e.message),
        _ => a.usenet.test(&kind, &id).map_err(|e| e.to_string()),
    })
    .await;
    match res {
        Ok(m) => json_reply(json!({"ok": true, "message": m})),
        Err(e) => json_reply(json!({"ok": false, "message": e})),
    }
}

pub async fn downloads(app: Arc<App>, req: Req, _: Params) -> Reply {
    if !app.usenet.has_sab() {
        return json_reply(json!({"configured": false, "items": []}));
    }
    let limit = req
        .query
        .get("limit")
        .filter(|l| !l.is_empty())
        .map_or(Ok(15), |l| l.trim().parse::<i64>())
        .map(|l| l.clamp(1, 200))
        .unwrap_or(15);
    let a = app.clone();
    match blocking(move || a.usenet.activity(limit as usize)).await {
        Ok(mut v) => {
            v["configured"] = json!(true);
            json_reply(v)
        }
        Err(e) => json_reply(json!({"configured": true, "error": e.to_string(), "items": []})),
    }
}

pub async fn downloads_clear(app: Arc<App>, req: Req, _: Params) -> Reply {
    if !app.usenet.has_sab() {
        return error(400, t("SABnzbd ist nicht eingerichtet", &[]));
    }
    let ids: Vec<String> = req
        .get("ids")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| match i {
            Value::String(s) => Some(s.chars().take(100).collect()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .take(200)
        .collect();
    let a = app.clone();
    let keep = app.requests.waiting_downloads(&app);
    match blocking(move || a.usenet.clear_finished(Some(ids), &keep)).await {
        Ok(v) => {
            app.audit(
                &req.user_name(),
                &req.ip,
                "downloads_cleared",
                &v["removed"].to_string(),
            );
            json_reply(v)
        }
        Err(e) => error(502, e.to_string()),
    }
}
