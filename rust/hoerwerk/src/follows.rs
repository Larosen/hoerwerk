//! Following authors and narrators: once a day Hörwerk looks at Audible for their newest books.
//! A book that came out after the follow started is requested by itself (as the person who follows, so the
//! monthly limit and the approval apply as for any request) – or only marked as new, if they chose so.
//! Older books are picked once in the person window and requested together.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::app::App;
use crate::db::compact;
use crate::text::{self, now, t};

pub const KINDS: [&str; 2] = ["author", "narrator"];
/// Follows per account – a household follows a handful of people, not the whole store.
const MAX_PER_USER: i64 = 200;
/// Works shown per person (newest first).
pub const WORKS_MAX: usize = 100;
/// How often one follow is looked at.
const EVERY: f64 = 24.0 * 3600.0;
/// Remembered ASINs per follow (old ones fall out; their release date keeps them out anyway).
const SEEN_MAX: usize = 400;

/// "de"/"en" from the page → Audible's language name ("" = any).
pub fn language_name(code: &str) -> &'static str {
    match code {
        "de" => "german",
        "en" => "english",
        _ => "",
    }
}

fn day(ts: f64) -> String {
    let tm = text::local(ts);
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let data: Value = serde_json::from_str(&r.get::<_, String>("data")?).unwrap_or(json!({}));
    Ok(json!({
        "id": r.get::<_, i64>("id")?, "user_id": r.get::<_, i64>("user_id")?, "kind": r.get::<_, String>("kind")?,
        "name": r.get::<_, String>("name")?, "language": r.get::<_, String>("language")?,
        "region": r.get::<_, String>("region")?, "auto": r.get::<_, i64>("auto")? != 0,
        "since": r.get::<_, f64>("since")?, "checked": r.get::<_, Option<f64>>("checked")?, "data": data,
    }))
}

/// What the page sees of a follow (not the remembered ASINs).
pub fn public(f: &Value) -> Value {
    let d = &f["data"];
    json!({
        "id": f["id"], "kind": f["kind"], "name": f["name"], "language": f["language"], "auto": f["auto"],
        "since": f["since"], "checked": f["checked"], "added": d["added"].as_i64().unwrap_or(0),
        "recent": d["recent"].as_array().cloned().unwrap_or_default(), "fresh": d["fresh"].as_array().cloned().unwrap_or_default(),
        "note": d["note"].as_str().unwrap_or(""),
    })
}

pub fn list(app: &App, uid: i64) -> Vec<Value> {
    let c = app.db.lock();
    let Ok(mut st) = c.prepare("SELECT * FROM follows WHERE user_id=? ORDER BY name COLLATE NOCASE") else {
        return Vec::new();
    };
    st.query_map([uid], row)
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn get(app: &App, fid: i64) -> Option<Value> {
    app.db
        .lock()
        .query_row("SELECT * FROM follows WHERE id=?", [fid], row)
        .optional()
        .ok()
        .flatten()
}

/// The follow of this account for this person, if there is one.
pub fn find(app: &App, uid: i64, kind: &str, name: &str) -> Option<Value> {
    app.db
        .lock()
        .query_row(
            "SELECT * FROM follows WHERE user_id=? AND kind=? AND name=? COLLATE NOCASE",
            rusqlite::params![uid, kind, name],
            row,
        )
        .optional()
        .ok()
        .flatten()
}

/// Clean name, known kind and language – or the message for the page.
pub fn validate(kind: &str, name: &str, language: &str) -> Result<(String, String, String), String> {
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect();
    if !KINDS.contains(&kind) || name.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return Err(t("Ungültige Daten", &[]));
    }
    let language = match language {
        "de" | "en" => language.to_string(),
        _ => String::new(),
    };
    Ok((kind.to_string(), name, language))
}

