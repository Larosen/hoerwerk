//! Free, legal audiobooks: LibriVox – volunteers read books that are in the public domain; the recordings are
//! public domain too. They are hosted by the Internet Archive (collection `librivoxaudio`), whose search knows
//! titles, authors and languages. A picked book is downloaded into the inbox with a `metadata.json`, so the
//! normal pipeline turns it into an M4B and files it – no Audible lookup, the details come from LibriVox.
//!
//! Only this one collection is ever searched or loaded, and only from archive.org hosts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::app::App;
use crate::audible;
use crate::rx;
use crate::rx::{is_match, sub};
use crate::text::{self, t};

const COLLECTION: &str = "librivoxaudio";
const SEARCH_URL: &str = "https://archive.org/advancedsearch.php";
const MAX_FILES: usize = 400;
const MAX_FILE: u64 = 400 << 20; // one chapter
const MAX_TOTAL: u64 = 4 << 30; // one book
const MAX_LOADING: usize = 2; // books at the same time
/// the MP3 variants the Internet Archive offers, best fit first (64 kbit/s mono is what LibriVox records)
const FORMATS: [&str; 4] = ["64Kbps MP3", "128Kbps MP3", "VBR MP3", "MP3"];

/// (url, most bytes) -> body
pub type Getter = dyn Fn(&str, u64) -> Result<Vec<u8>, String> + Send + Sync;

fn archive_host(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |x| x.1).to_lowercase();
    host == "archive.org" || host.ends_with(".archive.org")
}

/// The real download: HTTPS and archive.org hosts only – also after redirects (downloads move to ia*.archive.org).
pub fn http_get(url: &str, max: u64) -> Result<Vec<u8>, String> {
    audible::http_get(url, 120, 3, false, max, Some(&archive_host)).map_err(|e| e.0)
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        Value::Array(a) => a.first().map(text_of).unwrap_or_default(),
        other => other.to_string(),
    }
}

fn list_of(v: &Value) -> Vec<String> {
    match v {
        Value::Null => Vec::new(),
        Value::Array(a) => a.iter().map(text_of).filter(|x| !x.is_empty()).collect(),
        other => vec![text_of(other)].into_iter().filter(|x| !x.is_empty()).collect(),
    }
}

pub fn valid_id(id: &str) -> bool {
    is_match(rx!(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$"), id)
}

/// "German", "ger", "deu", "de" … -> "german" / "english" / the value itself in lower case
fn language_of(v: &Value) -> String {
    let all: Vec<String> = list_of(v).iter().map(|x| x.to_lowercase()).collect();
    for l in &all {
        match l.as_str() {
            "german" | "ger" | "deu" | "de" | "deutsch" => return "german".into(),
            "english" | "eng" | "en" => return "english".into(),
            _ => {}
        }
    }
    all.first().cloned().unwrap_or_default()
}

/// "5:23:11", "23:11" or seconds -> minutes
fn runtime_min(v: &Value) -> Value {
    let raw = text_of(v);
    let parts: Vec<f64> = raw.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    if parts.is_empty() || parts.len() != raw.split(':').count() {
        return Value::Null;
    }
    let secs = parts.iter().fold(0.0, |acc, p| acc * 60.0 + p);
    if secs <= 0.0 {
        Value::Null
    } else {
        json!((secs / 60.0).round() as i64)
    }
}

/// "Storm, Theodor, 1817-1888" -> "Theodor Storm"
fn author_name(raw: &str) -> String {
    let parts: Vec<&str> = raw.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    let name: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| !is_match(rx!(r"^[\d\s?–-]+$"), p))
        .collect();
    match name.as_slice() {
        [last, first] => format!("{first} {last}"),
        _ => name.join(", "),
    }
}

/// The search address for a text and a language ("de", "en" or "" for any).
pub fn search_url(query: &str, language: &str, rows: usize) -> Option<String> {
    let words: Vec<String> = sub(rx!(r"[^\w\s]"), query, " ")
        .split_whitespace()
        .filter(|w| !matches!(w.to_uppercase().as_str(), "AND" | "OR" | "NOT" | "TO"))
        .take(8)
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        return None;
    }
    let lang = match language {
        "de" => " AND language:(German OR ger OR deu OR de)",
        "en" => " AND language:(English OR eng OR en)",
        _ => "",
    };
    let q = format!(
        "collection:{COLLECTION} AND mediatype:audio AND ({}){lang}",
        words.join(" AND ")
    );
    let mut params = vec![("q", q)];
    for f in ["identifier", "title", "creator", "language", "runtime", "downloads"] {
        params.push(("fl[]", f.to_string()));
    }
    params.push(("sort[]", "downloads desc".into()));
    params.push(("rows", rows.to_string()));
    params.push(("output", "json".into()));
    Some(format!("{SEARCH_URL}?{}", audible::form(&params)))
}

