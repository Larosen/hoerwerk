//! The library: stats, browsing, covers, rescan, books that are not one M4B yet, and tidying up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use bytes::Bytes;
use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::library::{COVER_FILES, rel_of};
use crate::text::{self, t};
use crate::{chapters, media};

const MAX_COVER: u64 = 15 * 1024 * 1024;
const METADATA_FILE: &str = "metadata.json";
const LEFTOVERS: [&str; 2] = ["desc.txt", "reader.txt"];

fn root(app: &App) -> PathBuf {
    std::fs::canonicalize(&app.cfg.library).unwrap_or_else(|_| app.cfg.library.clone())
}

/// A book folder inside the library (no way out via .. or symlinks).
pub fn library_path(app: &App, rel: &str) -> Result<PathBuf, Reply> {
    let root = root(app);
    let gone = || error(404, t("Ordner nicht in der Bibliothek gefunden", &[]));
    let path = std::fs::canonicalize(root.join(rel.trim_matches('/'))).map_err(|_| gone())?;
    if path == root || !path.starts_with(&root) || !path.is_dir() {
        return Err(gone());
    }
    Ok(path)
}

fn query_usize(req: &Req, key: &str, default: usize) -> Result<usize, Reply> {
    match req.query.get(key).map(String::as_str) {
        None | Some("") => Ok(default),
        Some(v) => v
            .trim()
            .parse::<i64>()
            .map(|n| n.max(0) as usize)
            .map_err(|_| error(400, t("Ungültige Daten", &[]))),
    }
}

pub async fn stats(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    let mut st = app.library.stats();
    st["trickle"] = app.pipeline.trickle_status(&app);
    json_reply(st)
}

/// Switch the gentle conversion of the whole library on or off (admins).
pub async fn trickle(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let Some(on) = req.get("on").as_bool() else {
        return error(400, t("on muss true oder false sein", &[]));
    };
    app.pipeline.set_trickle(&app, on);
    app.audit(
        &req.user_name(),
        &req.ip,
        "library_trickle",
        if on { "on" } else { "off" },
    );
    json_reply(app.pipeline.trickle_status(&app))
}

pub async fn books(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let offset = match query_usize(&req, "offset", 0) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let limit = match query_usize(&req, "limit", 100) {
        Ok(v) => v.clamp(1, 2000),
        Err(r) => return r,
    };
    let q: String = req
        .query
        .get("q")
        .map(|q| q.chars().take(200).collect())
        .unwrap_or_default();
    let sort = req.query.get("sort").map(String::as_str).unwrap_or("");
    let issue = req.query.get("issue").map(String::as_str).unwrap_or("");
    // "select all": the paths of every book that matches, not only of the ones shown so far
    let ids_only = req.query.get("ids").is_some_and(|v| v == "1");
    let limit = if ids_only { 100_000 } else { limit };
    let reply = |v: Value| {
        if ids_only {
            json_reply(
                json!({"total": v["total"], "paths": v["books"].as_array().map(|b| b.iter().map(|x| x["path"].clone()).collect::<Vec<_>>()).unwrap_or_default()}),
            )
        } else {
            json_reply(v)
        }
    };
    reply(app.library.books(
        &q,
        offset,
        limit,
        sort,
        &crate::library::Extra {
            health: &app.health.snapshot(&app.ratings.runtimes()),
            ratings: &app.ratings.snapshot(),
            issue,
            admin: req.is_admin(),
            settled: &app.health.settled_by_hand(),
        },
    ))
}

fn image(data: Vec<u8>) -> Option<Reply> {
    let ctype = media::image_type(&data)?;
    Some(Reply::Raw(
        StatusCode::OK,
        ctype,
        Bytes::from(data),
        vec![("Cache-Control".into(), "private, max-age=86400".into())],
    ))
}

pub async fn cover(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let folder = match library_path(&app, req.query.get("path").map(String::as_str).unwrap_or("")) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let found = tokio::task::spawn_blocking(move || {
        for name in COVER_FILES {
            let p = folder.join(name);
            if media::is_regular(&p)
                && std::fs::metadata(&p).is_ok_and(|m| m.len() <= MAX_COVER)
                && let Some(r) = std::fs::read(&p).ok().and_then(image)
            {
                return Some(r);
            }
        }
        let first = media::collect_audio(&folder).into_iter().next()?;
        media::embedded_cover(&folder.join(first))
            .filter(|d| d.len() as u64 <= MAX_COVER)
            .and_then(image)
    })
    .await
    .ok()
    .flatten();
    found.unwrap_or_else(|| error(404, t("Kein Cover", &[])))
}

pub async fn scan(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = app.library.scan() {
            text::error(
                "Bibliothek",
                &t("Bibliothek konnte nicht eingelesen werden: {error}", &[("error", &e)]),
            );
        }
    });
    json_reply(json!({"ok": true}))
}

