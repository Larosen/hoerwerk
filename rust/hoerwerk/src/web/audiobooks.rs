//! Audiobook pipeline: status, review (search, choose, own data), duplicates, retry.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::audible;
use crate::pipeline::{JobRef, PipeError};
use crate::text::t;

type Params = HashMap<String, String>;

fn job_or_404(app: &App, p: &Params) -> Result<JobRef, Reply> {
    app.pipeline
        .get(p.get("job_id").map(String::as_str).unwrap_or(""))
        .ok_or_else(|| error(404, t("Hörbuch nicht gefunden", &[])))
}

fn out(slot: &JobRef) -> Reply {
    json_reply(slot.lock().to_dict(false))
}

/// Blocking pipeline work off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("worker thread")
}

fn pipe_error(e: PipeError, refused: u16, failed: u16) -> Reply {
    match e {
        PipeError::NotFound => error(404, t("Hörbuch nicht gefunden", &[])),
        PipeError::Refused(m) => error(refused, m),
        other => error(failed, other.to_string()),
    }
}

fn region(app: &App, req: &Req) -> Result<String, Reply> {
    let r = req.str("region");
    let r = if r.is_empty() {
        app.settings()
            .get("ab_region")
            .and_then(Value::as_str)
            .unwrap_or("de")
            .to_string()
    } else {
        r
    };
    if !audible::REGIONS.iter().any(|x| x.0 == r) {
        return Err(error(400, t("Unbekannte Region", &[])));
    }
    Ok(r)
}

pub async fn status(app: Arc<App>, _: Req, _: Params) -> Reply {
    json_reply(app.snapshot("audiobooks"))
}

pub async fn paths(app: Arc<App>, req: Req, _: Params) -> Reply {
    let s = app.settings();
    let pick = |q: &str, k: &str| {
        req.query
            .get(q)
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| s.get(k).and_then(Value::as_str).unwrap_or("").to_string())
    };
    json_reply(
        json!({"examples": crate::filing::example(&pick("folder", "ab_folder_template"), &pick("file", "ab_file_template")),
                      "regions": audible::REGIONS.iter().map(|r| r.0).collect::<Vec<_>>()}),
    )
}

pub async fn scan(app: Arc<App>, _: Req, _: Params) -> Reply {
    app.pipeline.scan_now();
    json_reply(json!({"ok": true}))
}

pub async fn detail(app: Arc<App>, _: Req, p: Params) -> Reply {
    match job_or_404(&app, &p) {
        Ok(slot) => {
            let j = slot.lock();
            let mut d = j.to_dict(false);
            d["log"] = json!(j.log);
            json_reply(d)
        }
        Err(r) => r,
    }
}

pub async fn search(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let query = req.str("query").trim().to_string();
    if query.is_empty() {
        return error(400, t("Suchbegriff fehlt", &[]));
    }
    if query.to_lowercase().starts_with("http://") || query.to_lowercase().starts_with("https://") {
        return error(
            400,
            t("Bitte einen Link von Audible, BookBeat oder Thalia einfügen", &[])
                + " (Rust-Backend: noch nicht umgesetzt)",
        );
    }
    let r = match region(&app, &req) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.search(&a, &id, &query, &r)).await {
        Ok(slot) => out(&slot),
        Err(e) => pipe_error(e, 502, 502),
    }
}

/// Fetch book data from a shop link; returns the job and the new candidate.
pub async fn link(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let url = req.str("url").trim().to_string();
    if !crate::links::is_link(&url) {
        return error(
            400,
            t("Bitte einen Link von Audible, BookBeat oder Thalia einfügen", &[]),
        );
    }
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.add_link(&a, &id, &url)).await {
        Ok((slot, _)) => {
            let data = slot.lock().to_dict(false);
            let first = data["candidates"][0].clone();
            json_reply(json!({"job": data, "candidate": first}))
        }
        Err(e) => pipe_error(e, 400, 502),
    }
}

pub async fn duplicate(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let (a, id, action) = (app.clone(), p["job_id"].clone(), req.str("action"));
    match blocking(move || a.pipeline.resolve_duplicate(&a, &id, &action)).await {
        Ok(slot) => out(&slot),
        Err(PipeError::Failed(e)) => error(
            500,
            t("Konnte die alte Fassung nicht verschieben: {error}", &[("error", &e)]),
        ),
        Err(e) => pipe_error(e, 400, 500),
    }
}

pub async fn reopen(app: Arc<App>, _: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.reopen(&a, &id)).await {
        Ok(slot) => out(&slot),
        Err(PipeError::Failed(e)) => error(
            500,
            t("Konnte nicht aus der Bibliothek holen: {error}", &[("error", &e)]),
        ),
        Err(e) => pipe_error(e, 409, 500),
    }
}

