//! Upload audiobooks from the browser into the inbox, e.g. ripped from CD (routes/upload.py).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::text::t;

type Params = HashMap<String, String>;

fn owner(req: &Req) -> String {
    format!("u{}", req.user_id())
}

pub async fn start(app: Arc<App>, req: Req, _: Params) -> Reply {
    match app.uploads.start(&owner(&req), &req.user_name()) {
        Ok(id) => json_reply(json!({"id": id})),
        Err(e) => error(400, e.0),
    }
}

/// One file; its path inside the upload (e.g. "Autor - Titel/CD1/01.mp3") in ?path=.
pub async fn put_file(app: Arc<App>, mut req: Req, p: Params) -> Reply {
    let Some(body) = req.raw.take() else {
        return error(411, t("Ungültige Länge", &[]));
    };
    let path = req.query.get("path").cloned().unwrap_or_default();
    let (length, who) = (req.length as u64, owner(&req));
    drop(req); // the request (not Sync) must not live across the await
    match app.uploads.put(&p["uid"], &path, body, length, &who).await {
        Ok(v) => json_reply(v),
        Err(e) => error(400, e.0),
    }
}

pub async fn finish(app: Arc<App>, req: Req, p: Params) -> Reply {
    let (a, uid, who) = (app.clone(), p["uid"].clone(), owner(&req));
    let names = match tokio::task::spawn_blocking(move || a.uploads.finish(&uid, &who))
        .await
        .expect("worker thread")
    {
        Ok(n) => n,
        Err(e) => return error(400, e.0),
    };
    if names.is_empty() {
        return error(400, t("Keine Dateien hochgeladen", &[]));
    }
    app.pipeline.arrived(&names);
    app.audit(&req.user_name(), &req.ip, "upload", &names.join(", "));
    json_reply(json!({"ok": true, "names": names}))
}

pub async fn cancel(app: Arc<App>, req: Req, p: Params) -> Reply {
    match app.uploads.cancel(&p["uid"], &owner(&req)) {
        Ok(()) => json_reply(json!({"ok": true})),
        Err(e) => error(400, e.0),
    }
}
