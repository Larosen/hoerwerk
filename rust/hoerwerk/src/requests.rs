//! Book requests: a user asks for a book, Hörwerk searches, downloads and files it.
//!
//!     pending ──approve──▶ searching ──grab──▶ downloading ──SABnzbd done──▶ processing ──filed──▶ available
//!        │                    ▲   (nothing found: again every few hours)             │
//!        └──decline──▶ declined   └──── download failed / import didn't fit: release blocked, next one ◀┘

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use rand::RngCore;
use rusqlite::OptionalExtension;
use serde_json::{Map, Value, json};

use crate::app::App;
use crate::db::compact;
use crate::releases;
use crate::text::{self, now, t};
use crate::usenet::UsenetError;
use crate::words::match_words;

pub const OPEN: [&str; 4] = ["pending", "searching", "downloading", "processing"];
const BOOK_KEYS: [&str; 19] = [
    "asin",
    "title",
    "subtitle",
    "authors",
    "narrators",
    "series",
    "series_position",
    "runtime_min",
    "release_date",
    "cover_url",
    "url",
    "source",
    "region",
    "isbn",
    "language",
    "format_type",
    "id",
    "genres",
    "series_asin",
];
const MAX_ATTEMPTS: i64 = 5;
const DOWNLOAD_TIMEOUT: f64 = 2.0 * 86400.0;
const SERIES_MAX: usize = 60;
const ROW_KEYS: [&str; 8] = [
    "id", "user_id", "user", "book", "book_key", "status", "created", "updated",
];

#[derive(Debug)]
pub struct RequestError {
    pub message: String,
    pub status: u16,
}

fn err<T>(message: String, status: u16) -> Result<T, RequestError> {
    Err(RequestError { message, status })
}

type Res<T> = Result<T, RequestError>;

