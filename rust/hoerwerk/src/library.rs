//! What is already in the library – so nothing gets downloaded or filed twice.
//! Covers the books Hörwerk filed (M4B with ASIN tags) and older collections: folders of MP3s/M4As/… are
//! recognised by their tags or, failing that, by the usual "Author/Title" or "Author/Series/Title" layout.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};

use regex_lite::Regex;
use serde_json::{Value, json};

use crate::db::{Db, compact};
use crate::media;
use crate::text::now;
use crate::words::{match_words, names_of, split_author_title, surnames, title_words};

pub const COVER_FILES: [&str; 7] = [
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.png",
    "front.jpg",
    "front.png",
];
const AUDIO_EXTS: [&str; 9] = [".m4b", ".m4a", ".mp3", ".mp4", ".aac", ".flac", ".ogg", ".opus", ".wma"];
/// bump when the way entries are read changes: cached folders are read again
const ENTRY_VERSION: i64 = 5;
const PUBLIC_KEYS: [&str; 11] = [
    "path",
    "title",
    "authors",
    "names",
    "series",
    "series_position",
    "asin",
    "format",
    "files",
    "mtime",
    "cover",
];

/// "CD1", "CD 2", "Disc 3", "Teil 1" … are parts of one book, not books of their own
static DISC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)[\s_.-]*\d+\b").expect("valid regex")
});
static VOLUME_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d+(?:[.,]\d+)?\s*[-–.]\s*").expect("valid regex"));
static VOLUME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d{1,3})(?:[.,]\d+)?\s*[-–.]").expect("valid regex"));
static LEADING_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d{1,3})").expect("valid regex"));

#[derive(Default)]
struct State {
    entries: HashMap<String, Value>,
    last_scan: Option<f64>,
    scanning: bool,
    /// changes by add()/remove() while a scan walks the disk (path -> entry, None = removed): applied on top of the
    /// scan's older picture at the end instead of being overwritten
    during: Option<HashMap<String, Option<Value>>>,
}

pub struct LibraryIndex {
    pub root: PathBuf,
    db: Db,
    state: Mutex<State>,
    notify: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

fn s<'a>(e: &'a Value, k: &str) -> &'a str {
    e[k].as_str().unwrap_or("")
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn has_cover_file(folder: &Path) -> bool {
    COVER_FILES.iter().any(|n| media::is_regular(&folder.join(n)))
}

/// Path order of the library list: digits compared as text, case ignored.
fn path_key(p: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut digits = false;
    for c in p.chars() {
        if c.is_ascii_digit() != digits {
            out.push(std::mem::take(&mut cur).to_lowercase());
            digits = c.is_ascii_digit();
        }
        cur.push(c);
    }
    out.push(cur.to_lowercase());
    if digits {
        out.push(String::new());
    }
    out
}

impl LibraryIndex {
    pub fn new(root: &Path, db: Db) -> Self {
        let root = if root.as_os_str().is_empty() {
            PathBuf::new()
        } else {
            std::fs::canonicalize(root).unwrap_or(root.to_path_buf())
        };
        let idx = LibraryIndex {
            root,
            db,
            state: Mutex::new(State::default()),
            notify: OnceLock::new(),
        };
        idx.load();
        idx
    }

    pub fn on_change(&self, f: Box<dyn Fn() + Send + Sync>) {
        let _ = self.notify.set(f);
    }

