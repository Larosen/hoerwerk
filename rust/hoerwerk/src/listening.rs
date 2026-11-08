//! What the family listens to: Hörwerk accounts linked to Audiobookshelf accounts and Plex profiles.
//! Links live in the account's prefs (`abs_user` / `plex_account` = {id, name}); one target belongs to at most one
//! account. Audiobookshelf knows progress and time per day; Plex only keeps plays that reached the end.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde_json::{Map, Value, json};

use crate::app::App;
use crate::text::{self, now, t};

const CACHE_SECONDS: f64 = 15.0 * 60.0;

fn key_of(kind: &str) -> Option<&'static str> {
    match kind {
        "abs" => Some("abs_user"),
        "plex" => Some("plex_account"),
        _ => None,
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        other => other.to_string(),
    }
}

pub fn prefs(app: &App, uid: i64) -> Map<String, Value> {
    let p: Option<String> = app
        .db
        .lock()
        .query_row("SELECT prefs FROM users WHERE id=?", [uid], |r| r.get(0))
        .ok();
    p.and_then(|p| serde_json::from_str::<Value>(&p).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn set_prefs(app: &App, uid: i64, changes: Map<String, Value>) {
    let mut p = prefs(app, uid);
    p.extend(changes);
    let _ = app.db.lock().execute(
        "UPDATE users SET prefs=? WHERE id=?",
        rusqlite::params![crate::db::compact(&Value::Object(p)), uid],
    );
}

fn date(ts: f64) -> String {
    let tm = text::local(ts);
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

type Cache = HashMap<(String, i64, String), (f64, Value)>;

#[derive(Default)]
pub struct Listening {
    cache: Mutex<Cache>,
}

impl Listening {
    pub fn available(app: &App) -> Value {
        let (a, p) = (app.abs.public_config(), app.plex.public_config());
        json!({"abs": !s(&a, "url").is_empty() && a["token_set"] == true, "plex": !s(&p, "url").is_empty() && p["token_set"] == true})
    }

    pub fn links(app: &App, uid: i64) -> Value {
        let p = prefs(app, uid);
        let get = |k: &str| {
            p.get(k)
                .filter(|v| !v.is_null() && v.as_object().is_none_or(|o| !o.is_empty()))
                .cloned()
                .unwrap_or(Value::Null)
        };
        json!({"abs": get("abs_user"), "plex": get("plex_account")})
    }

    /// Link (or with target None: unlink); the same target is taken away from any other account.
    pub fn set_link(&self, app: &App, uid: i64, kind: &str, target: Option<&Value>) -> Result<Value, String> {
        let key = key_of(kind).ok_or_else(|| t("Ungültige Daten", &[]))?;
        if app.accounts.get(&app.db, uid).is_none() {
            return Err(t("Ungültige Daten", &[]));
        }
        let target = target.map(|tg| {
            let cut = |v: &Value| text_of(v).chars().take(100).collect::<String>();
            json!({"id": cut(&tg["id"]), "name": cut(&tg["name"])})
        });
        if let Some(tg) = &target {
            for u in app.accounts.list(&app.db) {
                let other_id = u["id"].as_i64().unwrap_or(0);
                if other_id != uid
                    && prefs(app, other_id)
                        .get(key)
                        .is_some_and(|o| text_of(&o["id"]) == s(tg, "id"))
                {
                    set_prefs(app, other_id, Map::from_iter([(key.to_string(), Value::Null)]));
                }
            }
        }
        set_prefs(
            app,
            uid,
            Map::from_iter([(key.to_string(), target.unwrap_or(Value::Null))]),
        );
        self.cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|k, _| k.1 != uid);
        Ok(Self::links(app, uid))
    }

    /// Sign in to Audiobookshelf once: proves the account is yours, then it is linked. Blocking.
    pub fn link_own_abs(&self, app: &App, uid: i64, username: &str, password: &str) -> Result<Value, (bool, String)> {
        if username.is_empty() || password.is_empty() {
            return Err((false, t("Benutzername und Passwort fehlen", &[])));
        }
        let user: String = username.chars().take(200).collect();
        let pw: String = password.chars().take(500).collect();
        let found = app.abs.check_login(&user, &pw).map_err(|e| (true, e.message))?;
        self.set_link(app, uid, "abs", Some(&found)).map_err(|e| (false, e))
    }

    fn abs_history(app: &App, abs_id: &str) -> Result<Value, String> {
        let items = app.abs.library_items().map_err(|e| e.message)?;
        let stats = app.abs.listening_stats(abs_id).map_err(|e| e.message)?;
        let mut books = Vec::new();
        for p in app.abs.progress(abs_id).map_err(|e| e.message)? {
            let item = items.get(&text_of(&p["libraryItemId"])).cloned().unwrap_or(json!({}));
            if item.as_object().is_none_or(|o| o.is_empty()) && s(&p, "displayTitle").is_empty() {
                continue;
            }
            let ms = |v: &Value| v.as_f64().unwrap_or(0.0) / 1000.0;
            let title = if s(&item, "title").is_empty() {
                s(&p, "displayTitle")
            } else {
                s(&item, "title")
            };
            books.push(json!({"title": title, "author": s(&item, "author"), "asin": item["asin"],
                              "progress": p["progress"].as_f64().unwrap_or(0.0), "finished": p["isFinished"].as_bool().unwrap_or(false),
                              "finished_at": if ms(&p["finishedAt"]) == 0.0 { Value::Null } else { json!(ms(&p["finishedAt"])) },
                              "last": if ms(&p["lastUpdate"]) == 0.0 { Value::Null } else { json!(ms(&p["lastUpdate"])) }, "source": "abs"}));
        }
        let days: Map<String, Value> = stats["days"]
            .as_object()
            .map(|d| {
                d.iter()
                    .map(|(k, v)| (k.clone(), json!(v.as_f64().unwrap_or(0.0))))
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({"books": books, "days": days}))
    }

    fn plex_history(app: &App, plex_id: &str) -> Result<Value, String> {
        let plays = app.plex.history(plex_id, 0.0, 2000).map_err(|e| e.message)?;
        let keys: Vec<String> = plays.iter().map(|p| s(p, "key").to_string()).collect();
        let lengths = app.plex.durations(&keys, 150);
        let mut days: BTreeMap<String, f64> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut books: HashMap<String, Value> = HashMap::new();
        for p in &plays {
            let viewed = p["viewed"].as_i64().unwrap_or(0);
            *days.entry(date(viewed as f64)).or_default() += lengths.get(s(p, "key")).copied().unwrap_or(0.0);
            let bk = s(p, "book_key").to_string();
            let b = books.entry(bk.clone()).or_insert_with(|| {
                order.push(bk.clone());
                json!({"title": p["book"], "author": p["author"], "asin": null, "progress": 1.0, "finished": true,
                       "finished_at": viewed, "last": viewed, "source": "plex"})
            });
            b["last"] = json!(b["last"].as_i64().unwrap_or(0).max(viewed));
        }
        let list: Vec<Value> = order.into_iter().filter_map(|k| books.remove(&k)).collect();
        Ok(json!({"books": list, "days": days}))
    }

    /// Everything this account heard: {books, days: {date: seconds}, errors, linked}. Blocking.
    pub fn history(&self, app: &App, uid: i64) -> Value {
        let links = Self::links(app, uid);
        let (mut books, mut days, mut errors, mut linked) = (
            Vec::<Value>::new(),
            BTreeMap::<String, f64>::new(),
            Vec::<String>::new(),
            Vec::<String>::new(),
        );
        for kind in ["abs", "plex"] {
            let link = &links[kind];
            if link.is_null() {
                continue;
            }
            linked.push(kind.into());
            let id = text_of(&link["id"]);
            let key = (kind.to_string(), uid, id.clone());
            let hit = self
                .cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&key)
                .filter(|(at, _)| *at > now() - CACHE_SECONDS)
                .map(|x| x.1.clone());
            let part = match hit {
                Some(v) => v,
                None => {
                    let fetched = if kind == "abs" {
                        Self::abs_history(app, &id)
                    } else {
                        Self::plex_history(app, &id)
                    };
                    match fetched {
                        Ok(v) => {
                            let mut c = self.cache.lock().unwrap_or_else(|p| p.into_inner());
                            if c.len() > 200 {
                                c.clear();
                            }
                            c.insert(key, (now(), v.clone()));
                            v
                        }
                        Err(e) => {
                            crate::text::warn(
                                "Hörverlauf",
                                &t("Hörverlauf ({source}): {error}", &[("source", kind), ("error", &e)]),
                            );
                            errors.push(e);
                            continue;
                        }
                    }
                }
            };
            books.extend(part["books"].as_array().cloned().unwrap_or_default());
            for (d, secs) in part["days"].as_object().cloned().unwrap_or_default() {
                *days.entry(d).or_default() += secs.as_f64().unwrap_or(0.0);
            }
        }
        let last = |b: &Value| b["last"].as_f64().unwrap_or(0.0);
        books.sort_by(|a, b| last(b).partial_cmp(&last(a)).unwrap_or(std::cmp::Ordering::Equal));
        json!({"books": books, "days": days, "errors": errors, "linked": linked})
    }

    /// Numbers for the statistics: seconds heard, books heard / finished, last time – since a moment.
    pub fn summary(&self, app: &App, uid: i64, since: f64) -> Value {
        let h = self.history(app, uid);
        let day0 = if since > 0.0 { date(since) } else { String::new() };
        let seconds: f64 = h["days"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(d, _)| **d >= day0)
            .map(|(_, v)| v.as_f64().unwrap_or(0.0))
            .sum();
        let books = h["books"].as_array().cloned().unwrap_or_default();
        let num = |v: &Value| v.as_f64().unwrap_or(0.0);
        let heard = books
            .iter()
            .filter(|b| num(&b["last"]) >= since && (num(&b["progress"]) >= 0.02 || b["finished"] == true))
            .count();
        let finished = books
            .iter()
            .filter(|b| {
                b["finished"] == true && {
                    let f = num(&b["finished_at"]);
                    (if f == 0.0 { num(&b["last"]) } else { f }) >= since
                }
            })
            .count();
        let last = books.iter().map(|b| num(&b["last"])).fold(0.0, f64::max);
        json!({"linked": h["linked"], "seconds": crate::releases::py_round(seconds), "books": heard, "finished": finished,
               "last": if last == 0.0 { Value::Null } else { json!(last) }, "errors": h["errors"]})
    }

    /// The books this account heard most recently (ASIN from Audiobookshelf or the library) – for "Für dich".
    pub fn seeds(&self, app: &App, uid: i64, limit: usize) -> Vec<Value> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for b in self.history(app, uid)["books"].as_array().cloned().unwrap_or_default() {
            if b["progress"].as_f64().unwrap_or(0.0) < 0.15 && b["finished"] != true {
                continue; // only looked in: says little
            }
            let mut asin = s(&b, "asin").to_string();
            if asin.is_empty() && !s(&b, "title").is_empty() {
                let hit = app
                    .library
                    .find(&json!({"title": b["title"], "authors": [s(&b, "author")]}));
                asin = hit.first().map(|h| s(h, "asin").to_string()).unwrap_or_default();
            }
            if !asin.is_empty() && !seen.contains(&asin) {
                seen.push(asin.clone());
                out.push(json!({"asin": asin, "title": b["title"]}));
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }
}