pub async fn choose(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let cand = req.str("id").trim().to_string();
    let asin = req.str("asin").trim().to_uppercase();
    if cand.is_empty() && audible::find_asin(&[&asin]).as_deref() != Some(asin.as_str()) {
        return error(400, t("Ungültige ASIN", &[]));
    }
    let r = req.str("region");
    let r = if r.is_empty() {
        app.settings()
            .get("ab_region")
            .and_then(Value::as_str)
            .unwrap_or("de")
            .to_string()
    } else {
        r
    };
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.choose(&a, &id, &cand, &asin, &r)).await {
        Ok(slot) => out(&slot),
        Err(PipeError::NotFound) => error(404, t("Treffer nicht gefunden – bitte neu suchen", &[])),
        Err(e) => pipe_error(e, 502, 502),
    }
}

pub async fn manual(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let data = if req.body.is_object() {
        req.body.clone()
    } else {
        json!({})
    };
    let base = match &data["base_id"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.manual(&a, &id, &data, &base)).await {
        Ok(slot) => out(&slot),
        Err(e) => pipe_error(e, 400, 500),
    }
}

pub async fn retry(app: Arc<App>, _: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.retry(&a, &id)).await {
        Ok(job) => json_reply(job),
        Err(PipeError::Failed(e)) => error(
            500,
            t(
                "Konnte nicht zurück in den Eingang verschieben: {error}",
                &[("error", &e)],
            ),
        ),
        Err(e) => pipe_error(e, 400, 500),
    }
}

pub async fn cancel(app: Arc<App>, _: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    match app.pipeline.cancel(&app, &p["job_id"]) {
        Ok(slot) => out(&slot),
        Err(e) => pipe_error(e, 400, 500),
    }
}

pub async fn remove(app: Arc<App>, _: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    match app.pipeline.remove(&app, &p["job_id"]) {
        Ok(()) => json_reply(json!({"ok": true})),
        Err(e) => pipe_error(e, 409, 500),
    }
}

pub async fn delete_source(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    let (a, id) = (app.clone(), p["job_id"].clone());
    match blocking(move || a.pipeline.delete_from_inbox(&a, &id)).await {
        Ok(name) => {
            app.audit(&req.user_name(), &req.ip, "inbox_deleted", &name);
            json_reply(json!({"ok": true, "name": name}))
        }
        Err(PipeError::Failed(e)) => error(500, t("Löschen fehlgeschlagen: {error}", &[("error", &e)])),
        Err(e) => pipe_error(e, 409, 500),
    }
}

pub async fn ignore(app: Arc<App>, req: Req, p: Params) -> Reply {
    if let Err(r) = job_or_404(&app, &p) {
        return r;
    }
    match app.pipeline.ignore(&app, &p["job_id"]) {
        Ok(name) => {
            app.audit(&req.user_name(), &req.ip, "inbox_ignored", &name);
            json_reply(json!({"ok": true, "name": name}))
        }
        Err(e) => pipe_error(e, 409, 500),
    }
}

pub async fn unignore(app: Arc<App>, req: Req, _: Params) -> Reply {
    json_reply(json!({"ok": true, "count": app.pipeline.unignore(&app, &req.str("name"))}))
}

pub async fn clear(app: Arc<App>, _: Req, _: Params) -> Reply {
    app.pipeline.clear_finished(&app);
    json_reply(json!({"ok": true}))
}

/// Queue library books for conversion: [{path, chapter_names?, rename_files?}].
pub async fn convert(app: Arc<App>, req: Req, _: Params) -> Reply {
    let Some(items) = req.get("items").as_array().filter(|i| !i.is_empty()).cloned() else {
        return error(400, t("Keine Bücher ausgewählt", &[]));
    };
    let mut jobs = Vec::new();
    for it in items.iter().take(200).filter(|i| i.is_object()) {
        let rel = match &it["path"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let folder = match super::library::library_path(&app, &rel) {
            Ok(f) => f,
            Err(r) => return r,
        };
        let mut names: Option<Vec<String>> = None;
        if !it["chapter_names"].is_null() {
            let Some(list) = it["chapter_names"]
                .as_array()
                .filter(|l| l.iter().all(Value::is_string))
            else {
                return error(400, t("chapter_names muss eine Liste von Texten sein", &[]));
            };
            if list.len() != crate::media::collect_audio(&folder).len() {
                return error(409, t("Die Tracks haben sich geändert – bitte neu laden", &[]));
            }
            names = Some(
                list.iter()
                    .map(|n| n.as_str().unwrap_or("").trim().chars().take(200).collect())
                    .collect(),
            );
        }
        let rename = it["rename_files"].as_bool().unwrap_or(false) && names.is_some();
        let slot = app.pipeline.add(&app, &folder, "library", names, rename, None);
        jobs.push(slot.lock().to_dict(false));
    }
    json_reply(json!({"jobs": jobs}))
}