pub fn add(app: &App, uid: i64, kind: &str, name: &str, language: &str, auto: bool) -> Result<Value, String> {
    let (kind, name, language) = validate(kind, name, language)?;
    if let Some(f) = find(app, uid, &kind, &name) {
        return Ok(f);
    }
    let count: i64 = app
        .db
        .lock()
        .query_row("SELECT COUNT(*) FROM follows WHERE user_id=?", [uid], |r| r.get(0))
        .unwrap_or(0);
    if count >= MAX_PER_USER {
        return Err(t("Höchstens {n} Personen", &[("n", &MAX_PER_USER.to_string())]));
    }
    let region = app
        .settings()
        .get("ab_region")
        .and_then(Value::as_str)
        .unwrap_or("de")
        .to_string();
    let id = {
        let c = app.db.lock();
        c.execute(
            "INSERT INTO follows(user_id, kind, name, language, region, auto, since) VALUES(?,?,?,?,?,?,?)",
            rusqlite::params![uid, kind, name, language, region, auto as i64, now()],
        )
        .map_err(|e| e.to_string())?;
        c.last_insert_rowid()
    };
    text::info("Folgen", &t("Neu gefolgt: {name}", &[("name", &name)]));
    get(app, id).ok_or_else(|| t("Nicht gefunden", &[]))
}