pub async fn unconverted(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    let mut entries: Vec<Value> = app
        .library
        .entries()
        .into_iter()
        .filter(|e| e["format"] != "m4b")
        .collect();
    entries.sort_by(|a, b| {
        crate::words::natural_path_cmp(a["path"].as_str().unwrap_or(""), b["path"].as_str().unwrap_or(""))
    });
    let keys = [
        "path",
        "title",
        "authors",
        "series",
        "series_position",
        "files",
        "format",
        "cover",
    ];
    let books: Vec<Value> = entries
        .iter()
        .map(|e| {
            let mut o: serde_json::Map<String, Value> = keys
                .iter()
                .map(|k| (k.to_string(), e.get(*k).cloned().unwrap_or(Value::Null)))
                .collect();
            let src = root(&app).join(e["path"].as_str().unwrap_or("")).display().to_string();
            let job = app
                .pipeline
                .all()
                .into_iter()
                .map(|j| j.lock().clone())
                .find(|j| j.source == src && !matches!(j.status.as_str(), "done" | "failed" | "cancelled"));
            o.insert(
                "job".into(),
                job.map_or(Value::Null, |j| json!({"id": j.id, "status": j.status})),
            );
            Value::Object(o)
        })
        .collect();
    json_reply(json!({"books": books, "scanning": app.library.scanning()}))
}

pub async fn tracks(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let folder = match library_path(&app, req.query.get("path").map(String::as_str).unwrap_or("")) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let rel = rel_of(&root(&app), &folder);
    let title = app
        .library
        .entry(&rel)
        .and_then(|e| e["title"].as_str().map(str::to_string))
        .unwrap_or_default();
    let lang = text::language();
    let out = tokio::task::spawn_blocking(move || {
        text::set_language(&lang);
        let mut out: Vec<Value> = media::collect_audio(&folder)
            .iter()
            .map(|f| {
                let info = media::probe_cached(&folder.join(f));
                let tag = info["tags"]["title"].as_str();
                json!({"file": f, "duration": info["duration"].as_f64().unwrap_or(0.0),
                       "auto": chapters::chapter_title(f, tag, "auto"),
                       "filename": chapters::chapter_title(f, tag, "filename"),
                       "tag": tag.unwrap_or("").trim()})
            })
            .collect();
        let autos: Vec<String> = out
            .iter()
            .map(|x| x["auto"].as_str().unwrap_or("").to_string())
            .collect();
        for (track, name) in out.iter_mut().zip(chapters::chapter_titles(&autos, &title)) {
            track["auto"] = json!(name);
        }
        out
    })
    .await
    .unwrap_or_default();
    json_reply(json!({"path": rel, "tracks": out}))
}

/// Paths (relative to the library) of redundant desc.txt/reader.txt: only in folders with a metadata.json.
fn leftovers(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&folder) else { continue };
        let mut dirs = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|f| f.is_dir()) && !name.starts_with('.') {
                dirs.push(name);
            }
        }
        dirs.sort();
        for d in dirs.iter().rev() {
            stack.push(folder.join(d));
        }
        if !media::is_regular(&folder.join(METADATA_FILE)) {
            continue;
        }
        for name in LEFTOVERS {
            let p = folder.join(name);
            if media::is_regular(&p) {
                out.push(rel_of(root, &p));
            }
        }
    }
    out
}

pub async fn cleanup_preview(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    let root = app.cfg.library.clone();
    let found = tokio::task::spawn_blocking(move || leftovers(&root))
        .await
        .unwrap_or_default();
    let folders: std::collections::BTreeSet<&str> =
        found.iter().map(|p| p.rsplit_once('/').map_or("", |x| x.0)).collect();
    json_reply(json!({"count": found.len(), "folders": folders.len(), "examples": &found[..found.len().min(5)]}))
}

pub async fn cleanup_run(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    let mut changes = serde_json::Map::new();
    changes.insert("write_desc".into(), json!(false));
    changes.insert("write_reader".into(), json!(false));
    let _ = app.settings.update(&app.db, changes);
    let root = app.cfg.library.clone();
    let (removed, errors) = tokio::task::spawn_blocking(move || {
        let (mut removed, mut errors) = (0, Vec::new());
        for rel in leftovers(&root) {
            match std::fs::remove_file(root.join(&rel)) {
                Ok(()) => removed += 1,
                Err(e) => errors.push(format!("{rel}: {e}")),
            }
        }
        (removed, errors)
    })
    .await
    .unwrap_or_default();
    text::info(
        "Bibliothek",
        &if removed == 1 {
            t("Aufgeräumt: 1 überflüssige Textdatei gelöscht", &[])
        } else {
            t(
                "Aufgeräumt: {n} überflüssige Textdateien gelöscht",
                &[("n", &removed.to_string())],
            )
        },
    );
    for e in &errors {
        text::warn("Bibliothek", &t("Nicht gelöscht: {error}", &[("error", e)]));
    }
    json_reply(json!({"removed": removed, "errors": &errors[..errors.len().min(20)]}))
}