pub fn book_key(book: &Value) -> String {
    if let Some(a) = book["asin"].as_str().filter(|a| !a.is_empty()) {
        return format!("asin:{}", a.to_uppercase());
    }
    let first = book["authors"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    format!(
        "book:{}|{}",
        match_words(book["title"].as_str().unwrap_or("")).join(" "),
        releases::surname(first)
    )
}

fn empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn is_admin(user: &Value) -> bool {
    user["role"] == "admin"
}

fn perm(user: &Value, p: &str) -> bool {
    user["perms"][p].as_bool().unwrap_or(false)
}

pub fn sees_all(user: &Value) -> bool {
    is_admin(user) || perm(user, "see_all_requests")
}

pub fn may_search(user: &Value) -> bool {
    is_admin(user) || perm(user, "interactive_search")
}

fn auto_approve(user: &Value) -> bool {
    is_admin(user) || perm(user, "auto_approve")
}

/// Only what the shops tell about a book, cut to size; title and author are required.
/// "M4B · 64 kbit/s" of the book that is there.
fn quality_label(have: &Value) -> String {
    let format = text_of(&have["format"]).to_uppercase();
    match have["kbps"].as_i64() {
        Some(k) if !format.is_empty() => format!("{format} · {k} kbit/s"),
        Some(k) => format!("{k} kbit/s"),
        None => format,
    }
}

pub fn clean(book: &Value) -> Res<Value> {
    let Some(obj) = book.as_object() else {
        return err(t("Ungültige Daten", &[]), 400);
    };
    let mut c: Map<String, Value> = BOOK_KEYS
        .iter()
        .filter_map(|k| obj.get(*k).filter(|v| !empty(v)).map(|v| (k.to_string(), v.clone())))
        .collect();
    c.insert(
        "title".into(),
        json!(cut(text_of(c.get("title").unwrap_or(&Value::Null)).trim(), 300)),
    );
    let authors: Vec<Value> = match c.get("authors") {
        Some(Value::Array(a)) => a.iter().take(5).map(|x| json!(cut(&text_of(x), 120))).collect(),
        Some(Value::String(s)) => s.chars().take(5).map(|x| json!(x.to_string())).collect(),
        _ => Vec::new(),
    };
    c.insert("authors".into(), json!(authors));
    if c["title"].as_str().unwrap_or("").is_empty() || authors.is_empty() {
        return err(t("Titel und Autor fehlen", &[]), 400);
    }
    if let Some(a) = c.get("asin").filter(|v| !empty(v)) {
        let a = cut(&text_of(a).to_uppercase(), 20);
        c.insert("asin".into(), json!(a));
    }
    for k in ["series", "series_position", "series_asin", "language", "region"] {
        if let Some(v) = c.get(k).filter(|v| !v.is_null()) {
            let v = cut(&text_of(v), 200);
            c.insert(k.into(), json!(v));
        }
    }
    for k in ["cover_url", "url"] {
        if c.get(k).is_some_and(|v| !text_of(v).starts_with("https://")) {
            c.remove(k);
        }
    }
    Ok(Value::Object(c))
}

/// (judged releases, queries asked, errors)
/// (judged releases, queries asked, errors, indexers resting until midnight)
type Found = (Vec<Value>, Vec<String>, Vec<String>, Vec<String>);

pub struct Requests {
    lock: Mutex<()>,
    busy: Mutex<HashSet<i64>>,
}

impl Default for Requests {
    fn default() -> Self {
        Self::new()
    }
}

impl Requests {
    pub fn new() -> Self {
        Requests {
            lock: Mutex::new(()),
            busy: Mutex::new(HashSet::new()),
        }
    }

    // -- storage -------------------------------------------------------------------------------
    fn row(app: &App, r: &rusqlite::Row) -> rusqlite::Result<Value> {
        let data: Value =
            serde_json::from_str(&r.get::<_, Option<String>>("data")?.unwrap_or_default()).unwrap_or(json!({}));
        let uid: Option<i64> = r.get("user_id")?;
        let mut out = data.as_object().cloned().unwrap_or_default();
        out.insert("id".into(), json!(r.get::<_, i64>("id")?));
        out.insert("user_id".into(), json!(uid));
        out.insert("_uid".into(), json!(uid));
        out.insert(
            "book".into(),
            serde_json::from_str(&r.get::<_, String>("book")?).unwrap_or(json!({})),
        );
        out.insert("book_key".into(), json!(r.get::<_, String>("book_key")?));
        out.insert("status".into(), json!(r.get::<_, String>("status")?));
        out.insert("created".into(), json!(r.get::<_, f64>("created")?));
        out.insert("updated".into(), json!(r.get::<_, f64>("updated")?));
        let _ = app;
        Ok(Value::Object(out))
    }

    /// Fill in the requester's name (outside the database lock).
    fn named(app: &App, mut v: Value) -> Value {
        let uid = v["_uid"].as_i64();
        let name = uid
            .filter(|u| *u != 0)
            .and_then(|u| app.accounts.get(&app.db, u))
            .and_then(|u| u["name"].as_str().map(str::to_string));
        if let Some(o) = v.as_object_mut() {
            o.remove("_uid");
            o.insert("user".into(), json!(name.unwrap_or_else(|| "–".into())));
        }
        v
    }

    fn query(app: &App, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Vec<Value> {
        let rows: Vec<Value> = {
            let c = app.db.lock();
            let Ok(mut st) = c.prepare(sql) else { return Vec::new() };
            st.query_map(params, |r| Self::row(app, r))
                .map(|it| it.filter_map(Result::ok).collect())
                .unwrap_or_default()
        };
        rows.into_iter().map(|v| Self::named(app, v)).collect()
    }

    pub fn get(&self, app: &App, rid: i64) -> Option<Value> {
        let v = app
            .db
            .lock()
            .query_row("SELECT * FROM requests WHERE id=?", [rid], |r| Self::row(app, r))
            .optional()
            .ok()
            .flatten()?;
        Some(Self::named(app, v))
    }

    fn save(&self, app: &App, req: &Value, status: Option<&str>, note: &str) -> Value {
        let status = status.map(str::to_string).unwrap_or_else(|| text_of(&req["status"]));
        let mut data: Map<String, Value> = req
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(k, _)| !ROW_KEYS.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if !note.is_empty() {
            let mut hist: Vec<Value> = data
                .get("history")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let skip = hist.len().saturating_sub(29);
            hist.drain(..skip);
            hist.push(json!({"time": now(), "text": note}));
            data.insert("history".into(), json!(hist));
        }
        let id = req["id"].as_i64().unwrap_or(0);
        let _ = app.db.lock().execute(
            "UPDATE requests SET status=?, updated=?, data=? WHERE id=?",
            rusqlite::params![status, now(), compact(&Value::Object(data)), id],
        );
        self.get(app, id).unwrap_or(Value::Null)
    }

    // -- rights and quota -----------------------------------------------------------------------
    pub fn can_see(user: &Value, req: &Value) -> bool {
        sees_all(user) || req["user_id"] == user["id"]
    }

    pub fn quota(&self, app: &App, user: &Value) -> Value {
        if is_admin(user) {
            return json!({"limit": null, "used": 0, "left": null});
        }
        let limit = user["quota"].as_i64().unwrap_or_else(|| {
            app.settings()
                .get("request_quota")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or(5)
        });
        // the volumes of a series request count once
        let used: i64 = app
            .db
            .lock()
            .query_row(
                "SELECT COUNT(DISTINCT COALESCE(json_extract(data, '$.group.id'), 'r' || id)) FROM requests \
                 WHERE user_id=? AND created>=? AND status<>'declined'",
                rusqlite::params![user["id"].as_i64(), text::local_month_start(now())],
                |r| r.get(0),
            )
            .unwrap_or(0);
        json!({"limit": limit, "used": used, "left": (limit - used).max(0)})
    }

    pub fn list(&self, app: &App, user: &Value, everyone: bool) -> Vec<Value> {
        if everyone && sees_all(user) {
            Self::query(app, "SELECT * FROM requests ORDER BY created DESC LIMIT 500", &[])
        } else {
            let uid = user["id"].as_i64();
            Self::query(
                app,
                "SELECT * FROM requests WHERE user_id=? ORDER BY created DESC LIMIT 500",
                &[&uid],
            )
        }
    }

    /// Book key -> status of its newest request (for "requested" marks in Entdecken and the search).
    pub fn status_by_key(&self, app: &App) -> Map<String, Value> {
        let c = app.db.lock();
        let mut out = Map::new();
        if let Ok(mut st) = c.prepare("SELECT book_key, status FROM requests ORDER BY created") {
            for (k, s) in st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .into_iter()
                .flatten()
                .flatten()
            {
                out.insert(k, json!(s));
            }
        }
        out
    }

    pub fn pending_count(&self, app: &App) -> i64 {
        app.db
            .lock()
            .query_row("SELECT COUNT(*) FROM requests WHERE status='pending'", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// ASINs of books on their way (downloading or processing).
    pub fn open_asins(&self, app: &App) -> HashSet<String> {
        let c = app.db.lock();
        let Ok(mut st) = c.prepare("SELECT book_key FROM requests WHERE status IN ('downloading','processing')") else {
            return HashSet::new();
        };
        st.query_map([], |r| r.get::<_, String>(0))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|k| k.strip_prefix("asin:").map(str::to_string))
            .collect()
    }

    // -- create / decide ------------------------------------------------------------------------
    fn owned(app: &App, book: &Value) -> bool {
        let asin = book["asin"].as_str().unwrap_or("").to_uppercase();
        if !asin.is_empty()
            && app
                .library
                .entries()
                .iter()
                .any(|e| e["asin"].as_str() == Some(asin.as_str()))
        {
            return true;
        }
        !app.library.find(book).is_empty()
    }

    fn is_open(c: &rusqlite::Connection, key: &str) -> bool {
        c.query_row(
            "SELECT 1 FROM requests WHERE book_key=? AND status IN (?,?,?,?)",
            rusqlite::params![key, OPEN[0], OPEN[1], OPEN[2], OPEN[3]],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
    }

    fn check_quota(&self, app: &App, user: &Value) -> Res<()> {
        let q = self.quota(app, user);
        if let Some(limit) = q["limit"].as_i64()
            && q["left"].as_i64().unwrap_or(0) <= 0
        {
            return err(
                t("Monatslimit erreicht ({n} Anfragen)", &[("n", &limit.to_string())]),
                429,
            );
        }
        Ok(())
    }

    pub fn create(&self, app: &Arc<App>, user: &Value, book: &Value) -> Res<Value> {
        let clean = clean(book)?;
        if Self::owned(app, &clean) {
            return err(t("Das Hörbuch ist schon in der Bibliothek", &[]), 409);
        }
        let key = book_key(&clean);
        let ts = now();
        let name = text_of(&user["name"]);
        let (rid, status) = {
            let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            if Self::is_open(&app.db.lock(), &key) {
                return err(t("Das Hörbuch ist schon angefragt", &[]), 409);
            }
            self.check_quota(app, user)?;
            let status = if auto_approve(user) { "searching" } else { "pending" };
            let data = json!({"history": [{"time": ts, "text": t("Angefragt von {name}", &[("name", &name)])}], "blocked": [], "attempts": 0});
            let c = app.db.lock();
            c.execute(
                "INSERT INTO requests(user_id, book_key, book, status, created, updated, data) VALUES(?,?,?,?,?,?,?)",
                rusqlite::params![
                    user["id"].as_i64(),
                    key,
                    compact(&clean),
                    status,
                    ts,
                    ts,
                    compact(&data)
                ],
            )
            .map_err(|e| RequestError {
                message: e.to_string(),
                status: 500,
            })?;
            (c.last_insert_rowid(), status)
        };
        let req = self.get(app, rid).unwrap_or(Value::Null);
        text::info(
            "Anfragen",
            &t(
                "Anfrage: „{title}“ von {name}",
                &[("title", clean["title"].as_str().unwrap_or("")), ("name", &name)],
            ),
        );
        if status == "pending" {
            app.notifier.send(
                "request_new",
                &t("Neue Buchanfrage", &[]),
                &format!("{name}: {}", label(&req)),
                None,
            );
        } else {
            search_soon(app, vec![rid]);
        }
        Ok(req)
    }

    /// Look again for a book that is there: a better release replaces it. The search runs like a request –
    /// automatically it takes only a clearly better release, in the release window any pick counts.
    pub fn create_upgrade(&self, app: &Arc<App>, user: &Value, rel: &str) -> Res<Value> {
        let Some(entry) = app
            .library
            .entries()
            .into_iter()
            .find(|e| e["path"].as_str() == Some(rel))
        else {
            return err(t("Nicht in der Bibliothek", &[]), 404);
        };
        let mut book = json!({});
        for k in ["title", "authors", "narrators", "series", "series_position", "asin"] {
            book[k] = entry[k].clone();
        }
        let asin = text_of(&entry["asin"]);
        let region = app
            .settings()
            .get("ab_region")
            .and_then(Value::as_str)
            .unwrap_or("de")
            .to_string();
        if !asin.is_empty()
            && let Ok(full) = app.providers.client.product(&asin, &region)
        {
            book = full; // runtime, narrators and release date: for the bitrate and the right edition
        }
        let clean = clean(&book)?;
        let key = format!("upgrade:{}", book_key(&clean));
        let mut have = crate::pipeline::Pipeline::quality_of(&app.pipeline.library.join(rel));
        if have.is_object() {
            have["format"] = entry["format"].clone();
        }
        let ts = now();
        let name = text_of(&user["name"]);
        let rid = {
            let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            if Self::is_open(&app.db.lock(), &key) {
                return err(
                    t("Für dieses Hörbuch wird schon ein besseres Release gesucht", &[]),
                    409,
                );
            }
            let data = json!({"history": [{"time": ts, "text": t("Besseres Release gesucht von {name}", &[("name", &name)])}],
                              "blocked": [], "attempts": 0, "upgrade": {"path": rel, "have": have}});
            let c = app.db.lock();
            c.execute(
                "INSERT INTO requests(user_id, book_key, book, status, created, updated, data) VALUES(?,?,?,?,?,?,?)",
                rusqlite::params![
                    user["id"].as_i64(),
                    key,
                    compact(&clean),
                    "searching",
                    ts,
                    ts,
                    compact(&data)
                ],
            )
            .map_err(|e| RequestError {
                message: e.to_string(),
                status: 500,
            })?;
            c.last_insert_rowid()
        };
        text::info(
            "Anfragen",
            &t(
                "Besseres Release gesucht: „{title}“ ({path})",
                &[("title", clean["title"].as_str().unwrap_or("")), ("path", rel)],
            ),
        );
        search_soon(app, vec![rid]);
        Ok(self.get(app, rid).unwrap_or(Value::Null))
    }

    /// Several volumes of one series as one request: one group, counted once, decided together.
    /// Several books as one request: the volumes of a series, or works of an author or narrator
    /// (`series.kind` = "author" | "narrator", `series.title` = the name). They count once for the monthly limit.
    pub fn create_series(&self, app: &Arc<App>, user: &Value, series: &Value, books: &Value) -> Res<Value> {
        let kind = match series["kind"].as_str() {
            Some("author") => "author",
            Some("narrator") => "narrator",
            _ => "series",
        };
        let (Some(_), Some(list)) = (series.as_object(), books.as_array().filter(|b| !b.is_empty())) else {
            return err(t("Bitte mindestens einen Band auswählen", &[]), 400);
        };
        if list.len() > SERIES_MAX {
            return err(
                t("Höchstens {n} Bände auf einmal", &[("n", &SERIES_MAX.to_string())]),
                400,
            );
        }
        let mut title: String = cut(
            &text_of(&series["title"])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            200,
        );
        let cleaned: Vec<Value> = list.iter().map(clean).collect::<Res<_>>()?;
        if title.is_empty() {
            title = cleaned[0]["series"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(cleaned[0]["title"].as_str().unwrap_or(""))
                .to_string();
        }
        let ts = now();
        let mut b = [0u8; 4];
        rand::rng().fill_bytes(&mut b);
        let asin = cut(&text_of(&series["asin"]), 20);
        let mut group = json!({"id": format!("g{}-{}-{}", (ts * 1000.0) as i64, text_of(&user["id"]), hex::encode(b)), "title": title,
                               "asin": if asin.is_empty() { Value::Null } else { json!(asin) }});
        if kind != "series" {
            group["kind"] = json!(kind);
        }
        let mut skipped: Vec<String> = Vec::new();
        let mut todo = Vec::new();
        for mut b in cleaned {
            if kind == "series" && empty(&b["series"]) {
                b["series"] = json!(title);
            }
            if Self::owned(app, &b) {
                skipped.push(text_of(&b["title"]));
            } else {
                todo.push(b);
            }
        }
        let name = text_of(&user["name"]);
        let (ids, status, fresh_first) = {
            let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            let mut fresh: Vec<(String, Value)> = Vec::new();
            let mut keys = HashSet::new();
            for b in todo {
                let key = book_key(&b);
                if keys.contains(&key) || Self::is_open(&app.db.lock(), &key) {
                    skipped.push(text_of(&b["title"]));
                    continue;
                }
                keys.insert(key.clone());
                fresh.push((key, b));
            }
            if fresh.is_empty() {
                return err(
                    if kind == "series" {
                        t("Alle ausgewählten Bände sind schon da oder angefragt", &[])
                    } else {
                        t("Alle ausgewählten Hörbücher sind schon da oder angefragt", &[])
                    },
                    409,
                );
            }
            self.check_quota(app, user)?;
            let status = if auto_approve(user) { "searching" } else { "pending" };
            group["count"] = json!(fresh.len());
            let mut c = app.db.lock();
            let tx = c.transaction().map_err(|e| RequestError {
                message: e.to_string(),
                status: 500,
            })?;
            let mut ids = Vec::new();
            for (key, b) in &fresh {
                let why = match kind {
                    "author" => t(
                        "Angefragt von {name} (Hörbücher von {person})",
                        &[("name", &name), ("person", &title)],
                    ),
                    "narrator" => t(
                        "Angefragt von {name} (gelesen von {person})",
                        &[("name", &name), ("person", &title)],
                    ),
                    _ => t(
                        "Angefragt von {name} (Reihe „{series}“)",
                        &[("name", &name), ("series", &title)],
                    ),
                };
                let data = json!({"history": [{"time": ts, "text": why}],
                                  "blocked": [], "attempts": 0, "group": group});
                tx.execute(
                    "INSERT INTO requests(user_id, book_key, book, status, created, updated, data) VALUES(?,?,?,?,?,?,?)",
                    rusqlite::params![user["id"].as_i64(), key, compact(b), status, ts, ts, compact(&data)],
                )
                .map_err(|e| RequestError { message: e.to_string(), status: 500 })?;
                ids.push(tx.last_insert_rowid());
            }
            tx.commit().map_err(|e| RequestError {
                message: e.to_string(),
                status: 500,
            })?;
            (ids, status, fresh[0].1.clone())
        };
        let author = fresh_first["authors"]
            .as_array()
            .map(|a| a.iter().map(text_of).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let n = ids.len().to_string();
        let lbl = match kind {
            "author" => t("{n} Hörbücher von {person}", &[("n", &n), ("person", &title)]),
            "narrator" => t("{n} Hörbücher gelesen von {person}", &[("n", &n), ("person", &title)]),
            _ => t(
                "{author} – Reihe „{series}“ ({n} Bände)",
                &[("author", &author), ("series", &title), ("n", &n)],
            ),
        };
        text::info(
            "Anfragen",
            &t(
                "Sammel-Anfrage: {label} von {name}",
                &[("label", &lbl), ("name", &name)],
            ),
        );
        if status == "pending" {
            app.notifier.send(
                "request_new",
                &if kind == "series" {
                    t("Neue Reihen-Anfrage", &[])
                } else {
                    t("Neue Sammel-Anfrage", &[])
                },
                &format!("{name}: {lbl}"),
                None,
            );
        } else {
            search_soon(app, ids.clone());
        }
        let items: Vec<Value> = ids.iter().filter_map(|i| self.get(app, *i)).collect();
        Ok(json!({"group": group, "status": status, "items": items, "skipped": skipped}))
    }

    /// All volumes of the series request this request belongs to (just itself without one).
    pub fn group(&self, app: &App, req: &Value) -> Vec<Value> {
        match req["group"]["id"].as_str().filter(|g| !g.is_empty()) {
            None => vec![req.clone()],
            Some(gid) => Self::query(
                app,
                "SELECT * FROM requests WHERE json_extract(data, '$.group.id')=? ORDER BY id",
                &[&gid],
            ),
        }
    }

    fn need(&self, app: &App, rid: i64) -> Res<Value> {
        self.get(app, rid).ok_or_else(|| RequestError {
            message: t("Nicht gefunden", &[]),
            status: 404,
        })
    }

    pub fn approve(&self, app: &Arc<App>, rid: i64, admin: &Value) -> Res<Value> {
        let req = self.need(app, rid)?;
        if req["status"] != "pending" {
            return err(t("Die Anfrage wartet nicht auf Freigabe", &[]), 409);
        }
        let name = text_of(&admin["name"]);
        let mut ids = Vec::new();
        for mut item in self.group(app, &req) {
            if item["status"] == "pending" {
                item["decided_by"] = json!(name);
                self.save(
                    app,
                    &item,
                    Some("searching"),
                    &t("Freigegeben von {name}", &[("name", &name)]),
                );
                ids.push(item["id"].as_i64().unwrap_or(0));
            }
        }
        search_soon(app, ids);
        self.need(app, rid)
    }

    pub fn decline(&self, app: &App, rid: i64, admin: &Value, reason: &str) -> Res<Value> {
        let req = self.need(app, rid)?;
        if !matches!(req["status"].as_str(), Some("pending" | "searching" | "failed")) {
            return err(t("Die Anfrage läuft schon – erst zurückziehen", &[]), 409);
        }
        let reason = cut(reason.trim(), 300);
        let name = text_of(&admin["name"]);
        let mut note = t("Abgelehnt von {name}", &[("name", &name)]);
        if !reason.is_empty() {
            note = format!("{note}: {reason}");
        }
        let group = self.group(app, &req);
        for mut item in group.clone() {
            if item["id"].as_i64() == Some(rid) || (item["status"] == "pending" && req["status"] == "pending") {
                item["decided_by"] = json!(name);
                item["reason"] = json!(reason);
                self.save(app, &item, Some("declined"), &note);
            }
        }
        let req = self.need(app, rid)?;
        let lbl = if group.len() == 1 {
            label(&req)
        } else {
            group_label(&req)
        };
        self.tell(
            app,
            &req,
            &t("Anfrage abgelehnt", &[]),
            &if reason.is_empty() {
                lbl
            } else {
                format!("{lbl}\n{reason}")
            },
        );
        Ok(req)
    }

    /// Delete the request (with `whole_group`: every volume of its series that may be withdrawn).
    pub fn withdraw(&self, app: &App, rid: i64, user: &Value, whole_group: bool) -> Res<usize> {
        let req = self.need(app, rid)?;
        let mut items = if whole_group {
            self.group(app, &req)
        } else {
            vec![req.clone()]
        };
        if !is_admin(user) {
            if req["user_id"] != user["id"] {
                return err(t("Nicht gefunden", &[]), 404);
            }
            let may = ["pending", "searching", "failed", "declined"];
            let running = || err(t("Der Download läuft schon – zurückziehen geht nicht mehr", &[]), 409);
            if !whole_group && !may.contains(&req["status"].as_str().unwrap_or("")) {
                return running();
            }
            items.retain(|i| may.contains(&i["status"].as_str().unwrap_or("")) && i["user_id"] == user["id"]);
            if items.is_empty() {
                return running();
            }
        }
        for i in &items {
            let _ = app
                .db
                .lock()
                .execute("DELETE FROM requests WHERE id=?", [i["id"].as_i64()]);
        }
        Ok(items.len())
    }

    pub fn retry(&self, app: &Arc<App>, rid: i64) -> Res<Value> {
        let mut req = self.need(app, rid)?;
        if !matches!(req["status"].as_str(), Some("failed" | "searching")) {
            return err(t("Die Anfrage läuft gerade", &[]), 409);
        }
        for k in ["error", "last_search"] {
            req[k] = Value::Null;
        }
        req["attempts"] = json!(0);
        let req = self.save(app, &req, Some("searching"), &t("Neuer Versuch", &[]));
        search_soon(app, vec![rid]);
        Ok(req)
    }

    // -- searching and grabbing ---------------------------------------------------------------------
    fn judge(app: &App, req: &Value, results: &[Value]) -> Vec<Value> {
        let unabridged = app
            .settings()
            .get("discover_unabridged")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let lang = releases::book_language(&req["book"]);
        let lang = if lang.is_empty() { "de".to_string() } else { lang };
        let blocked: Vec<String> = req["blocked"].as_array().into_iter().flatten().map(text_of).collect();
        releases::rank(results, &req["book"], &lang, unabridged, &blocked)
    }

    /// (judged releases, queries asked, errors). A typed query is searched as it is (in all spellings);
    /// otherwise round by round – broader only while nothing fits.
    fn find(app: &App, req: &Value, query: Option<&str>) -> Result<Found, UsenetError> {
        let res = match query {
            Some(q) => app.usenet.search(q, "", &[], true, 6, false)?,
            None => {
                let fits = |found: &[Value]| Self::judge(app, req, found).iter().any(|r| r["verdict"]["ok"] == true);
                app.usenet.search_stages(
                    &releases::search_stages(&req["book"]),
                    &fits,
                    "",
                    releases::SEARCH_BUDGET,
                    false,
                )?
            }
        };
        let results = res["results"].as_array().cloned().unwrap_or_default();
        let strs = |k: &str| res[k].as_array().into_iter().flatten().map(text_of).collect::<Vec<_>>();
        Ok((
            Self::judge(app, req, &results),
            strs("queries"),
            strs("errors"),
            strs("resting"),
        ))
    }

    /// All releases for a request with the verdict on each (interactive search). Blocking.
    pub fn releases(&self, app: &App, rid: i64, query: &str) -> Res<Value> {
        let req = self.need(app, rid)?;
        if !app.usenet.has_indexers() {
            return err(t("Noch kein Indexer eingerichtet", &[]), 400);
        }
        let typed = cut(query.trim(), 200);
        let typed = (!typed.is_empty()).then_some(typed);
        let (mut ranked, queries, errors, resting) =
            Self::find(app, &req, typed.as_deref()).map_err(|e| RequestError {
                message: e.to_string(),
                status: 502,
            })?;
        let q = typed.unwrap_or_else(|| {
            releases::search_stages(&req["book"])
                .first()
                .and_then(|s| s.first())
                .cloned()
                .unwrap_or_default()
        });
        let queries = if queries.is_empty() { vec![q.clone()] } else { queries };
        let have = &req["upgrade"]["have"];
        if !have.is_null() {
            // a better release for a book that is there: say which ones would be an improvement
            let (yes, no) = (t("besser als vorhanden", &[]), t("nicht besser als vorhanden", &[]));
            for r in ranked.iter_mut().filter(|r| r["verdict"]["ok"] == true) {
                let note = if releases::better_than(r, have) { &yes } else { &no };
                if let Some(n) = r["verdict"]["notes"].as_array_mut() {
                    n.insert(0, json!(note));
                }
            }
        }
        Ok(
            json!({"request": req, "query": q, "queries": queries, "errors": errors, "resting": resting,
                  "results": ranked}),
        )
    }

    /// Another volume of the same series is loading a pack that holds this volume too.
    fn coming_with_pack(&self, app: &App, req: &Value) -> Option<Value> {
        let vol = releases::volumes(&req["book"])?;
        if req["group"].is_null() {
            return None;
        }
        self.group(app, req).into_iter().find(|o| {
            o["id"] != req["id"]
                && matches!(o["status"].as_str(), Some("downloading" | "processing"))
                && o["pack_volumes"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|x| x.as_i64() == Some(vol)))
        })
    }

    /// Search the indexers; the best accepted release goes to SABnzbd right away. Blocking.
    pub fn search(&self, app: &App, rid: i64) -> Option<Value> {
        {
            let mut busy = self.busy.lock().unwrap_or_else(|p| p.into_inner());
            if !busy.insert(rid) {
                return None;
            }
        }
        let out = self.search_inner(app, rid);
        self.busy.lock().unwrap_or_else(|p| p.into_inner()).remove(&rid);
        out
    }

    fn search_inner(&self, app: &App, rid: i64) -> Option<Value> {
        let mut req = self.get(app, rid)?;
        if req["status"] != "searching" {
            return Some(req);
        }
        if req["upgrade"].is_null() && Self::owned(app, &req["book"]) {
            return Some(self.save(app, &req, Some("available"), &t("Schon in der Bibliothek", &[])));
        }
        if !app.usenet.has_indexers() {
            req["last_search"] = json!(now());
            req["error"] = json!(t("Noch kein Indexer eingerichtet", &[]));
            return Some(self.save(app, &req, None, ""));
        }
        if let Some(carrier) = self.coming_with_pack(app, &req) {
            let release = text_of(&carrier["release"]);
            let note = if req["pack_noted"].as_str() == Some(release.as_str()) {
                String::new()
            } else {
                t("Kommt mit dem Paket „{release}“", &[("release", &release)])
            };
            req["last_search"] = json!(now());
            req["error"] = Value::Null;
            req["pack_noted"] = json!(release);
            return Some(self.save(app, &req, None, &note));
        }
        let (ranked, queries, errors, resting) = match Self::find(app, &req, None) {
            Ok(x) => x,
            Err(e) => {
                req["last_search"] = json!(now());
                req["error"] = json!(e.to_string());
                return Some(self.save(app, &req, None, ""));
            }
        };
        let fits = ranked.iter().filter(|r| r["verdict"]["ok"] == true).count();
        let have = req["upgrade"]["have"].clone();
        // a better release for a book that is there: automatically only what is clearly better
        let good: Vec<&Value> = ranked
            .iter()
            .filter(|r| r["verdict"]["ok"] == true && (have.is_null() || releases::better_than(r, &have)))
            .collect();
        req["last_search"] = json!(now());
        req["error"] = if errors.is_empty() {
            Value::Null
        } else {
            json!(errors.join("; "))
        };
        req["resting"] = if resting.is_empty() {
            Value::Null
        } else {
            json!(resting)
        };
        req["found"] = json!(good.len());
        req["seen"] = json!(ranked.len());
        req["queries"] = json!(queries);
        if good.is_empty() && fits > 0 && !have.is_null() {
            let note = t(
                "{n} passende Releases, keins besser als das vorhandene ({have}) – neuer Versuch in {h} h",
                &[
                    ("n", &fits.to_string()),
                    ("have", &quality_label(&have)),
                    ("h", &interval_hours(app).to_string()),
                ],
            );
            return Some(self.save(app, &req, None, &note));
        }
        if good.is_empty() {
            let note = t(
                "Nichts Passendes gefunden ({n} Releases geprüft, {q} Suchbegriffe) – neuer Versuch in {h} h",
                &[
                    ("n", &ranked.len().to_string()),
                    ("q", &queries.len().to_string()),
                    ("h", &interval_hours(app).to_string()),
                ],
            );
            return Some(self.save(app, &req, None, &note));
        }
        if !app.usenet.has_sab() {
            req["error"] = json!(t("SABnzbd ist nicht eingerichtet", &[]));
            let note = if good.len() == 1 {
                t("1 passendes Release gefunden", &[])
            } else {
                t("{n} passende Releases gefunden", &[("n", &good.len().to_string())])
            };
            return Some(self.save(app, &req, None, &note));
        }
        let best = good[0].clone();
        // what the search found is kept (found/seen/queries) before the grab reads the request again
        self.save(app, &req, None, "");
        match self.grab(app, rid, best["id"].as_str().unwrap_or(""), None, Some(&best)) {
            Ok(r) => Some(r),
            Err(e) => {
                text::warn(
                    "Anfragen",
                    &t(
                        "Anfrage {id}: Suche fehlgeschlagen: {error}",
                        &[("id", &rid.to_string()), ("error", &e.message)],
                    ),
                );
                self.get(app, rid)
            }
        }
    }

    pub fn grab(
        &self,
        app: &App,
        rid: i64,
        release_id: &str,
        by: Option<&Value>,
        release: Option<&Value>,
    ) -> Res<Value> {
        let mut req = self.need(app, rid)?;
        if matches!(req["status"].as_str(), Some("downloading" | "processing" | "available")) {
            return err(t("Für diese Anfrage läuft schon ein Download", &[]), 409);
        }
        let book = req["book"].clone();
        let authors = book["authors"]
            .as_array()
            .map(|a| a.iter().map(text_of).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let mut hints = json!({"title": book["title"], "author": authors, "asin": book["asin"], "request_id": rid});
        let vol = releases::volumes(&book);
        let name = release.map(|r| text_of(&r["title"])).unwrap_or_default();
        let pack = if name.is_empty() {
            None
        } else {
            crate::packs::pack_info(&name)
        };
        req["pack_volumes"] = Value::Null;
        if let (Some(p), Some(v)) = (&pack, vol) {
            // a pack: keep this volume – and the other volumes of the same series request that are still sought
            let in_pack: Vec<i64> = p["volumes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .collect();
            let mut wanted = std::collections::BTreeSet::from([v]);
            for other in self.group(app, &req) {
                if let Some(ov) = releases::volumes(&other["book"])
                    && other["id"].as_i64() != Some(rid)
                    && other["status"] == "searching"
                    && in_pack.contains(&ov)
                {
                    wanted.insert(ov);
                }
            }
            hints["volumes"] = json!(wanted);
            req["pack_volumes"] = json!(wanted);
        }
        let sent = app.usenet.send(release_id, &hints).map_err(|e| RequestError {
            message: e.to_string(),
            status: 400,
        })?;
        let title = text_of(&sent["title"]);
        req["release"] = json!(title);
        req["nzo_id"] = sent["nzo_id"].clone();
        req["sent_at"] = json!(now());
        req["error"] = Value::Null;
        req["attempts"] = json!(req["attempts"].as_i64().unwrap_or(0) + 1);
        let who = by
            .map(|u| t(" (von Hand: {name})", &[("name", &text_of(&u["name"]))]))
            .unwrap_or_default();
        let req = self.save(
            app,
            &req,
            Some("downloading"),
            &format!("{}{who}", t("Lädt: {release}", &[("release", &title)])),
        );
        text::info(
            "Anfragen",
            &t(
                "Anfrage „{title}“: {release} an SABnzbd",
                &[("title", book["title"].as_str().unwrap_or("")), ("release", &title)],
            ),
        );
        Ok(req)
    }

    pub fn block(&self, app: &App, rid: i64, release_title: &str) -> Res<Value> {
        let mut req = self.need(app, rid)?;
        let mut blocked: Vec<Value> = req["blocked"].as_array().cloned().unwrap_or_default();
        if !release_title.is_empty() && !blocked.iter().any(|b| b.as_str() == Some(release_title)) {
            blocked.push(json!(cut(release_title, 300)));
            let skip = blocked.len().saturating_sub(50);
            blocked.drain(..skip);
            req["blocked"] = json!(blocked);
        }
        Ok(self.save(
            app,
            &req,
            None,
            &t("Release gesperrt: {release}", &[("release", release_title)]),
        ))
    }

    /// Download or import went wrong: block that release and try the next one (or give up).
    fn failed_release(&self, app: &Arc<App>, mut req: Value, why: &str) -> Value {
        if let Some(rel) = req["release"].as_str().filter(|r| !r.is_empty()) {
            let mut blocked: Vec<Value> = req["blocked"].as_array().cloned().unwrap_or_default();
            blocked.push(json!(rel));
            let skip = blocked.len().saturating_sub(50);
            blocked.drain(..skip);
            req["blocked"] = json!(blocked);
        }
        for k in ["nzo_id", "job_id", "last_search", "pack_volumes"] {
            req[k] = Value::Null;
        }
        let attempts = req["attempts"].as_i64().unwrap_or(0);
        if attempts >= MAX_ATTEMPTS {
            req["error"] = json!(why);
            let req = self.save(
                app,
                &req,
                Some("failed"),
                &t(
                    "Aufgegeben nach {n} Versuchen: {why}",
                    &[("n", &attempts.to_string()), ("why", why)],
                ),
            );
            app.notifier.send(
                "request_failed",
                &t("Buchanfrage fehlgeschlagen", &[]),
                &format!(
                    "{}\n{why}\n{}",
                    label(&req),
                    t("„Jetzt suchen“ bei der Anfrage startet die Suche neu.", &[])
                ),
                None,
            );
            self.tell(app, &req, &t("Anfrage fehlgeschlagen", &[]), &label(&req));
            return req;
        }
        let req = self.save(
            app,
            &req,
            Some("searching"),
            &t("{why} – nächstes Release", &[("why", why)]),
        );
        search_soon(app, vec![req["id"].as_i64().unwrap_or(0)]);
        req
    }

    /// The inbox found that a download for this request is another book: block it, search on.
    pub fn wrong_download(&self, app: &Arc<App>, rid: i64, release_name: &str, why: &str) {
        let Some(mut req) = self.get(app, rid) else { return };
        if !OPEN.contains(&req["status"].as_str().unwrap_or("")) {
            return;
        }
        let why = t("Falscher Inhalt geladen: {why}", &[("why", why)]);
        let current = text_of(&req["release"]);
        if !current.is_empty() && !release_name.is_empty() && release_name != current {
            // an old download turned up again: block it, but leave the current download alone
            let mut blocked: Vec<Value> = req["blocked"].as_array().cloned().unwrap_or_default();
            if !blocked.iter().any(|b| b.as_str() == Some(release_name)) {
                blocked.push(json!(cut(release_name, 300)));
                let skip = blocked.len().saturating_sub(50);
                blocked.drain(..skip);
                req["blocked"] = json!(blocked);
                self.save(
                    app,
                    &req,
                    None,
                    &t(
                        "{why} – Release gesperrt: {release}",
                        &[("why", &why), ("release", release_name)],
                    ),
                );
            }
            return;
        }
        req["release"] = json!(if current.is_empty() {
            release_name.to_string()
        } else {
            current
        });
        self.failed_release(app, req, &why);
    }

    /// A filed book was the wrong recording (marked by hand): also an available request blocks the release
    /// and searches again.
    pub fn wrong_edition(&self, app: &Arc<App>, rid: i64, release_name: &str, why: &str) {
        let Some(mut req) = self.get(app, rid) else { return };
        if !OPEN.contains(&req["status"].as_str().unwrap_or("")) && req["status"] != "available" {
            return;
        }
        if text_of(&req["release"]).is_empty() {
            req["release"] = json!(cut(release_name, 300));
        }
        req["path"] = Value::Null;
        self.failed_release(app, req, why);
    }

    // -- following the download -------------------------------------------------------------------
    /// Every few minutes: search again when due, follow downloads and imports, notice arrivals. Blocking.
    pub fn tick(&self, app: &Arc<App>) {
        let ids: Vec<i64> = {
            let c = app.db.lock();
            let Ok(mut st) = c.prepare("SELECT id FROM requests WHERE status IN (?,?,?,?)") else {
                return;
            };
            st.query_map(rusqlite::params![OPEN[0], OPEN[1], OPEN[2], OPEN[3]], |r| r.get(0))
                .into_iter()
                .flatten()
                .flatten()
                .collect()
        };
        let mut activity: Option<Vec<Value>> = None;
        for rid in ids {
            let Some(req) = self.get(app, rid) else { continue };
            if req["status"] != "pending" && req["upgrade"].is_null() && Self::owned(app, &req["book"]) {
                self.arrived(app, req, "");
                continue;
            }
            match req["status"].as_str() {
                Some("searching") => {
                    let last = req["last_search"].as_f64().unwrap_or(0.0);
                    // an indexer rested (daily limit): search again right after midnight, not hours later
                    let rested = req["resting"].as_array().is_some_and(|r| !r.is_empty())
                        && crate::text::next_local_midnight(last) + 120.0 <= now();
                    if rested || last < now() - interval_hours(app) as f64 * 3600.0 {
                        self.search(app, rid);
                    }
                }
                Some("downloading") => {
                    let items = activity.get_or_insert_with(|| {
                        if app.usenet.has_sab() {
                            app.usenet
                                .activity(15)
                                .ok()
                                .and_then(|a| a["items"].as_array().cloned())
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        }
                    });
                    self.follow_download(app, req, &items.clone());
                }
                Some("processing") => self.follow_job(app, req, None),
                _ => {}
            }
        }
    }

    /// SABnzbd ids that requests still follow – their history entry must stay until they saw it finish.
    pub fn waiting_downloads(&self, app: &App) -> Vec<String> {
        let c = app.db.lock();
        let Ok(mut st) = c.prepare("SELECT data FROM requests WHERE status='downloading'") else {
            return Vec::new();
        };
        st.query_map([], |r| r.get::<_, String>(0))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|d| serde_json::from_str::<Value>(&d).ok())
            .filter_map(|d| d["nzo_id"].as_str().filter(|n| !n.is_empty()).map(str::to_string))
            .collect()
    }

    fn follow_download(&self, app: &Arc<App>, req: Value, items: &[Value]) {
        let item = items
            .iter()
            .find(|i| i["id"].as_str().is_some_and(|x| !x.is_empty()) && i["id"] == req["nzo_id"]);
        match item {
            Some(i) if i["done"] == true => {
                if i["status"] == "Failed" {
                    let e = i["error"]
                        .as_str()
                        .filter(|e| !e.is_empty())
                        .unwrap_or("SABnzbd")
                        .to_string();
                    self.failed_release(app, req, &t("Download fehlgeschlagen: {error}", &[("error", &e)]));
                } else {
                    self.save(
                        app,
                        &req,
                        Some("processing"),
                        &t("Download fertig – wird verarbeitet", &[]),
                    );
                }
            }
            None if now() - req["sent_at"].as_f64().unwrap_or(0.0) > DOWNLOAD_TIMEOUT => {
                self.failed_release(app, req, &t("Download nach 2 Tagen nicht in SABnzbd zu finden", &[]));
            }
            _ => self.follow_job(app, req, None), // the inbox may be quicker than SABnzbd's history
        }
    }

    /// The pipeline job of this request: done → available, failed → next release, review → note it.
    pub fn follow_job(&self, app: &Arc<App>, mut req: Value, job: Option<Value>) {
        let Some(job) = job.or_else(|| app.job_for_request(req["id"].as_i64().unwrap_or(0))) else {
            return;
        };
        match job["status"].as_str() {
            Some("done") => self.arrived(app, req, job["dest"].as_str().unwrap_or("")),
            Some("failed") => {
                let e = text_of(&job["error"]);
                self.failed_release(app, req, &t("Import fehlgeschlagen: {error}", &[("error", &e)]));
            }
            Some("review") if req["review_noted"] != true => {
                req["review_noted"] = json!(true);
                self.save(
                    app,
                    &req,
                    Some("processing"),
                    &t("Wartet auf Prüfung durch einen Admin", &[]),
                );
            }
            _ if req["status"] == "downloading" => {
                self.save(
                    app,
                    &req,
                    Some("processing"),
                    &t("Download fertig – wird verarbeitet", &[]),
                );
            }
            _ => {}
        }
    }

    /// Called by the pipeline when a job is linked to a request, finishes, fails or waits for review.
    pub fn on_job(&self, app: &Arc<App>, job: &Value) {
        let Some(rid) = job["hints"]["request_id"].as_i64().filter(|r| *r != 0) else {
            return;
        };
        let Some(mut req) = self.get(app, rid) else { return };
        if !OPEN.contains(&req["status"].as_str().unwrap_or("")) {
            return;
        }
        let found = t(
            "Im Eingang gefunden: {name}",
            &[("name", job["name"].as_str().unwrap_or(""))],
        );
        if matches!(job["status"].as_str(), Some("done" | "failed" | "review")) {
            if req["status"] != "processing" {
                req = self.save(app, &req, Some("processing"), &found);
            }
            self.follow_job(app, req, Some(job.clone()));
        } else if req["status"] != "processing" {
            self.save(app, &req, Some("processing"), &found);
        }
    }

    /// Open requests for the recognition in the inbox: [{id, user, key, book}].
    pub fn open_for_matching(&self, app: &App) -> Vec<Value> {
        Self::query(
            app,
            "SELECT * FROM requests WHERE status IN ('pending','searching','downloading','processing')",
            &[],
        )
        .into_iter()
        .map(|r| json!({"id": r["id"], "user": r["user"], "key": r["book_key"], "book": r["book"]}))
        .collect()
    }

    fn arrived(&self, app: &App, mut req: Value, dest: &str) {
        req["path"] = json!(dest);
        let req = self.save(app, &req, Some("available"), &t("Verfügbar in der Bibliothek", &[]));
        text::info(
            "Anfragen",
            &t(
                "Anfrage erfüllt: „{title}“",
                &[("title", req["book"]["title"].as_str().unwrap_or(""))],
            ),
        );
        self.tell(app, &req, &t("Dein Hörbuch ist da", &[]), &label(&req));
    }

    /// The requester's own ntfy topic (set in their account), if any.
    fn tell(&self, app: &App, req: &Value, title: &str, message: &str) {
        let Some(uid) = req["user_id"].as_i64() else { return };
        let prefs: Option<String> = app
            .db
            .lock()
            .query_row("SELECT prefs FROM users WHERE id=?", [uid], |r| r.get(0))
            .ok();
        let topic = prefs
            .and_then(|p| serde_json::from_str::<Value>(&p).ok())
            .and_then(|p| p["ntfy_topic"].as_str().map(str::to_string));
        if let Some(topic) = topic.filter(|t| !t.is_empty()) {
            app.notifier.send_to(&topic, title, message);
        }
    }

    /// The wanted list of version 5.3 becomes requests of the first admin (once).
    pub fn import_wanted(&self, app: &Arc<App>) -> usize {
        let Some(items) = app
            .db
            .get("wanted")
            .and_then(|v| v.as_object().cloned())
            .filter(|o| !o.is_empty())
        else {
            return 0;
        };
        let Some(admin) = app.accounts.list(&app.db).into_iter().find(|u| u["role"] == "admin") else {
            return 0;
        };
        let n = items
            .values()
            .filter(|item| self.create(app, &admin, item).is_ok())
            .count();
        let _ = app.db.lock().execute("DELETE FROM kv WHERE key='wanted'", []);
        n
    }
}

pub fn interval_hours(app: &App) -> i64 {
    app.settings()
        .get("wanted_interval_hours")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .filter(|h| *h != 0)
        .unwrap_or(12)
        .max(1)
}

fn label(req: &Value) -> String {
    let b = &req["book"];
    let authors = b["authors"]
        .as_array()
        .map(|a| a.iter().map(text_of).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    format!("{authors} – {}", text_of(&b["title"]))
}

fn group_label(req: &Value) -> String {
    let b = &req["book"];
    let authors = b["authors"]
        .as_array()
        .map(|a| a.iter().map(text_of).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let series = req["group"]["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| text_of(&b["series"]));
    match req["group"]["kind"].as_str() {
        Some("author") => return t("Hörbücher von {person}", &[("person", &series)]),
        Some("narrator") => return t("Hörbücher gelesen von {person}", &[("person", &series)]),
        _ => {}
    }
    t(
        "{author} – Reihe „{series}“",
        &[("author", &authors), ("series", &series)],
    )
}

/// The volumes one after the other in the background – not all at once at the indexers.
pub fn search_soon(app: &Arc<App>, ids: Vec<i64>) {
    if ids.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for rid in ids {
            app.requests.search(&app, rid);
        }
    });
}

/// What the browser gets: no blocked-release internals for family accounts.
pub fn summary(req: &Value, user: &Value, sees_all: bool) -> Value {
    let keys = [
        "id",
        "book",
        "status",
        "created",
        "updated",
        "reason",
        "release",
        "error",
        "last_search",
        "found",
        "seen",
        "history",
        "path",
        "attempts",
        "group",
        "resting",
        "upgrade",
    ];
    let mut out: BTreeMap<String, Value> = keys
        .iter()
        .map(|k| (k.to_string(), req.get(*k).cloned().unwrap_or(Value::Null)))
        .collect();
    out.insert("mine".into(), json!(req["user_id"] == user["id"]));
    if sees_all {
        out.insert("user".into(), req["user"].clone());
    }
    if is_admin(user) {
        out.insert(
            "blocked".into(),
            req.get("blocked")
                .filter(|b| !b.is_null())
                .cloned()
                .unwrap_or(json!([])),
        );
    }
    json!(out)
}
