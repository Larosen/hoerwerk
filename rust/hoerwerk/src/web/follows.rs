//! Following authors and narrators (every account follows for itself).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::follows;
use crate::text::t;

type Params = HashMap<String, String>;

fn uid(req: &Req) -> i64 {
    req.user_id()
}

fn fid(p: &Params) -> Option<i64> {
    p.get("fid").and_then(|v| v.trim().parse().ok())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task")
}

pub async fn list(app: Arc<App>, req: Req, _: Params) -> Reply {
    let items: Vec<Value> = follows::list(&app, uid(&req)).iter().map(follows::public).collect();
    json_reply(json!({"follows": items}))
}

pub async fn add(app: Arc<App>, req: Req, _: Params) -> Reply {
    let auto = req.get("auto").as_bool().unwrap_or(true);
    match follows::add(
        &app,
        uid(&req),
        &req.str("kind"),
        &req.str("name"),
        &req.str("language"),
        auto,
    ) {
        Ok(f) => json_reply(follows::public(&f)),
        Err(e) => error(400, e),
    }
}

pub async fn update(app: Arc<App>, req: Req, p: Params) -> Reply {
    let Some(id) = fid(&p) else {
        return error(404, t("Nicht gefunden", &[]));
    };
    let ok = match req.get("auto").as_bool() {
        Some(auto) => follows::set_auto(&app, uid(&req), id, auto),
        None => {
            follows::seen_fresh(&app, uid(&req), id);
            true
        }
    };
    if ok {
        json_reply(json!({"ok": true}))
    } else {
        error(404, t("Nicht gefunden", &[]))
    }
}

pub async fn remove(app: Arc<App>, req: Req, p: Params) -> Reply {
    let Some(id) = fid(&p) else {
        return error(404, t("Nicht gefunden", &[]));
    };
    if follows::remove(&app, uid(&req), id) {
        json_reply(json!({"ok": true}))
    } else {
        error(404, t("Nicht gefunden", &[]))
    }
}

/// The person's books for the window: what is in the library or asked for already is marked.
pub async fn works(app: Arc<App>, req: Req, _: Params) -> Reply {
    let (kind, name, language) = match follows::validate(&req.str("kind"), &req.str("name"), &req.str("language")) {
        Ok(v) => v,
        Err(e) => return error(400, e),
    };
    let region = app
        .settings()
        .get("ab_region")
        .and_then(Value::as_str)
        .unwrap_or("de")
        .to_string();
    let (a, k, n, l) = (app.clone(), kind.clone(), name.clone(), language.clone());
    match blocking(move || follows::works(&a, &k, &n, &l, &region)).await {
        Ok(books) => {
            let follow = follows::find(&app, uid(&req), &kind, &name).map(|f| follows::public(&f));
            json_reply(
                json!({"kind": kind, "name": name, "language": language, "follow": follow,
                              "books": super::search::for_browser(&app, &req, &books)}),
            )
        }
        Err(e) => error(502, e),
    }
}