/// Delete a book from the library (admins) – moved to `_ersetzt` or deleted, as set for replaced originals.
pub async fn delete_book(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let rel = req.query.get("path").cloned().unwrap_or_default();
    let a = app.clone();
    let res = tokio::task::spawn_blocking(move || {
        let r = a.pipeline.delete_from_library(&a, &rel);
        if r.is_ok() {
            let _ = a.library.scan();
        }
        r
    })
    .await
    .expect("worker thread");
    match res {
        Ok(shown) => {
            app.audit(&req.user_name(), &req.ip, "library_deleted", &shown);
            json_reply(json!({"ok": true, "path": shown}))
        }
        Err(e) => error(400, e.to_string()),
    }
}

/// `GET /api/library/check` – progress and what the check found, by kind (admins).
pub async fn check_status(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    json_reply(app.health.status(&app))
}

/// `POST /api/library/check {on}` – start the check of the library (books that are not checked yet or changed)
/// or stop a running one (admins).
pub async fn check_run(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let Some(on) = req.get("on").as_bool() else {
        return error(400, t("on muss true oder false sein", &[]));
    };
    if on {
        if app.health.start(&app) {
            text::info("Bibliothek", &t("Bibliotheks-Check gestartet", &[]));
        }
    } else {
        app.health.stop();
        app.autofix.stop();
    }
    app.audit(
        &req.user_name(),
        &req.ip,
        if on { "library_check" } else { "library_check_stop" },
        "",
    );
    json_reply(app.health.status(&app))
}

/// `GET /api/library/ratings` – how many books have an Audible rating, and the progress of fetching them (admins).
pub async fn ratings_status(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    json_reply(app.ratings.status(&app))
}

/// `POST /api/library/ratings {on}` – fetch Audible's ratings of the library's books in the background (the ones
/// missing or older than a month), or stop a running fetch (admins).
pub async fn ratings_run(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let Some(on) = req.get("on").as_bool() else {
        return error(400, t("on muss true oder false sein", &[]));
    };
    if on {
        if app.ratings.start(&app) {
            text::info("Bibliothek", &t("Audible-Daten werden abgefragt (im Hintergrund)", &[]));
        }
    } else {
        app.ratings.stop();
    }
    app.audit(
        &req.user_name(),
        &req.ip,
        if on { "library_ratings" } else { "library_ratings_stop" },
        "",
    );
    json_reply(app.ratings.status(&app))
}

/// `PUT /api/library/cover?path=<book folder>` – the picture of a book from the body (JPEG or PNG, up to 15 MB; admins):
/// as `cover.jpg`/`cover.png` in its folder and inside the M4B, the other tags stay. For books nobody offers a
/// cover for. The path, the name and the book's place in the library are not touched.
pub async fn put_cover(app: Arc<App>, mut req: Req, _: HashMap<String, String>) -> Reply {
    use http_body_util::{BodyExt, Limited};
    let rel = req.query.get("path").cloned().unwrap_or_default();
    let folder = match library_path(&app, &rel) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if req.length as u64 > MAX_COVER {
        return error(413, t("Bild zu groß (höchstens 15 MB)", &[]));
    }
    let Some(body) = req.raw.take() else {
        return error(411, t("Ungültige Länge", &[]));
    };
    let (user, ip) = (req.user_name(), req.ip.clone());
    drop(req); // the request (not Sync) must not live across the await
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        Limited::new(body, MAX_COVER as usize).collect(),
    )
    .await;
    let data = match read {
        Ok(Ok(c)) => c.to_bytes().to_vec(),
        Ok(Err(_)) => return error(413, t("Bild zu groß (höchstens 15 MB)", &[])),
        Err(_) => return error(408, t("Anfrage zu langsam", &[])),
    };
    let Some(ctype) = media::image_type(&data) else {
        return error(400, t("Das ist kein JPEG- oder PNG-Bild", &[]));
    };
    let (a, r) = (app.clone(), rel.clone());
    let done = tokio::task::spawn_blocking(move || set_cover(&a, &r, &folder, data, ctype))
        .await
        .unwrap_or_else(|_| Err("intern".to_string()));
    match done {
        Ok(()) => {
            app.audit(&user, &ip, "library_cover", &rel);
            json_reply(json!({"ok": true}))
        }
        Err(e) => error(500, e),
    }
}

