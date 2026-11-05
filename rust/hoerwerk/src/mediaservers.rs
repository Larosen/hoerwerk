//! Audiobookshelf and Plex (abs.py, plex.py): rescan after books were filed (one scan after the last book of a
//! row), connection test, and what each account heard. Tokens stay on the server, never follow a changed address
//! and never follow a redirect.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::db::Db;
use crate::http;
use crate::rx;
use crate::text::{self, now, t};

#[derive(Debug)]
pub struct MediaError {
    pub message: String,
    pub timeout: bool,
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

type Res<T> = Result<T, MediaError>;

fn fail<T>(m: String) -> Res<T> {
    Err(MediaError {
        message: m,
        timeout: false,
    })
}

fn base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if !url.is_empty() && !rx::is_match(rx!(r"(?i)^https?://"), url) {
        format!("http://{url}")
    } else {
        url.to_string()
    }
}

fn origin(url: &str) -> String {
    let u = if rx::is_match(rx!(r"(?i)^https?://"), url) {
        url.to_string()
    } else {
        format!("http://{url}")
    };
    http::origin(&u)
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false))) && v != Some(&json!("")) && v != Some(&json!(0))
}

fn quote(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b'/');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

/// One scan after the last of several requests in a row (a debounced timer).
struct Debounce {
    generation: Arc<AtomicU64>,
}

impl Debounce {
    fn new() -> Self {
        Debounce {
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    fn later(&self, delay: Duration, f: impl FnOnce() + Send + 'static) {
        let mine = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if generation.load(Ordering::SeqCst) == mine {
                f();
            }
        });
    }
}

// -- Audiobookshelf ------------------------------------------------------------------------------------