    fn changed(&self) {
        if let Some(f) = self.notify.get() {
            f();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn load(&self) {
        let mut entries = HashMap::new();
        {
            let c = self.db.lock();
            if let Ok(mut st) = c.prepare(
                "SELECT path, title, authors, asin, series, series_position, format, files, mtime FROM library",
            ) {
                let rows = st.query_map([], |r| {
                    Ok(json!({
                        "path": r.get::<_, String>(0)?, "title": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        "authors": serde_json::from_str::<Value>(&r.get::<_, Option<String>>(2)?.unwrap_or_default()).unwrap_or(json!([])),
                        "asin": r.get::<_, Option<String>>(3)?, "series": r.get::<_, Option<String>>(4)?,
                        "series_position": r.get::<_, Option<String>>(5)?, "format": r.get::<_, Option<String>>(6)?,
                        "files": r.get::<_, Option<i64>>(7)?.unwrap_or(0), "mtime": r.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                    }))
                });
                if let Ok(rows) = rows {
                    for e in rows.flatten() {
                        entries.insert(s(&e, "path").to_string(), e);
                    }
                }
            }
        }
        let last = self.db.get("library_last_scan").and_then(|v| v.as_f64());
        let mut st = self.lock();
        st.entries = entries;
        st.last_scan = last;
    }

    fn store(&self, changed: &[Value], removed: &[String], last_scan: Option<f64>) -> rusqlite::Result<()> {
        let mut c = self.db.lock();
        let tx = c.transaction()?;
        for path in removed {
            tx.execute("DELETE FROM library WHERE path=?", [path])?;
        }
        for e in changed {
            tx.execute(
                "INSERT OR REPLACE INTO library(path, title, authors, asin, series, series_position, format, files, mtime) \
                 VALUES(?,?,?,?,?,?,?,?,?)",
                rusqlite::params![
                    s(e, "path"),
                    s(e, "title"),
                    compact(&json!(strs(&e["authors"]))),
                    e["asin"].as_str(),
                    e["series"].as_str(),
                    match &e["series_position"] {
                        Value::Null => None,
                        Value::String(x) => Some(x.clone()),
                        other => Some(other.to_string()),
                    },
                    e["format"].as_str(),
                    e["files"].as_i64().unwrap_or(0),
                    e["mtime"].as_f64().unwrap_or(0.0),
                ],
            )?;
        }
        if let Some(t) = last_scan {
            tx.execute(
                "INSERT OR REPLACE INTO kv(key, value) VALUES('library_last_scan', ?)",
                [compact(&json!(t))],
            )?;
        }
        tx.commit()
    }

    // -- scanning ------------------------------------------------------------------------------------
    /// Walk the library; unchanged folders come from the cache. Blocking – run it off the async threads.
    pub fn scan(&self) -> Result<usize, String> {
        if self.root.as_os_str().is_empty() || !self.root.is_dir() {
            return Ok(0);
        }
        {
            let mut st = self.lock();
            if st.scanning {
                return Ok(st.entries.len());
            }
            st.scanning = true;
            st.during = Some(HashMap::new());
        }
        self.changed();
        let result = self.walk_and_store();
        {
            let mut st = self.lock();
            st.scanning = false;
            st.during = None;
        }
        self.changed();
        result
    }

    fn walk(&self) -> BTreeMap<String, Vec<String>> {
        let mut books: BTreeMap<String, Vec<String>> = BTreeMap::new();
        // depth first in sorted order, like os.walk with sorted dirs
        let mut stack = vec![self.root.clone()];
        while let Some(folder) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&folder) else { continue };
            let (mut dirs, mut audio) = (Vec::new(), Vec::new());
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let path = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if !name.starts_with(['.', '_']) {
                        dirs.push(name);
                    }
                } else if !name.starts_with('.')
                    && AUDIO_EXTS.iter().any(|x| name.to_lowercase().ends_with(x))
                    && path.is_file()
                {
                    audio.push(name);
                }
            }
            dirs.sort();
            for d in dirs.iter().rev() {
                stack.push(folder.join(d));
            }
            if audio.is_empty() {
                continue;
            }
            audio.sort();
            let rel = rel_of(&self.root, &folder);
            let base = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parent = rel.rsplit_once('/').map(|x| x.0.to_string());
            let (book, sub) = match parent {
                Some(p) if DISC.is_match(&base) => (p, base), // CD1 belongs to its parent
                _ => (rel, String::new()),
            };
            books.entry(book).or_default().extend(
                audio
                    .into_iter()
                    .map(|f| if sub.is_empty() { f } else { format!("{sub}/{f}") }),
            );
        }
        books
    }

    fn walk_and_store(&self) -> Result<usize, String> {
        let books = self.walk();
        let old: HashMap<String, Value> = self.lock().entries.clone();
        let mut found: HashMap<String, Value> = HashMap::new();
        let mut changed: Vec<Value> = Vec::new();
        for (rel, mut audio) in books {
            let folder = if rel == "." {
                self.root.clone()
            } else {
                self.root.join(&rel)
            };
            let Some(mtime) = audio
                .iter()
                .map(|f| media::mtime(&folder.join(f)))
                .try_fold(f64::MIN, |m, x| x.map(|x| m.max(x)))
            else {
                continue;
            };
            audio.sort();
            if let Some(o) = old.get(&rel)
                && o["mtime"].as_f64() == Some(mtime)
                && o["files"].as_u64() == Some(audio.len() as u64)
                && o["v"].as_i64() == Some(ENTRY_VERSION)
            {
                found.insert(rel, o.clone());
                continue;
            }
            let e = entry(&folder, &rel, &audio, mtime);
            changed.push(e.clone());
            found.insert(rel, e);
        }
        let mut st = self.lock();
        let during = st.during.clone().unwrap_or_default();
        for (rel, e) in &during {
            match e {
                None => {
                    found.remove(rel);
                }
                Some(e) => {
                    found.insert(rel.clone(), e.clone());
                }
            }
        }
        changed.retain(|c| !during.contains_key(s(c, "path")));
        let removed: Vec<String> = st.entries.keys().filter(|p| !found.contains_key(*p)).cloned().collect();
        let n = found.len();
        st.entries = found;
        st.last_scan = Some(now());
        self.store(&changed, &removed, st.last_scan)
            .map_err(|e| e.to_string())?;
        Ok(n)
    }

    /// Register a freshly filed book without waiting for the next scan.
    pub fn add(&self, dest_file: &Path, book: &Value) {
        let folder = dest_file.parent().unwrap_or(Path::new(""));
        let rel = rel_of(&self.root, folder);
        let authors = strs(&book["authors"]);
        let author_refs: Vec<&str> = authors.iter().map(String::as_str).collect();
        let asin = s(book, "asin").to_uppercase();
        let e = json!({
            "path": rel, "files": 1, "mtime": media::mtime(dest_file).unwrap_or(0.0),
            "title": s(book, "title"), "authors": authors, "asin": if asin.is_empty() { Value::Null } else { json!(asin) },
            "series": book["series"].clone(), "series_position": book["series_position"].clone(), "format": "m4b",
            "v": ENTRY_VERSION, "names": names_of(&author_refs),
            "cover": has_cover_file(folder) || media::read_book(dest_file)["_cover"].as_bool().unwrap_or(false),
        });
        {
            let mut st = self.lock();
            st.entries.insert(rel.clone(), e.clone());
            if let Some(d) = st.during.as_mut() {
                d.insert(rel, Some(e.clone()));
            }
            let _ = self.store(&[e], &[], None);
        }
        self.changed();
    }

    pub fn remove(&self, dest_file: &Path) {
        let rel = rel_of(&self.root, dest_file.parent().unwrap_or(Path::new("")));
        {
            let mut st = self.lock();
            if let Some(d) = st.during.as_mut() {
                d.insert(rel.clone(), None);
            }
            if st.entries.remove(&rel).is_some() {
                let _ = self.store(&[], &[rel], None);
            }
        }
        self.changed();
    }

    // -- lookups -------------------------------------------------------------------------------------
    pub fn entries(&self) -> Vec<Value> {
        self.lock().entries.values().cloned().collect()
    }

    pub fn entry(&self, rel: &str) -> Option<Value> {
        self.lock().entries.get(rel).cloned()
    }

    pub fn scanning(&self) -> bool {
        self.lock().scanning
    }

    fn names(e: &Value) -> BTreeSet<String> {
        let mut all = strs(&e["authors"]);
        all.extend(strs(&e["names"]));
        surnames(&all)
    }

    /// Library entries that are this book: same ASIN, same author and title, or same series and volume.
    pub fn find(&self, book: &Value) -> Vec<Value> {
        let asin = s(book, "asin").to_uppercase();
        let authors = strs(&book["authors"]);
        let want = surnames(&names_of(&authors.iter().map(String::as_str).collect::<Vec<_>>()));
        let words = title_words(s(book, "title"));
        let series = s(book, "series").trim().to_lowercase();
        let pos = py_str(&book["series_position"]);
        let mut out = Vec::new();
        for e in self.entries() {
            if !asin.is_empty() && s(&e, "asin") == asin {
                out.push(with_why(e, "asin"));
                continue;
            }
            let names = Self::names(&e);
            let shared = !want.is_disjoint(&names);
            if !words.is_empty() && shared && title_words(s(&e, "title")) == words {
                out.push(with_why(e, "title"));
            } else if !series.is_empty()
                && !pos.is_empty()
                && shared
                && s(&e, "series").trim().to_lowercase() == series
                && py_str(&e["series_position"]) == pos
            {
                out.push(with_why(e, "series"));
            }
        }
        out
    }

    /// Library books whose every title word appears in a release name (or a search term). Normally the author's
    /// surname must be there too; titles of two or more significant words also count without it. `lenient`
    /// (for what the user typed) accepts one-word titles without the author too.
    pub fn match_release(&self, release: &str, lenient: bool) -> Vec<Value> {
        let words: BTreeSet<String> = match_words(release).into_iter().collect();
        let mut hits = Vec::new();
        for e in self.entries() {
            let tw = title_words(s(&e, "title"));
            if tw.is_empty() || !tw.iter().all(|w| words.contains(w)) {
                continue;
            }
            if !Self::names(&e).is_disjoint(&words) {
                hits.push(with_why(e, "title"));
            } else if lenient || tw.len() >= 2 {
                hits.push(with_why(e, "similar"));
            }
        }
        hits.sort_by_key(|e| {
            (
                s(e, "why") != "title",
                std::cmp::Reverse(title_words(s(e, "title")).len()),
            )
        });
        hits
    }

    /// Volumes of a series that are in the library: {volume: folder}.
    pub fn series_owned(&self, series: &str, author: &str) -> BTreeMap<i64, String> {
        let norm = |x: &str| {
            title_words(x)
                .into_iter()
                .filter(|w| !["reihe", "serie", "series", "saga", "trilogie"].contains(&w.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let want = norm(series);
        let wanted = if author.is_empty() {
            BTreeSet::new()
        } else {
            surnames(&names_of(&[author]))
        };
        let mut out = BTreeMap::new();
        if want.is_empty() {
            return out;
        }
        for e in self.entries() {
            if norm(s(&e, "series")) != want || (!wanted.is_empty() && wanted.is_disjoint(&Self::names(&e))) {
                continue;
            }
            if let Some(n) = LEADING_NUMBER
                .captures(&py_str(&e["series_position"]))
                .and_then(|c| c[1].parse().ok())
            {
                out.entry(n).or_insert_with(|| s(&e, "path").to_string());
            }
        }
        out
    }

    /// Browse the library: every word of the query in title, author, series or path.
    pub fn books(&self, query: &str, offset: usize, limit: usize) -> Value {
        let want = match_words(query);
        let mut entries = self.entries();
        if !want.is_empty() {
            entries.retain(|e| {
                let text = [
                    s(e, "title").to_string(),
                    strs(&e["authors"]).join(" "),
                    strs(&e["names"]).join(" "),
                    s(e, "series").to_string(),
                    s(e, "path").to_string(),
                ]
                .join(" ");
                let have: BTreeSet<String> = match_words(&text).into_iter().collect();
                want.iter().all(|w| have.iter().any(|t| t.starts_with(w.as_str())))
            });
        }
        entries.sort_by_key(|e| path_key(s(e, "path")));
        let total = entries.len();
        let books: Vec<Value> = entries
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|e| {
                Value::Object(
                    PUBLIC_KEYS
                        .iter()
                        .map(|k| (k.to_string(), e.get(*k).cloned().unwrap_or(Value::Null)))
                        .collect(),
                )
            })
            .collect();
        json!({"total": total, "offset": offset, "books": books})
    }

    pub fn stats(&self) -> Value {
        let st = self.lock();
        let n = st.entries.len();
        let m4b = st.entries.values().filter(|e| e["format"] == "m4b").count();
        json!({"books": n, "m4b": m4b, "other": n - m4b, "last_scan": st.last_scan, "scanning": st.scanning})
    }
}

fn with_why(mut e: Value, why: &str) -> Value {
    e["why"] = json!(why);
    e
}

/// str() of a JSON value the way Python prints it for series positions ("" for None).
fn py_str(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        other => other.to_string(),
    }
}