fn set_cover(app: &Arc<App>, rel: &str, folder: &Path, data: Vec<u8>, ctype: &str) -> Result<(), String> {
    let name = if ctype == "image/png" { "cover.png" } else { "cover.jpg" };
    // one cover file: the other kind would win or confuse a media server
    for other in COVER_FILES.iter().filter(|f| **f != name && f.starts_with("cover.")) {
        let _ = std::fs::remove_file(folder.join(other));
    }
    std::fs::write(folder.join(name), &data).map_err(|e| e.to_string())?;
    // inside the M4B too (a single one): Plex and the players read it from there
    let audio = media::collect_audio(folder);
    if let [file] = audio.as_slice()
        && file.to_lowercase().ends_with(".m4b")
    {
        crate::tagger::set_cover(&folder.join(file), data)?;
    }
    app.library.refresh(rel);
    app.health.recheck(app, rel);
    // an own picture: this is the admin's own work (a recording or a conference nobody sells, a cover made by hand).
    // The picture stays, and the book is not matched, completed or flagged by itself – nothing replaces it later
    app.health.settle(&app.db, rel, &crate::health::SETTLEABLE, "cover");
    crate::mediaservers::request_scans(app);
    Ok(())
}

/// `POST /api/library/settle {paths, undo}` – the admin says "fine as it is" about books: no match, no details and
/// no length finding is shown for them any more and the automatic matching leaves them alone. `undo` takes the
/// decision back. Decided by hand, it stays – also when the files change. Admins.
pub async fn settle(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let undo = req.get("undo") == &Value::Bool(true);
    let paths: Vec<String> = req
        .get("paths")
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str().map(str::to_string))
                .take(5000)
                .collect()
        })
        .unwrap_or_default();
    if paths.is_empty() {
        return error(400, t("Ungültige Daten", &[]));
    }
    let mut changed = 0;
    for rel in paths.iter().filter(|p| app.library.entry(p).is_some()) {
        if undo {
            app.health.unsettle(&app.db, rel);
        } else {
            app.health.settle(&app.db, rel, &crate::health::SETTLEABLE, "admin");
        }
        changed += 1;
    }
    app.audit(
        &req.user_name(),
        &req.ip,
        if undo { "library_unsettled" } else { "library_settled" },
        &format!("{changed}"),
    );
    json_reply(json!({"ok": true, "changed": changed}))
}

/// `POST /api/library/enrich {path, apply}` – what the book lacks that Hörwerk is sure about (the edition is known by
/// its ASIN, or title, author and length fit closely): without `apply` only the list, with it the gaps are filled
/// (year, narrators, publisher, series, description, cover …; nothing that is there is changed). Admins.
pub async fn enrich(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let rel = req.str("path");
    if rel.is_empty() {
        return error(400, t("Kein Hörbuch angegeben", &[]));
    }
    if let Err(r) = library_path(&app, &rel) {
        return r;
    }
    let apply = req.get("apply") != &Value::Bool(false);
    let (a, r) = (app.clone(), rel.clone());
    let done = tokio::task::spawn_blocking(move || {
        if apply {
            crate::enrich::apply(&a, &r).map(|n| json!({"added": n}))
        } else {
            crate::enrich::plan(&a, &r)
        }
    })
    .await
    .unwrap_or_else(|_| Err("intern".to_string()));
    match done {
        Ok(v) => {
            if apply && v["added"].as_u64().unwrap_or(0) > 0 {
                app.audit(&req.user_name(), &req.ip, "library_enrich", &rel);
            }
            json_reply(v)
        }
        Err(e) => error(502, e),
    }
}

/// `POST /api/library/delete {paths}` – several books at once (admins): each goes as `DELETE /api/library/book`
/// does (moved to `_ersetzt` or deleted, as set), the library is read once at the end.
pub async fn delete_books(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let paths: Vec<String> = req.get("paths").as_array().map_or_else(Vec::new, |a| {
        a.iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .take(2000)
            .collect()
    });
    if paths.is_empty() {
        return error(400, t("Kein Hörbuch angegeben", &[]));
    }
    let a = app.clone();
    let (deleted, failed) = tokio::task::spawn_blocking(move || {
        let (mut deleted, mut failed) = (Vec::new(), Vec::new());
        for rel in paths {
            match a.pipeline.delete_from_library(&a, &rel) {
                Ok(shown) => deleted.push(shown),
                Err(e) => failed.push(json!({"path": rel, "error": e.to_string()})),
            }
        }
        if !deleted.is_empty() {
            let _ = a.library.scan();
        }
        (deleted, failed)
    })
    .await
    .unwrap_or_default();
    for shown in &deleted {
        app.audit(&req.user_name(), &req.ip, "library_deleted", shown);
    }
    json_reply(json!({"deleted": deleted.len(), "failed": failed}))
}
