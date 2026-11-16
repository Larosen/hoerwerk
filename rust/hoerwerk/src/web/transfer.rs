//! Settings export and import – for moving to a new server. The file holds the settings and the sources (indexers,
//! SABnzbd); API keys only if the export asked for them (then the file must be kept like a password). Accounts,
//! requests and the library are not part of it (they live in the database and the folders – see the backup).

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::text::{now, t};

type Params = HashMap<String, String>;

const FORMAT: i64 = 1;

pub async fn export(app: Arc<App>, req: Req, _: Params) -> Reply {
    let keys = matches!(req.query.get("keys").map(String::as_str), Some("1" | "true"));
    let file = json!({
        "hoerwerk_export": FORMAT,
        "version": crate::settings::version(),
        "exported": now(),
        "with_keys": keys,
        "settings": Value::Object(app.settings()),
        "sources": app.usenet.export_config(keys),
    });
    app.audit(
        &req.user_name(),
        &req.ip,
        "settings_export",
        if keys { "with keys" } else { "" },
    );
    let name = format!("hoerwerk-settings-{}.json", crate::text::today());
    Reply::Raw(
        StatusCode::OK,
        "application/json",
        Bytes::from(serde_json::to_vec_pretty(&file).unwrap_or_default()),
        vec![("content-disposition".into(), format!("attachment; filename=\"{name}\""))],
    )
}

pub async fn import(app: Arc<App>, req: Req, _: Params) -> Reply {
    let body = &req.body;
    if body["hoerwerk_export"].as_i64() != Some(FORMAT) {
        return error(400, t("Das ist keine Hörwerk-Exportdatei", &[]));
    }
    let data = body["settings"].as_object().cloned().unwrap_or_default();
    let clean = match app.settings.clean(&data) {
        Ok(c) => c,
        Err(key) => return error(400, t("Ungültiger Wert für {key}", &[("key", &key)])),
    };
    let n = clean.len();
    if let Err(e) = app.settings.update(&app.db, clean) {
        return error(500, e.to_string());
    }
    if let Some(lang) = app.settings().get("language").and_then(Value::as_str) {
        crate::text::set_language(lang);
    }
    let mut indexers = 0;
    if body["sources"].is_object() {
        let r = app.usenet.update_config(&body["sources"]);
        indexers = r["indexers"].as_array().map_or(0, Vec::len);
    }
    app.audit(
        &req.user_name(),
        &req.ip,
        "settings_import",
        &format!("{n} settings, {indexers} indexers"),
    );
    json_reply(json!({"ok": true, "settings": n, "indexers": indexers}))
}
