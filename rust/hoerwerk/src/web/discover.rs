//! Entdecken: suggested audiobooks – for the library, by genre, from book reviews.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::discover::{self, Env};
use crate::listening::Listening;
use crate::text::t;

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

fn section(value: &str) -> Result<String, Reply> {
    if discover::SECTIONS.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(error(400, t("Ungültige Daten", &[])))
    }
}

fn digits(genre: &str) -> bool {
    !genre.is_empty() && genre.chars().all(|c| c.is_ascii_digit())
}

fn env(app: &Arc<App>) -> Arc<dyn Env> {
    app.clone()
}

pub async fn suggestions(app: Arc<App>, req: Req, _: Params) -> Reply {
    let sec = match section(
        req.query
            .get("section")
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("foryou"),
    ) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let genre = req.query.get("genre").map_or("", |g| g.trim()).to_string();
    let uid = req.user_id();
    let a = app.clone();
    let res = blocking(move || {
        let e = env(&a);
        if sec == "genre" && !(digits(&genre) && discover::known_genre(&e, &genre)) {
            return json!({"items": [], "updated": null, "error": null, "running": false});
        }
        let mut res = discover::get(&e, &sec, &genre, Some(uid));
        let links = Listening::links(&a, uid);
        res["linked"] = json!(crate::discover::truthy(&links["abs"]) || crate::discover::truthy(&links["plex"]));
        let requested = a.requests.status_by_key(&a);
        let items: Vec<Value> = res["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| {
                let mut i = i.clone();
                i["requested"] = requested
                    .get(&crate::requests::book_key(&i))
                    .cloned()
                    .unwrap_or(Value::Null);
                i
            })
            .collect();
        res["items"] = json!(items);
        if sec == "critics" {
            res["waiting"] = json!(discover::waiting(&a.db));
        }
        res
    })
    .await;
    json_reply(res)
}

pub async fn genres(app: Arc<App>, _: Req, _: Params) -> Reply {
    let a = app.clone();
    let gs = blocking(move || discover::genres(&env(&a))).await;
    json_reply(json!({"genres": gs}))
}

pub async fn refresh(app: Arc<App>, req: Req, _: Params) -> Reply {
    let raw = req.str("section");
    let sec = match section(if raw.is_empty() { "foryou" } else { &raw }) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let genre = match req.get("genre") {
        Value::Null | Value::Bool(false) => String::new(),
        Value::String(g) => g.trim().to_string(),
        Value::Number(n) if n.as_f64() == Some(0.0) => String::new(),
        other => other.to_string().trim().to_string(),
    };
    let uid = req.user_id();
    let a = app.clone();
    let ok = blocking(move || {
        let e = env(&a);
        if sec == "genre" && !(digits(&genre) && discover::known_genre(&e, &genre)) {
            return false;
        }
        discover::refresh(&e, &sec, &genre, false, Some(uid));
        true
    })
    .await;
    if !ok {
        return error(400, t("Ungültige Daten", &[]));
    }
    json_reply(json!({"ok": true}))
}
