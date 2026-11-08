//! The library: stats, browsing, covers, rescan, books that are not one M4B yet, and tidying up (routes/library.py).

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
    json_reply(app.library.stats())
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
    json_reply(app.library.books(&q, offset, limit))
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