pub fn set_auto(app: &App, uid: i64, fid: i64, auto: bool) -> bool {
    app.db
        .lock()
        .execute(
            "UPDATE follows SET auto=? WHERE id=? AND user_id=?",
            rusqlite::params![auto as i64, fid, uid],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

pub fn remove(app: &App, uid: i64, fid: i64) -> bool {
    app.db
        .lock()
        .execute(
            "DELETE FROM follows WHERE id=? AND user_id=?",
            rusqlite::params![fid, uid],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// The person's books at Audible, newest first.
pub fn works(app: &App, kind: &str, name: &str, language: &str, region: &str) -> Result<Vec<Value>, String> {
    app.providers
        .client
        .by_person(region, kind, name, language_name(language), WORKS_MAX)
        .map_err(|e| e.0)
}

/// Books that are new for a follow: out on or after the day it started and not looked at before.
pub fn new_works(works: &[Value], seen: &HashSet<String>, since_day: &str) -> Vec<Value> {
    works
        .iter()
        .filter(|b| {
            let asin = b["asin"].as_str().unwrap_or("");
            let released = b["release_date"].as_str().unwrap_or("");
            !asin.is_empty() && !seen.contains(asin) && !released.is_empty() && released >= since_day
        })
        .cloned()
        .collect()
}

fn titles(books: &[Value]) -> Vec<Value> {
    books.iter().map(|b| b["title"].clone()).collect()
}

/// Look at one follow now: request (or mark) what is new. Returns how many books were requested.
pub fn check(app: &Arc<App>, f: &Value) -> Result<usize, String> {
    let (fid, uid) = (f["id"].as_i64().unwrap_or(0), f["user_id"].as_i64().unwrap_or(0));
    let (kind, name) = (f["kind"].as_str().unwrap_or(""), f["name"].as_str().unwrap_or(""));
    let found = works(
        app,
        kind,
        name,
        f["language"].as_str().unwrap_or(""),
        f["region"].as_str().unwrap_or("de"),
    )?;
    let mut data = f["data"].clone();
    if !data.is_object() {
        data = json!({});
    }
    let mut seen: Vec<String> = data["seen"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    let fresh = new_works(
        &found,
        &seen.iter().cloned().collect(),
        &day(f["since"].as_f64().unwrap_or(0.0)),
    );
    let mut requested = 0;
    let mut done: Vec<&Value> = Vec::new();
    data["note"] = json!("");
    if !fresh.is_empty() {
        if f["auto"] == true {
            let user = app.accounts.get(&app.db, uid).filter(|u| u["disabled"] != true);
            match user {
                None => {}
                Some(user) => {
                    let group = json!({"kind": kind, "title": name});
                    match app.requests.create_series(app, &user, &group, &json!(fresh)) {
                        Ok(res) => {
                            requested = res["items"].as_array().map_or(0, Vec::len);
                            done.extend(fresh.iter());
                            let mut recent: Vec<Value> = res["items"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|r| r["book"]["title"].clone())
                                .collect();
                            recent.extend(data["recent"].as_array().cloned().unwrap_or_default());
                            recent.truncate(5);
                            data["recent"] = json!(recent);
                            data["added"] = json!(data["added"].as_i64().unwrap_or(0) + requested as i64);
                        }
                        // everything already there or asked for: nothing to do, also next time
                        Err(e) if e.status == 409 => done.extend(fresh.iter()),
                        // e.g. the monthly limit: keep them for the next look
                        Err(e) => data["note"] = json!(e.message),
                    }
                }
            }
        } else {
            let mut marked = titles(&fresh);
            marked.extend(data["fresh"].as_array().cloned().unwrap_or_default());
            marked.truncate(10);
            data["fresh"] = json!(marked);
            done.extend(fresh.iter());
        }
    }
    for b in done {
        if let Some(a) = b["asin"].as_str() {
            seen.push(a.to_string());
        }
    }
    if seen.len() > SEEN_MAX {
        seen.drain(..seen.len() - SEEN_MAX);
    }
    data["seen"] = json!(seen);
    let _ = app.db.lock().execute(
        "UPDATE follows SET checked=?, data=? WHERE id=?",
        rusqlite::params![now(), compact(&data), fid],
    );
    if requested > 0 {
        text::info(
            "Folgen",
            &t(
                "{name}: {n} neue Hörbücher angefragt",
                &[("name", name), ("n", &requested.to_string())],
            ),
        );
    }
    Ok(requested)
}

/// Clear the "new" marks of a follow (the person looked at them).
pub fn seen_fresh(app: &App, uid: i64, fid: i64) {
    let Some(f) = get(app, fid).filter(|f| f["user_id"].as_i64() == Some(uid)) else {
        return;
    };
    let mut data = f["data"].clone();
    data["fresh"] = json!([]);
    let _ = app.db.lock().execute(
        "UPDATE follows SET data=? WHERE id=?",
        rusqlite::params![compact(&data), fid],
    );
}

/// The background round: every follow that is due, one after the other.
pub fn tick(app: &Arc<App>) {
    let due: Vec<Value> = {
        let c = app.db.lock();
        let Ok(mut st) = c.prepare("SELECT * FROM follows WHERE checked IS NULL OR checked < ? ORDER BY checked")
        else {
            return;
        };
        st.query_map([now() - EVERY], row)
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    for f in due {
        if let Err(e) = check(app, &f) {
            text::warn(
                "Folgen",
                &t(
                    "{name}: Audible nicht erreichbar ({error})",
                    &[("name", f["name"].as_str().unwrap_or("")), ("error", &e)],
                ),
            );
            return; // try again in the next round
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_books_after_the_follow_count_as_new() {
        let works = vec![
            json!({"asin": "B0NEW00001", "title": "Neu", "release_date": "2026-11-20"}),
            json!({"asin": "B0SAME0001", "title": "Am selben Tag", "release_date": "2026-11-13"}),
            json!({"asin": "B0OLD00001", "title": "Alt", "release_date": "2020-01-01"}),
            json!({"asin": "B0SEEN0001", "title": "Schon gesehen", "release_date": "2026-11-21"}),
            json!({"asin": "B0NODATE01", "title": "Ohne Datum"}),
        ];
        let seen: HashSet<String> = ["B0SEEN0001".to_string()].into_iter().collect();
        let fresh = new_works(&works, &seen, "2026-11-13");
        let got: Vec<&str> = fresh.iter().map(|b| b["title"].as_str().unwrap()).collect();
        assert_eq!(got, ["Neu", "Am selben Tag"]);
    }

    #[test]
    fn names_and_kinds_are_checked() {
        assert!(validate("author", "  Klara   Klang ", "de").is_ok_and(|v| v.1 == "Klara Klang" && v.2 == "de"));
        assert!(validate("narrator", "Rudi Ruhig", "xx").is_ok_and(|v| v.2.is_empty()));
        assert!(validate("publisher", "Verlag", "").is_err());
        assert!(validate("author", " . ", "").is_err());
    }
}