fn abs_call(method: &str, url: &str, token: &str, timeout: u64) -> Res<Value> {
    let r = http::Call::new(method, url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/json")
        .timeout(timeout)
        .no_redirect()
        .limit(2 << 20)
        .send()
        .map_err(|e| {
            if e == "timed out" {
                MediaError {
                    message: t("Audiobookshelf antwortet nicht rechtzeitig", &[]),
                    timeout: true,
                }
            } else {
                MediaError {
                    message: t("Audiobookshelf nicht erreichbar: {reason}", &[("reason", &e)]),
                    timeout: false,
                }
            }
        })?;
    match r.status {
        401 | 403 => fail(t("Audiobookshelf: API-Token ungültig oder ohne Admin-Rechte", &[])),
        s if !(200..300).contains(&s) => fail(format!("Audiobookshelf: HTTP {s}")),
        _ => Ok(if r.body.iter().all(u8::is_ascii_whitespace) {
            json!({})
        } else {
            r.json().unwrap_or(json!({}))
        }),
    }
}

pub struct Abs {
    db: Db,
    debounce: Debounce,
    pub last: Arc<Mutex<Option<String>>>,
    items: Mutex<Option<(f64, String, HashMap<String, Value>)>>,
}

impl Abs {
    pub fn new(db: Db) -> Self {
        Abs {
            db,
            debounce: Debounce::new(),
            last: Arc::new(Mutex::new(None)),
            items: Mutex::new(None),
        }
    }

    fn load(&self) -> Map<String, Value> {
        self.db
            .get("abs")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    pub fn configured(&self) -> bool {
        let c = self.load();
        truthy(c.get("url")) && truthy(c.get("token")) && truthy(c.get("library"))
    }

    pub fn public_config(&self) -> Value {
        let c = self.load();
        let g = |k: &str| c.get(k).map(text_of).unwrap_or_default();
        json!({"url": g("url"), "library": g("library"), "library_name": g("library_name"), "token_set": truthy(c.get("token")),
               "configured": self.configured(), "last": *self.last.lock().unwrap_or_else(|p| p.into_inner())})
    }

    pub fn update_config(&self, new: &Value) -> Value {
        let old = self.load();
        let url = text_of(&new["url"]).trim().to_string();
        let keep = if origin(&old.get("url").map(text_of).unwrap_or_default()) == origin(&url) {
            old.get("token").map(text_of).unwrap_or_default()
        } else {
            String::new()
        };
        let token = text_of(&new["token"]).trim().to_string();
        let name = if truthy(new.get("library_name")) {
            text_of(&new["library_name"])
        } else {
            old.get("library_name").map(text_of).unwrap_or_default()
        };
        let _ = self.db.put(
            "abs",
            &json!({"url": url, "library": text_of(&new["library"]).trim(), "library_name": name,
                                           "token": if token.is_empty() { keep } else { token }}),
        );
        self.public_config()
    }

    fn api(&self, path: &str) -> Res<Value> {
        let c = self.load();
        if !truthy(c.get("url")) || !truthy(c.get("token")) {
            return fail(t("Audiobookshelf ist noch nicht eingerichtet", &[]));
        }
        abs_call(
            "GET",
            &format!("{}{path}", base(&text_of(&c["url"]))),
            &text_of(&c["token"]),
            15,
        )
    }

    pub fn libraries(&self) -> Res<Vec<Value>> {
        let data = self.api("/api/libraries")?;
        let libs = if data.is_object() {
            data["libraries"].clone()
        } else {
            data
        };
        Ok(libs
            .as_array()
            .into_iter()
            .flatten()
            .filter(|l| l.is_object() && truthy(l.get("id")))
            .map(|l| json!({"id": text_of(&l["id"]), "name": text_of(&l["name"]), "type": l["mediaType"]}))
            .collect())
    }

    /// Check the connection; pick the only book library when none is chosen yet.
    pub fn test(&self) -> Res<String> {
        let libs: Vec<Value> = self
            .libraries()?
            .into_iter()
            .filter(|l| l["type"].is_null() || l["type"] == "book")
            .collect();
        let mut c = self.load();
        if !truthy(c.get("library")) && libs.len() == 1 {
            c.insert("library".into(), libs[0]["id"].clone());
            c.insert("library_name".into(), libs[0]["name"].clone());
            let _ = self.db.put("abs", &Value::Object(c));
            return Ok(t(
                "verbunden, Bibliothek „{name}“ gewählt",
                &[("name", libs[0]["name"].as_str().unwrap_or(""))],
            ));
        }
        if let Some(chosen) = libs.iter().find(|l| Some(&l["id"]) == c.get("library")) {
            c.insert("library_name".into(), chosen["name"].clone());
            let _ = self.db.put("abs", &Value::Object(c));
            return Ok(t(
                "verbunden, Bibliothek „{name}“",
                &[("name", chosen["name"].as_str().unwrap_or(""))],
            ));
        }
        let names: Vec<String> = libs
            .iter()
            .map(|l| format!("{} ({})", text_of(&l["name"]), text_of(&l["id"])))
            .collect();
        Ok(t(
            "verbunden – bitte Bibliothek wählen: {names}",
            &[(
                "names",
                &if names.is_empty() {
                    "keine".to_string()
                } else {
                    names.join(", ")
                },
            )],
        ))
    }

    /// Audiobookshelf answers when the scan is finished – a big library takes minutes. Blocking.
    pub fn scan_now(&self) -> Option<String> {
        if !self.configured() {
            return None;
        }
        let c = self.load();
        let url = format!(
            "{}/api/libraries/{}/scan",
            base(&text_of(&c["url"])),
            quote(&text_of(&c["library"]))
        );
        let (last, error) = match abs_call("POST", &url, &text_of(&c["token"]), 10) {
            Ok(_) => (t("Scan abgeschlossen", &[]), None),
            Err(e) if e.timeout => (t("Scan läuft", &[]), None),
            Err(e) => {
                text::warn("Audiobookshelf", &e.message);
                (e.message.clone(), Some(e.message))
            }
        };
        *self.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(last);
        error
    }

    pub fn users(&self) -> Res<Vec<Value>> {
        let data = self.api("/api/users")?;
        let users = if data.is_object() { data["users"].clone() } else { data };
        Ok(users
            .as_array()
            .into_iter()
            .flatten()
            .filter(|u| u.is_object() && truthy(u.get("id")))
            .map(|u| json!({"id": text_of(&u["id"]), "name": text_of(&u["username"]), "active": u.get("isActive").cloned().unwrap_or(json!(true))}))
            .collect())
    }

    /// The user's progress per book (no podcast episodes).
    pub fn progress(&self, user_id: &str) -> Res<Vec<Value>> {
        let data = self.api(&format!("/api/users/{}", quote(user_id)))?;
        Ok(data["mediaProgress"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p.is_object() && !truthy(p.get("episodeId")))
            .cloned()
            .collect())
    }

    pub fn listening_stats(&self, user_id: &str) -> Res<Value> {
        let data = self.api(&format!("/api/users/{}/listening-stats", quote(user_id)))?;
        Ok(if data.is_object() { data } else { json!({}) })
    }

    /// id -> {title, author, asin, duration} of the chosen library (cached for 10 minutes).
    pub fn library_items(&self) -> Res<HashMap<String, Value>> {
        let c = self.load();
        let lib = c.get("library").map(text_of).unwrap_or_default();
        if let Some((at, l, items)) = self.items.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
            && *at > now() - 600.0
            && *l == lib
        {
            return Ok(items.clone());
        }
        let mut out = HashMap::new();
        if !lib.is_empty() {
            for page in 0..200 {
                let data = self.api(&format!(
                    "/api/libraries/{}/items?minified=1&limit=500&page={page}",
                    quote(&lib)
                ))?;
                let results = data["results"].as_array().cloned().unwrap_or_default();
                for item in &results {
                    let media = &item["media"];
                    let meta = &media["metadata"];
                    let asin = text_of(&meta["asin"]).to_uppercase();
                    out.insert(text_of(&item["id"]), json!({"title": text_of(&meta["title"]), "author": text_of(&meta["authorName"]),
                                                            "asin": if asin.is_empty() { Value::Null } else { json!(asin) },
                                                            "duration": media["duration"].as_f64().unwrap_or(0.0)}));
                }
                if results.len() < 500 {
                    break;
                }
            }
        }
        *self.items.lock().unwrap_or_else(|p| p.into_inner()) = Some((now(), lib, out.clone()));
        Ok(out)
    }

    /// {id, name} of the Audiobookshelf account these credentials belong to (nothing is stored). Blocking.
    pub fn check_login(&self, username: &str, password: &str) -> Res<Value> {
        let c = self.load();
        if !truthy(c.get("url")) {
            return fail(t("Audiobookshelf ist noch nicht eingerichtet", &[]));
        }
        let r = http::post(&format!("{}/login", base(&text_of(&c["url"]))))
            .json(&json!({"username": username, "password": password}))
            .header("Accept", "application/json")
            .timeout(15)
            .no_redirect()
            .limit(1 << 20)
            .send()
            .map_err(|e| MediaError {
                message: t("Audiobookshelf nicht erreichbar: {reason}", &[("reason", &e)]),
                timeout: false,
            })?;
        match r.status {
            401 | 403 => return fail(t("Audiobookshelf: Benutzername oder Passwort falsch", &[])),
            429 => return fail(t("Audiobookshelf: zu viele Versuche – bitte später noch einmal", &[])),
            s if !(200..300).contains(&s) => return fail(format!("Audiobookshelf: HTTP {s}")),
            _ => {}
        }
        let user = r.json().map(|d| d["user"].clone()).unwrap_or(Value::Null);
        if !truthy(user.get("id")) {
            return fail(t("Audiobookshelf: unerwartete Antwort", &[]));
        }
        let name = if truthy(user.get("username")) {
            text_of(&user["username"])
        } else {
            username.to_string()
        };
        Ok(json!({"id": text_of(&user["id"]), "name": name}))
    }
}

// -- Plex ---------------------------------------------------------------------------------------------

fn plex_call(method: &str, url: &str, token: &str) -> Res<Value> {
    let r = http::Call::new(method, url)
        .header("X-Plex-Token", token)
        .header("Accept", "application/json")
        .header("X-Plex-Product", "Hoerwerk")
        .header("X-Plex-Client-Identifier", "hoerwerk")
        .timeout(15)
        .no_redirect()
        .limit(4 << 20)
        .send()
        .map_err(|e| MediaError {
            message: t("Plex nicht erreichbar: {reason}", &[("reason", &e)]),
            timeout: false,
        })?;
    match r.status {
        401 | 403 => fail(t("Plex: Token ungültig", &[])),
        s if !(200..300).contains(&s) => fail(format!("Plex: HTTP {s}")),
        _ => Ok(if r.body.iter().all(u8::is_ascii_whitespace) {
            json!({})
        } else {
            r.json().unwrap_or(json!({}))
        }),
    }
}

pub struct Plex {
    db: Db,
    debounce: Debounce,
    pub last: Arc<Mutex<Option<String>>>,
}

impl Plex {
    pub fn new(db: Db) -> Self {
        Plex {
            db,
            debounce: Debounce::new(),
            last: Arc::new(Mutex::new(None)),
        }
    }

    fn load(&self) -> Map<String, Value> {
        self.db
            .get("plex")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    pub fn configured(&self) -> bool {
        let c = self.load();
        truthy(c.get("url")) && truthy(c.get("token")) && truthy(c.get("section"))
    }

    pub fn public_config(&self) -> Value {
        let c = self.load();
        let g = |k: &str| c.get(k).map(text_of).unwrap_or_default();
        json!({"url": g("url"), "section": g("section"), "section_name": g("section_name"), "empty_trash": truthy(c.get("empty_trash")),
               "token_set": truthy(c.get("token")), "configured": self.configured(),
               "last": *self.last.lock().unwrap_or_else(|p| p.into_inner())})
    }

    pub fn update_config(&self, new: &Value) -> Res<Value> {
        let old = self.load();
        let mut url = base(&text_of(&new["url"]));
        if !url.is_empty() {
            if !http::valid_url(&url) {
                return fail(t("Ungültige Plex-Adresse", &[]));
            }
            // "http://tower:32400/web/index.html" -> the server
            let (scheme, rest) = url.split_once("://").unwrap_or(("http", ""));
            url = format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or(""));
        }
        let keep = if origin(&old.get("url").map(text_of).unwrap_or_default()) == origin(&url) {
            old.get("token").map(text_of).unwrap_or_default()
        } else {
            String::new()
        };
        let section = text_of(&new["section"]).trim().to_string();
        let name = if Some(&json!(section)) == old.get("section") {
            old.get("section_name").map(text_of).unwrap_or_default()
        } else {
            String::new()
        };
        let token = text_of(&new["token"]).trim().to_string();
        let _ = self.db.put(
            "plex",
            &json!({"url": url, "section": section, "section_name": name, "empty_trash": truthy(new.get("empty_trash")),
                                            "token": if token.is_empty() { keep } else { token }}),
        );
        Ok(self.public_config())
    }

    fn get(&self, path: &str) -> Res<Value> {
        let c = self.load();
        if !truthy(c.get("url")) || !truthy(c.get("token")) {
            return fail(t("Plex ist noch nicht eingerichtet", &[]));
        }
        plex_call(
            "GET",
            &format!("{}{path}", base(&text_of(&c["url"]))),
            &text_of(&c["token"]),
        )
    }

    pub fn sections(&self) -> Res<Vec<Value>> {
        let data = self.get("/library/sections")?;
        Ok(data["MediaContainer"]["Directory"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d.is_object() && truthy(d.get("key")))
            .map(|d| {
                let paths: Vec<String> = d["Location"].as_array().into_iter().flatten().filter(|l| l.is_object()).map(|l| text_of(&l["path"])).collect();
                json!({"key": text_of(&d["key"]), "title": text_of(&d["title"]), "type": d["type"], "refreshing": truthy(d.get("refreshing")), "paths": paths})
            })
            .collect())
    }

    /// Check the connection; pick the audiobook library when it is clear which one it is.
    pub fn test(&self) -> Res<String> {
        let secs = self.sections()?;
        let mut c = self.load();
        if let Some(chosen) = secs.iter().find(|s| Some(&s["key"]) == c.get("section")) {
            c.insert("section_name".into(), chosen["title"].clone());
            let _ = self.db.put("plex", &Value::Object(c));
            return Ok(t(
                "verbunden, Mediathek „{name}“",
                &[("name", chosen["title"].as_str().unwrap_or(""))],
            ));
        }
        let music: Vec<&Value> = secs.iter().filter(|s| s["type"] == "artist").collect(); // Plex keeps audiobooks in music libraries
        let named: Vec<&Value> = music
            .iter()
            .copied()
            .filter(|s| {
                let paths: Vec<String> = s["paths"].as_array().into_iter().flatten().map(text_of).collect();
                rx::is_match(
                    rx!(r"(?i)h(ö|oe)rb|audio ?book|book"),
                    &format!("{}{}", text_of(&s["title"]), paths.join(" ")),
                )
            })
            .collect();
        let pick = if named.len() == 1 {
            named
        } else if music.len() == 1 {
            music
        } else {
            Vec::new()
        };
        if let Some(chosen) = pick.first() {
            c.insert("section".into(), chosen["key"].clone());
            c.insert("section_name".into(), chosen["title"].clone());
            let _ = self.db.put("plex", &Value::Object(c));
            return Ok(t(
                "verbunden, Mediathek „{name}“ gewählt",
                &[("name", chosen["title"].as_str().unwrap_or(""))],
            ));
        }
        let names: Vec<String> = secs
            .iter()
            .map(|s| format!("{} ({})", text_of(&s["title"]), text_of(&s["key"])))
            .collect();
        Ok(t(
            "verbunden – bitte Mediathek wählen: {names}",
            &[(
                "names",
                &if names.is_empty() {
                    "keine".to_string()
                } else {
                    names.join(", ")
                },
            )],
        ))
    }

    /// Refresh the section; with "empty trash" wait for the scan and remove entries whose files are gone. Blocking.
    pub fn scan_now(&self) -> Option<String> {
        if !self.configured() {
            return None;
        }
        let c = self.load();
        let (b, key, token) = (
            base(&text_of(&c["url"])),
            quote(&text_of(&c["section"])),
            text_of(&c["token"]),
        );
        let set = |m: String| *self.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(m);
        if let Err(e) = plex_call("GET", &format!("{b}/library/sections/{key}/refresh"), &token) {
            text::warn("Plex", &e.message);
            set(e.message.clone());
            return Some(e.message);
        }
        set(t("Scan angestoßen", &[]));
        text::info("Plex", &t("Plex-Mediathek wird neu eingelesen", &[]));
        if truthy(c.get("empty_trash")) {
            let end = now() + 900.0;
            std::thread::sleep(Duration::from_secs(2));
            while now() < end {
                let refreshing = self
                    .sections()
                    .ok()
                    .and_then(|s| s.into_iter().find(|x| x["key"] == c["section"]))
                    .is_some_and(|s| s["refreshing"] == true);
                if !refreshing {
                    if let Err(e) = plex_call("PUT", &format!("{b}/library/sections/{key}/emptyTrash"), &token) {
                        set(e.message.clone());
                        return Some(e.message);
                    }
                    set(t("Scan abgeschlossen, Papierkorb geleert", &[]));
                    text::info(
                        "Plex",
                        &t(
                            "Plex: Papierkorb geleert (entfernte Dateien sind aus der Mediathek verschwunden)",
                            &[],
                        ),
                    );
                    return None;
                }
                std::thread::sleep(Duration::from_secs(10));
            }
            set(t("Scan läuft noch – Papierkorb nicht geleert", &[]));
        }
        None
    }

    /// The server's accounts (owner, Plex Home profiles, friends): [{id, name}].
    pub fn accounts(&self) -> Res<Vec<Value>> {
        let data = self.get("/accounts")?;
        Ok(data["MediaContainer"]["Account"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a.is_object() && !matches!(text_of(&a["id"]).as_str(), "" | "0") && truthy(a.get("name")))
            .map(|a| json!({"id": text_of(&a["id"]), "name": text_of(&a["name"])}))
            .collect())
    }

    /// What this account played to the end in the audiobook library, newest first.
    pub fn history(&self, account_id: &str, since: f64, limit: usize) -> Res<Vec<Value>> {
        let c = self.load();
        let mut params = vec![
            ("accountID", account_id.to_string()),
            ("sort", "viewedAt:desc".into()),
            ("X-Plex-Container-Start", "0".into()),
            ("X-Plex-Container-Size", limit.to_string()),
        ];
        if truthy(c.get("section")) {
            params.push(("librarySectionID", text_of(&c["section"])));
        }
        let mut query = crate::audible::form(&params);
        if since > 0.0 {
            query.push_str(&format!("&viewedAt%3E={}", since as i64));
        }
        let data = self.get(&format!("/status/sessions/history/all?{query}"))?;
        let num = |v: &Value| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|x| x.parse().ok()))
                .unwrap_or(0.0)
        };
        Ok(data["MediaContainer"]["Metadata"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m.is_object() && !(since > 0.0 && num(&m["viewedAt"]) < since))
            .map(|m| {
                let book_key = if truthy(m.get("parentRatingKey")) { text_of(&m["parentRatingKey"]) } else { text_of(&m["ratingKey"]) };
                let book = if truthy(m.get("parentTitle")) { text_of(&m["parentTitle"]) } else { text_of(&m["title"]) };
                json!({"key": text_of(&m["ratingKey"]), "book_key": book_key, "title": text_of(&m["title"]), "book": book,
                       "author": text_of(&m["grandparentTitle"]), "viewed": num(&m["viewedAt"]) as i64})
            })
            .collect())
    }

    /// Seconds per track (rating key); remembered, since a track's length does not change.
    pub fn durations(&self, keys: &[String], limit: usize) -> HashMap<String, f64> {
        let mut known: Map<String, Value> = self
            .db
            .get("plex_durations")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut missing: Vec<&String> = Vec::new();
        for k in keys {
            if !k.is_empty() && !known.contains_key(k) && !missing.contains(&k) {
                missing.push(k);
            }
        }
        missing.truncate(limit);
        for k in &missing {
            let d = self
                .get(&format!("/library/metadata/{}", quote(k)))
                .ok()
                .and_then(|data| {
                    data["MediaContainer"]["Metadata"]
                        .as_array()
                        .and_then(|a| a.first())
                        .map(|m| m["duration"].as_f64().unwrap_or(0.0))
                })
                .unwrap_or(0.0);
            known.insert((*k).clone(), json!((d / 1000.0 * 10.0).round_ties_even() / 10.0));
        }
        if !missing.is_empty() {
            if known.len() > 20000 {
                let skip = known.len() - 20000;
                known = known.into_iter().skip(skip).collect();
            }
            let _ = self.db.put("plex_durations", &Value::Object(known.clone()));
        }
        keys.iter()
            .map(|k| (k.clone(), known.get(k).and_then(Value::as_f64).unwrap_or(0.0)))
            .collect()
    }
}

/// Media servers should look again after books were filed (each once things calm down, 30 s after the last).
pub fn request_scans(app: &Arc<crate::app::App>) {
    if app.abs.configured() {
        let a = app.clone();
        app.abs.debounce.later(Duration::from_secs(30), move || {
            if let Some(e) = a.abs.scan_now() {
                a.notifier
                    .send("abs_error", &t("Audiobookshelf-Scan fehlgeschlagen", &[]), &e, None);
            }
        });
    }
    if app.plex.configured() {
        let a = app.clone();
        app.plex.debounce.later(Duration::from_secs(30), move || {
            if let Some(e) = a.plex.scan_now() {
                a.notifier
                    .send("plex_error", &t("Plex-Scan fehlgeschlagen", &[]), &e, None);
            }
        });
    }
}
