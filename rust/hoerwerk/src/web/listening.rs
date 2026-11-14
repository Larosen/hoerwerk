//! Listening history: link Hörwerk accounts with Audiobookshelf accounts and Plex profiles.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::listening::Listening;
use crate::text::{now, t};

type Params = HashMap<String, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let lang = crate::text::language();
    tokio::task::spawn_blocking(move || {
        crate::text::set_language(&lang);
        f()
    })
    .await
    .expect("worker thread")
}

fn kind_ok(k: &str) -> Result<String, Reply> {
    if matches!(k, "abs" | "plex") {
        Ok(k.to_string())
    } else {
        Err(error(404, t("Nicht gefunden", &[])))
    }
}

pub async fn mine(app: Arc<App>, req: Req, _: Params) -> Reply {
    let uid = req.user_id();
    let links = Listening::links(&app, uid);
    let mut out = json!({"available": Listening::available(&app), "links": links, "recent": null});
    if !links["abs"].is_null() || !links["plex"].is_null() {
        let a = app.clone();
        out["recent"] = blocking(move || a.listening.summary(&a, uid, now() - 30.0 * 86400.0)).await;
    }
    json_reply(out)
}

/// Sign in to Audiobookshelf once – wrong passwords count on the same brake as Hörwerk's login.
pub async fn link_abs(app: Arc<App>, req: Req, _: Params) -> Reply {
    if app.accounts.blocked(&req.ip) {
        return error(
            429,
            t("Zu viele Fehlversuche – bitte in 15 Minuten erneut versuchen", &[]),
        );
    }
    let (a, uid, user, pw) = (
        app.clone(),
        req.user_id(),
        req.str("username").trim().to_string(),
        req.str("password"),
    );
    match blocking(move || a.listening.link_own_abs(&a, uid, &user, &pw)).await {
        Ok(links) => {
            app.audit(
                &req.user_name(),
                &req.ip,
                "listening_linked",
                &format!("abs {}", links["abs"]["name"].as_str().unwrap_or("")),
            );
            json_reply(json!({"links": links}))
        }
        Err((from_abs, e)) => {
            if from_abs && (e.contains("falsch") || e.contains("wrong")) {
                app.accounts.fail(&req.ip, &req.user_name());
                crate::authlog::event(
                    "login_failed",
                    &[("user", &req.user_name()), ("ip", &req.ip), ("reason", "bad_abs_login")],
                );
            }
            error(400, e)
        }
    }
}

pub async fn unlink(app: Arc<App>, req: Req, p: Params) -> Reply {
    let kind = match kind_ok(p.get("kind").map(String::as_str).unwrap_or("")) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match app.listening.set_link(&app, req.user_id(), &kind, None) {
        Ok(links) => json_reply(json!({"links": links})),
        Err(e) => error(400, e),
    }
}

pub async fn accounts(app: Arc<App>, _: Req, _: Params) -> Reply {
    let avail = Listening::available(&app);
    let users: Vec<Value> = app
        .accounts
        .list(&app.db)
        .iter()
        .map(|u| json!({"id": u["id"], "name": u["name"], "links": Listening::links(&app, u["id"].as_i64().unwrap_or(0))}))
        .collect();
    let a = app.clone();
    let (abs, plex, errors) = blocking(move || {
        let mut errors = Vec::new();
        let abs = if avail["abs"] == true {
            a.abs.users().map(|u| json!(u)).unwrap_or_else(|e| {
                errors.push(e.message);
                Value::Null
            })
        } else {
            Value::Null
        };
        let plex = if avail["plex"] == true {
            a.plex.accounts().map(|u| json!(u)).unwrap_or_else(|e| {
                errors.push(e.message);
                Value::Null
            })
        } else {
            Value::Null
        };
        (abs, plex, errors)
    })
    .await;
    json_reply(
        json!({"available": Listening::available(&app), "abs": abs, "plex": plex, "errors": errors, "users": users,
               "people": Listening::people(&app)}),
    )
}

/// Merge an Audiobookshelf account and a Plex profile that belong to the same person (no Hörwerk account needed).
pub async fn merge(app: Arc<App>, req: Req, _: Params) -> Reply {
    let (abs_id, plex_id, name) = (
        req.str("abs").trim().to_string(),
        req.str("plex").trim().to_string(),
        req.str("name"),
    );
    let a = app.clone();
    let (targets, errors) = blocking(move || a.listening.targets(&a, true)).await;
    if let Some(e) = errors.first() {
        return error(502, e.clone());
    }
    let find = |kind: &str, id: &str| -> Result<Option<Value>, Reply> {
        if id.is_empty() {
            return Ok(None);
        }
        targets[kind]
            .as_array()
            .and_then(|a| a.iter().find(|t| t["id"].as_str() == Some(id)).cloned())
            .map(Some)
            .ok_or_else(|| error(400, t("Ungültige Daten", &[])))
    };
    let (abs, plex) = match (find("abs", &abs_id), find("plex", &plex_id)) {
        (Ok(a), Ok(p)) => (a, p),
        (Err(r), _) | (_, Err(r)) => return r,
    };
    match app.listening.merge(&app, abs.as_ref(), plex.as_ref(), &name) {
        Ok(people) => {
            app.audit(
                &req.user_name(),
                &req.ip,
                "listening_merge",
                &format!("abs {abs_id} + plex {plex_id}"),
            );
            json_reply(json!({"people": people}))
        }
        Err(e) => error(400, e),
    }
}

pub async fn split(app: Arc<App>, req: Req, p: Params) -> Reply {
    let Some(id) = p.get("id").and_then(|x| x.parse::<i64>().ok()) else {
        return error(400, t("Ungültige Daten", &[]));
    };
    match app.listening.split(&app, id) {
        Ok(people) => {
            app.audit(&req.user_name(), &req.ip, "listening_split", &id.to_string());
            json_reply(json!({"people": people}))
        }
        Err(e) => error(400, e),
    }
}

pub async fn set_link(app: Arc<App>, req: Req, _: Params) -> Reply {
    let kind = match kind_ok(&req.str("kind")) {
        Ok(k) => k,
        Err(r) => return r,
    };
    let Some(uid) = req
        .get("user_id")
        .as_i64()
        .or_else(|| req.get("user_id").as_str().and_then(|x| x.trim().parse().ok()))
    else {
        return error(400, t("Ungültige Daten", &[]));
    };
    let target_id = req.str("target_id");
    let mut target = None;
    if !target_id.is_empty() {
        let (a, k) = (app.clone(), kind.clone());
        let choices = blocking(move || if k == "abs" { a.abs.users() } else { a.plex.accounts() }).await;
        match choices {
            Err(e) => return error(502, e.message),
            Ok(list) => match list.into_iter().find(|c| c["id"].as_str() == Some(target_id.as_str())) {
                Some(c) => target = Some(c),
                None => return error(400, t("Ungültige Daten", &[])),
            },
        }
    }
    match app.listening.set_link(&app, uid, &kind, target.as_ref()) {
        Ok(links) => {
            let name = target
                .as_ref()
                .map_or("–".to_string(), |x| x["name"].as_str().unwrap_or("").to_string());
            app.audit(
                &req.user_name(),
                &req.ip,
                "listening_link",
                &format!("{uid} {kind} {name}"),
            );
            json_reply(json!({"links": links}))
        }
        Err(e) => error(400, e),
    }
}