/// os.path.relpath for a folder inside the library ("." for the library itself).
pub fn rel_of(root: &Path, folder: &Path) -> String {
    match folder.strip_prefix(root) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r.display().to_string(),
        Err(_) => folder.display().to_string(),
    }
}

/// The library entry of one book folder: tags of the M4B (or of the first audio file), folder names as fallback.
fn entry(folder: &Path, rel: &str, audio: &[String], mtime: f64) -> Value {
    let mut cover = has_cover_file(folder);
    let m4b = audio.iter().find(|f| {
        let l = f.to_lowercase();
        l.ends_with(".m4b") || l.ends_with(".m4a")
    });
    let (book, mut names): (Value, Vec<String>) = if let Some(m) = m4b {
        let b = media::read_book(&folder.join(m));
        let names = names_of(&strs(&b["authors"]).iter().map(String::as_str).collect::<Vec<_>>());
        (b, names)
    } else {
        let info = media::probe_cached(&folder.join(&audio[0]));
        cover = cover || !info["cover_codec"].is_null();
        let tags = &info["tags"];
        let names = names_of(&[s(tags, "album_artist"), s(tags, "artist"), s(tags, "composer")]);
        let asin = if s(tags, "asin").is_empty() {
            tags["audible_asin"].clone()
        } else {
            tags["asin"].clone()
        };
        (
            json!({"title": s(tags, "album"), "authors": names.iter().take(1).collect::<Vec<_>>(), "asin": asin}),
            names,
        )
    };
    let parts: Vec<&str> = rel.split('/').collect();
    let last = parts[parts.len() - 1];
    let mut folder_author = (parts.len() > 1).then(|| parts[0].to_string());
    let mut folder_title = VOLUME_PREFIX.replace(last, "").into_owned();
    if folder_author.is_none()
        && let Some((a, t)) = split_author_title(last)
    {
        folder_author = Some(a);
        folder_title = t;
    }
    if let Some(a) = &folder_author {
        for n in names_of(&[a]) {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    let mut title = if s(&book, "title").is_empty() {
        folder_title
    } else {
        s(&book, "title").to_string()
    };
    // album tags like "Theodor Storm - Der Schimmelreiter": the author is not part of the title
    if let Some((a, t)) = split_author_title(&title)
        && !surnames(&names_of(&[&a])).is_disjoint(&surnames(&names))
    {
        title = t;
    }
    // M4B: our own tags. MP3 & co.: the artist tag is often the narrator – a folder named after the author wins
    let tag_authors = strs(&book["authors"]);
    let authors: Vec<String> = match (&folder_author, m4b) {
        (Some(a), None) => vec![a.clone()],
        _ if !tag_authors.is_empty() => tag_authors,
        (Some(a), _) => vec![a.clone()],
        _ => Vec::new(),
    };
    let asin = s(&book, "asin").to_uppercase();
    let series = match s(&book, "series") {
        "" => (parts.len() > 2)
            .then(|| parts[1].to_string())
            .map(Value::from)
            .unwrap_or(Value::Null),
        x => json!(x),
    };
    let position = match &book["series_position"] {
        Value::String(x) if !x.is_empty() => json!(x),
        Value::Null | Value::String(_) => (parts.len() > 2)
            .then(|| VOLUME.captures(last).map(|c| c[1].to_string()))
            .flatten()
            .map(Value::from)
            .unwrap_or(Value::Null),
        other => other.clone(),
    };
    let format = if m4b.is_some() {
        "m4b".to_string()
    } else {
        audio[0]
            .rsplit_once('.')
            .map(|x| x.1.to_lowercase())
            .unwrap_or_default()
    };
    json!({
        "v": ENTRY_VERSION, "path": rel, "files": audio.len(), "mtime": mtime, "names": names,
        "cover": cover || book["_cover"].as_bool().unwrap_or(false),
        "title": title, "authors": authors, "asin": if asin.is_empty() { Value::Null } else { json!(asin) },
        "series": series, "series_position": position, "format": format,
    })
}