/// One hit of the search as the page shows it.
fn normalize(d: &Value) -> Option<Value> {
    let id = text_of(&d["identifier"]);
    let title = text_of(&d["title"]).trim().to_string();
    if !valid_id(&id) || title.is_empty() {
        return None;
    }
    let authors: Vec<String> = list_of(&d["creator"])
        .iter()
        .map(|a| author_name(a))
        .filter(|a| !a.is_empty())
        .collect();
    Some(json!({
        "id": id, "title": title, "authors": authors, "language": language_of(&d["language"]),
        "runtime_min": runtime_min(&d["runtime"]),
        "url": format!("https://archive.org/details/{id}"),
        "cover_url": format!("https://archive.org/services/img/{id}"),
        "source": "librivox",
    }))
}

pub fn parse_search(body: &[u8]) -> Result<Vec<Value>, String> {
    let data: Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    Ok(data["response"]["docs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(normalize)
        .collect())
}

/// What to download for one item: (book details for metadata.json, [(file name, size)], cover file name).
/// (book details for metadata.json, [(file name, size)], cover file name)
pub type Plan = (Value, Vec<(String, u64)>, Option<String>);

pub fn plan(id: &str, meta: &Value) -> Result<Plan, String> {
    let m = &meta["metadata"];
    if !list_of(&m["collection"]).iter().any(|c| c == COLLECTION) {
        return Err(t(
            "Das ist keine LibriVox-Aufnahme – nur gemeinfreie Hörbücher von LibriVox werden geladen",
            &[],
        ));
    }
    let size = |f: &Value| text_of(&f["size"]).parse::<u64>().unwrap_or(0);
    let safe =
        |n: &str| !n.is_empty() && !n.contains('/') && !n.contains('\\') && !n.starts_with('.') && !n.contains("..");
    let all: Vec<&Value> = meta["files"].as_array().into_iter().flatten().collect();
    let mut files: Vec<(String, u64)> = Vec::new();
    for format in FORMATS {
        files = all
            .iter()
            .filter(|f| text_of(&f["format"]) == format)
            .map(|f| (text_of(&f["name"]), size(f)))
            .filter(|(n, _)| safe(n) && n.to_lowercase().ends_with(".mp3"))
            .collect();
        if !files.is_empty() {
            break;
        }
    }
    if files.is_empty() {
        return Err(t("Keine MP3-Dateien in dieser Aufnahme gefunden", &[]));
    }
    files.sort_by_key(|f| crate::words::natural_key(&f.0));
    let total: u64 = files.iter().map(|f| f.1).sum();
    if files.len() > MAX_FILES || total > MAX_TOTAL || files.iter().any(|f| f.1 > MAX_FILE) {
        return Err(t("Diese Aufnahme ist zu groß", &[]));
    }
    let cover = all
        .iter()
        // the JPEG is the cover (PNGs are waveform pictures of the chapters)
        .filter(|f| text_of(&f["format"]) == "JPEG")
        .map(|f| (text_of(&f["name"]), size(f)))
        .filter(|(n, s)| safe(n) && *s > 0 && *s < (10 << 20))
        .max_by_key(|f| f.1)
        .map(|f| f.0);
    let title = text_of(&m["title"]).trim().to_string();
    if title.is_empty() {
        return Err(t("Aufnahme ohne Titel", &[]));
    }
    let authors: Vec<String> = list_of(&m["creator"])
        .iter()
        .map(|a| author_name(a))
        .filter(|a| !a.is_empty())
        .collect();
    let language = match language_of(&m["language"]).as_str() {
        "german" => "German".to_string(),
        "english" => "English".to_string(),
        other => audible::capitalize(other),
    };
    let description: String = audible::html_to_text(&text_of(&m["description"]))
        .chars()
        .take(4000)
        .collect();
    // "Read in German by Felix" – the volunteer who read it; group recordings stay "LibriVox"
    let narrator = rx::search(rx!(r"(?i)\bread (?:in [\w ]{2,20} )?by ([^\n.<]{2,60})"), &description)
        .map(|c| c[1].trim().to_string())
        .filter(|n| !n.is_empty() && !n.to_lowercase().contains("librivox"))
        .unwrap_or_else(|| "LibriVox".to_string());
    let date = text_of(&m["date"]);
    let book = json!({
        "title": title, "authors": authors, "narrators": [narrator], "publisher": "LibriVox",
        "language": language, "description": description,
        "releaseDate": if is_match(rx!(r"^\d{4}(-\d\d-\d\d)?"), &date) { json!(date) } else { Value::Null },
        "genres": ["Klassiker"], "librivox": format!("https://archive.org/details/{id}"),
    });
    Ok((book, files, cover))
}

#[derive(Default)]
struct State {
    /// id -> {id, title, status: loading | done | failed, done, total, error}
    jobs: HashMap<String, Value>,
}

pub struct Free {
    get: Box<Getter>,
    state: Mutex<State>,
}

impl Default for Free {
    fn default() -> Self {
        Self::with_getter(Box::new(http_get))
    }
}

impl Free {
    pub fn with_getter(get: Box<Getter>) -> Self {
        Free {
            get,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// LibriVox recordings for a text; `state` tells which of them are being loaded right now.
    pub fn search(&self, query: &str, language: &str) -> Result<Vec<Value>, String> {
        let Some(url) = search_url(query, language, 30) else {
            return Ok(Vec::new());
        };
        let body = (self.get)(&url, 4 << 20).map_err(|e| t("LibriVox nicht erreichbar: {error}", &[("error", &e)]))?;
        let mut found = parse_search(&body)?;
        let st = self.state();
        for f in &mut found {
            if let Some(j) = st.jobs.get(f["id"].as_str().unwrap_or("")) {
                f["state"] = j["status"].clone();
            }
        }
        Ok(found)
    }

    pub fn jobs(&self) -> Vec<Value> {
        self.state().jobs.values().cloned().collect()
    }

    fn set(&self, id: &str, change: Value) {
        let mut st = self.state();
        let job = st.jobs.entry(id.to_string()).or_insert_with(|| json!({"id": id}));
        if let (Some(j), Some(c)) = (job.as_object_mut(), change.as_object()) {
            j.extend(c.clone());
        }
    }

    /// Start loading one recording into the inbox (in the background). Answers what is being loaded.
    pub fn fetch(&self, app: &Arc<App>, id: &str) -> Result<Value, String> {
        if !valid_id(id) {
            return Err(t("Ungültige Daten", &[]));
        }
        {
            let st = self.state();
            if st.jobs.get(id).is_some_and(|j| j["status"] == "loading") {
                return Err(t("Wird schon geladen", &[]));
            }
            if st.jobs.values().filter(|j| j["status"] == "loading").count() >= MAX_LOADING {
                return Err(t("Es werden schon zwei Hörbücher geladen – bitte kurz warten", &[]));
            }
        }
        let meta_raw = (self.get)(&format!("https://archive.org/metadata/{id}"), 8 << 20)
            .map_err(|e| t("LibriVox nicht erreichbar: {error}", &[("error", &e)]))?;
        let meta: Value = serde_json::from_slice(&meta_raw).map_err(|e| e.to_string())?;
        let (book, files, cover) = plan(id, &meta)?;
        let title = book["title"].as_str().unwrap_or("").to_string();
        self.set(
            id,
            json!({"title": title, "status": "loading", "done": 0, "total": files.len(), "error": null}),
        );
        let answer = json!({"id": id, "title": title, "files": files.len()});
        let (app2, id2, lang) = (app.clone(), id.to_string(), text::language());
        let spawned = std::thread::Builder::new().name("librivox".into()).spawn(move || {
            text::set_language(&lang);
            let free = &app2.free;
            match free.load(&app2, &id2, &book, &files, cover.as_deref()) {
                Ok(folder) => {
                    free.set(&id2, json!({"status": "done"}));
                    text::info(
                        "LibriVox",
                        &t("{title} geladen – der Eingang übernimmt", &[("title", &title)]),
                    );
                    app2.notifier
                        .send("download_done", &t("Download fertig", &[]), &folder, None);
                    app2.pipeline.scan_now();
                }
                Err(e) => {
                    free.set(&id2, json!({"status": "failed", "error": e}));
                    text::error("LibriVox", &t("{title}: {error}", &[("title", &title), ("error", &e)]));
                    app2.notifier.send(
                        "download_failed",
                        &t("Download fehlgeschlagen", &[]),
                        &format!("{title}\n{e}"),
                        Some(4),
                    );
                }
            }
        });
        if let Err(e) = spawned {
            self.set(id, json!({"status": "failed", "error": e.to_string()}));
            return Err(e.to_string());
        }
        Ok(answer)
    }

    /// Download into a hidden folder of the inbox, then move it in with one rename – like a finished download.
    fn load(
        &self,
        app: &App,
        id: &str,
        book: &Value,
        files: &[(String, u64)],
        cover: Option<&str>,
    ) -> Result<String, String> {
        let inbox = &app.cfg.inbox;
        let staging = inbox.join(format!(".librivox-{id}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        let result = (|| -> Result<PathBuf, String> {
            let base = format!("https://archive.org/download/{id}/");
            let enc =
                |n: &str| percent_encoding::utf8_percent_encode(n, percent_encoding::NON_ALPHANUMERIC).to_string();
            for (n, (name, _)) in files.iter().enumerate() {
                if app.stopping() {
                    return Err(t("Beim Beenden unterbrochen", &[]));
                }
                let data = (self.get)(&format!("{base}{}", enc(name)), MAX_FILE)?;
                if !(data.starts_with(b"ID3") || data.first() == Some(&0xff)) {
                    return Err(t("{file} ist keine MP3-Datei", &[("file", name)]));
                }
                write_file(&staging.join(name), &data)?;
                self.set(id, json!({"done": n + 1}));
            }
            if let Some(c) = cover
                && let Ok(img) = (self.get)(&format!("{base}{}", enc(c)), 10 << 20)
                && (img.starts_with(b"\xff\xd8\xff") || img.starts_with(b"\x89PNG\r\n\x1a\n"))
            {
                let ext = if img.starts_with(b"\x89PNG") { "png" } else { "jpg" };
                write_file(&staging.join(format!("cover.{ext}")), &img)?;
            }
            let meta = serde_json::to_vec_pretty(book).map_err(|e| e.to_string())?;
            write_file(&staging.join("metadata.json"), &meta)?;
            let author = book["authors"][0].as_str().unwrap_or("");
            let title = book["title"].as_str().unwrap_or("");
            let name = crate::filing::sanitize(&if author.is_empty() {
                title.to_string()
            } else {
                format!("{author} - {title}")
            });
            let dest = crate::filing::unique_folder(&inbox.join(if name.is_empty() { id.to_string() } else { name }));
            std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
            Ok(dest)
        })();
        match result {
            Ok(dest) => Ok(dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }
}

fn write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| e.to_string())
}

/// Half-loaded books of an earlier run (the container was stopped meanwhile) go away at the start.
pub fn cleanup(inbox: &Path) {
    let Ok(rd) = std::fs::read_dir(inbox) else { return };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(".librivox-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_address_and_hits() {
        let url = search_url("Storm: Der Schimmelreiter OR x", "de", 30).unwrap();
        assert!(url.starts_with("https://archive.org/advancedsearch.php?q=collection%3Alibrivoxaudio"));
        assert!(url.contains("Storm+AND+Der+AND+Schimmelreiter+AND+x"));
        assert!(url.contains("language%3A%28German"));
        assert!(search_url("  ?! ", "de", 30).is_none());
        assert!(archive_host("https://ia800.us.archive.org/1/x.mp3") && archive_host("https://archive.org/metadata/x"));
        assert!(!archive_host("http://archive.org/x") && !archive_host("https://archive.org.evil.example/x"));
        assert!(
            !archive_host("https://evil.example/archive.org") && !archive_host("https://archive.org@evil.example/")
        );
        let body = json!({"response": {"docs": [
            {"identifier": "schimmelreiter_1202_librivox", "title": "Der Schimmelreiter", "creator": "Storm, Theodor, 1817-1888",
             "language": ["ger"], "runtime": "4:51:09", "item_size": 140000000},
            {"identifier": "../etc", "title": "Böse"},
            {"identifier": "alice_librivox", "title": "Alice's Adventures in Wonderland", "creator": ["Lewis Carroll"], "language": "English"}]}});
        let got = parse_search(body.to_string().as_bytes()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["authors"], json!(["Theodor Storm"]));
        assert_eq!(
            (got[0]["language"].clone(), got[0]["runtime_min"].clone()),
            (json!("german"), json!(291))
        );
        assert_eq!(got[1]["language"], json!("english"));
    }

    #[test]
    fn only_librivox_and_only_safe_files() {
        crate::text::set_language("de");
        let files = json!([
            {"name": "schimmelreiter_02_storm_64kb.mp3", "format": "64Kbps MP3", "size": "9000000"},
            {"name": "schimmelreiter_01_storm_64kb.mp3", "format": "64Kbps MP3", "size": "8000000"},
            {"name": "schimmelreiter_01_storm.mp3", "format": "VBR MP3", "size": "20000000"},
            {"name": "../../etc/passwd.mp3", "format": "64Kbps MP3", "size": "5"},
            {"name": "sub/x.mp3", "format": "64Kbps MP3", "size": "5"},
            {"name": "cover_small.jpg", "format": "JPEG", "size": "20000"},
            {"name": "cover_big.jpg", "format": "JPEG", "size": "300000"},
            {"name": "wave_01.png", "format": "PNG", "size": "900000"},
            {"name": "schimmelreiter.zip", "format": "ZIP", "size": "999"}]);
        let meta = json!({"metadata": {"collection": ["librivoxaudio", "audio_bookspoetry"], "title": "Der Schimmelreiter",
            "creator": "Storm, Theodor", "language": "ger", "description": "LibriVox recording.<br>Read in German by Felix<br><p>Eine <b>Novelle</b>.</p>", "date": "2012-02-20"},
            "files": files});
        let (book, got, cover) = plan("schimmelreiter_1202_librivox", &meta).unwrap();
        assert_eq!(
            got.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
            ["schimmelreiter_01_storm_64kb.mp3", "schimmelreiter_02_storm_64kb.mp3"]
        );
        assert_eq!(cover.as_deref(), Some("cover_big.jpg"));
        assert_eq!(
            (book["authors"].clone(), book["language"].clone()),
            (json!(["Theodor Storm"]), json!("German"))
        );
        assert_eq!(book["narrators"], json!(["Felix"]));
        assert!(book["description"].as_str().unwrap().ends_with("Eine Novelle."));
        // anything outside the LibriVox collection is refused, whatever the item says about itself
        let other = json!({"metadata": {"collection": ["opensource_audio"], "title": "Irgendwas"}, "files": files});
        assert!(plan("irgendwas", &other).is_err());
        assert!(valid_id("schimmelreiter_1202_librivox") && !valid_id("../x") && !valid_id("a/b") && !valid_id(""));
    }
}
