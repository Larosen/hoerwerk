//! Audiobook pipeline: watch the inbox, convert to M4B, tag from Audible, file into the library –
//! with a review queue for books that cannot be matched with confidence. A job runs in stages
//! (prepare → convert → finish), each one step in a queue of the job engine; see docs/JOBS.md.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::App;
use crate::audible;
use crate::db::compact;
use crate::encoder::{self, Chapter, EncodeError, Runner};
use crate::filing;
use crate::media::{self, probe_cached};
use crate::packs;
use crate::quality;
use crate::releases;
use crate::rx;
use crate::tagger;
use crate::text::{self, now, t};
use crate::words::{natural_key, natural_path_cmp};

const GRAB_SETTLE: i64 = 15;
const TEMP_SUFFIXES: [&str; 10] = [
    ".part",
    ".!qb",
    ".crdownload",
    ".tmp",
    ".partial",
    ".download",
    ".filepart",
    ".aria2",
    ".!ut",
    ".opdownload",
];
pub const ACTIVE: [&str; 4] = ["queued", "matching", "converting", "tagging"];
const FINAL: [&str; 3] = ["done", "failed", "cancelled"];
const SUSPICIOUS_EXTS: [&str; 26] = [
    ".exe", ".com", ".bat", ".cmd", ".scr", ".pif", ".msi", ".dll", ".ps1", ".vbs", ".vbe", ".js", ".jse", ".wsf",
    ".hta", ".lnk", ".jar", ".apk", ".sh", ".py", ".app", ".dmg", ".iso", ".reg", ".docm", ".xlsm",
];
const CANDIDATE_KEYS: [&str; 23] = [
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
    "match",
    "region",
    "url",
    "format_type",
    "publisher",
    "language",
    "genres",
    "description",
    "source",
    "isbn",
    "mismatch",
    "request_id",
    "request_user",
];
const EDITABLE: [&str; 11] = [
    "title",
    "subtitle",
    "authors",
    "narrators",
    "series",
    "series_position",
    "release_date",
    "publisher",
    "genres",
    "description",
    "language",
];
const SIDECARS: [&str; 5] = [
    "desc.txt",
    "reader.txt",
    "cover.jpg",
    "cover.png",
    tagger::METADATA_FILE,
];

fn stage_queue(stage: &str) -> &'static str {
    if stage == "convert" { "encode" } else { "io" }
}

fn stage_status(stage: &str) -> &'static str {
    match stage {
        "prepare" => "matching",
        "convert" => "converting",
        _ => "tagging",
    }
}

pub fn candidate_id(book: &Value) -> String {
    for k in ["id", "asin"] {
        match &book[k] {
            Value::String(s) if !s.is_empty() => return s.clone(),
            Value::Number(n) => return n.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn obj() -> Value {
    json!({})
}

fn default_stage() -> String {
    "prepare".into()
}

fn default_status() -> String {
    "queued".into()
}

/// What is saved of a job (the same JSON as the Python version writes into the table "jobs").
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Job {
    #[serde(default)]
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub rel: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub origin: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub duration: f64,
    #[serde(default = "obj")]
    pub hints: Value,
    #[serde(default)]
    pub candidates: Vec<Value>,
    #[serde(rename = "match", default)]
    pub matched: Option<Value>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub m4b: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub dest: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub created: f64,
    #[serde(default)]
    pub updated: f64,
    #[serde(default = "obj")]
    pub converted_info: Value,
    #[serde(default)]
    pub duplicate: Option<Value>,
    #[serde(default)]
    pub dup_decision: Option<String>,
    #[serde(default)]
    pub chapter_names: Option<Vec<String>>,
    #[serde(default)]
    pub rename_files: bool,
    #[serde(default = "default_stage")]
    pub stage: String,
    #[serde(default)]
    pub pack: Option<Value>,
    /// taken back for a correction: never filed automatically, a person picks the right book
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review_first: bool,
    #[serde(default)]
    pub filing: Option<Value>,
}

fn new_id() -> String {
    let mut b = [0u8; 5];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

fn base_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn rel_to(path: &str, root: &Path) -> String {
    Path::new(path)
        .strip_prefix(root)
        .map(|r| r.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

impl Job {
    fn new(source: &str, rel: &str, origin: &str) -> Self {
        let base = base_name(source);
        // a folder keeps its dots ("J.K. Rowling - …"); only a file loses its extension
        let name = if Path::new(source).is_dir() {
            base.clone()
        } else {
            Path::new(&base)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(base.clone())
        };
        Job {
            id: new_id(),
            source: source.into(),
            rel: if rel.is_empty() { base } else { rel.into() },
            name,
            origin: origin.into(),
            status: "queued".into(),
            message: t("Wartet …", &[]),
            progress: 0.0,
            files: Vec::new(),
            duration: 0.0,
            hints: json!({}),
            candidates: Vec::new(),
            matched: None,
            region: None,
            m4b: None,
            cover: None,
            dest: None,
            error: None,
            log: Vec::new(),
            created: now(),
            updated: now(),
            converted_info: json!({}),
            duplicate: None,
            dup_decision: None,
            chapter_names: None,
            rename_files: false,
            stage: "prepare".into(),
            pack: None,
            review_first: false,
            filing: None,
        }
    }

    pub fn say(&mut self, msg: &str, progress: Option<f64>, level: &str) {
        self.message = msg.to_string();
        let repeated = self.log.last().is_some_and(|l| l.ends_with(msg));
        if (progress.is_none_or(|p| p <= 0.0 || p >= 1.0) || level != "info") && !repeated {
            text::log(level, &self.name, msg);
        }
        if let Some(p) = progress {
            self.progress = p;
        }
        self.updated = now();
        if !repeated {
            let tm = text::local(now());
            self.log
                .push(format!("{:02}:{:02}:{:02} {msg}", tm.tm_hour, tm.tm_min, tm.tm_sec));
            let skip = self.log.len().saturating_sub(60);
            self.log.drain(..skip);
        }
    }

    pub fn info(&mut self, msg: &str) {
        self.say(msg, None, "info");
    }

    pub fn to_dict(&self, full: bool) -> Value {
        let mut d = serde_json::to_value(self).unwrap_or(json!({}));
        d["progress"] = json!((self.progress * 10000.0).round_ties_even() / 10000.0);
        if !full {
            if let Some(o) = d.as_object_mut() {
                o.remove("log");
            }
            d["candidates"] = Value::Array(
                self.candidates
                    .iter()
                    .map(|c| {
                        let mut m: Map<String, Value> = CANDIDATE_KEYS
                            .iter()
                            .map(|k| (k.to_string(), c.get(*k).cloned().unwrap_or(Value::Null)))
                            .collect();
                        m.insert("id".into(), json!(candidate_id(c)));
                        Value::Object(m)
                    })
                    .collect(),
            );
        }
        d
    }

    fn hint(&self, k: &str) -> String {
        match &self.hints[k] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

/// A job and what belongs to it at runtime only.
pub struct Slot {
    job: Mutex<Job>,
    pub cancel: Arc<AtomicBool>,
    runner: Mutex<Option<Arc<Runner>>>,
}

pub type JobRef = Arc<Slot>;

impl Slot {
    fn new(job: Job) -> JobRef {
        Arc::new(Slot {
            job: Mutex::new(job),
            cancel: Arc::new(AtomicBool::new(false)),
            runner: Mutex::new(None),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Job> {
        self.job.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn id(&self) -> String {
        self.lock().id.clone()
    }

    fn say(&self, msg: &str) {
        self.lock().info(msg);
    }
}

#[derive(Debug)]
pub enum PipeError {
    Cancelled,
    Failed(String),
    NotFound,
    Refused(String),
}

impl From<EncodeError> for PipeError {
    fn from(e: EncodeError) -> Self {
        match e {
            EncodeError::Cancelled => PipeError::Cancelled,
            EncodeError::Failed(m) => PipeError::Failed(m),
        }
    }
}

impl From<std::io::Error> for PipeError {
    fn from(e: std::io::Error) -> Self {
        PipeError::Failed(e.to_string())
    }
}

impl From<String> for PipeError {
    fn from(e: String) -> Self {
        PipeError::Failed(e)
    }
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Cancelled => write!(f, "{}", t("Abgebrochen", &[])),
            PipeError::Failed(m) | PipeError::Refused(m) => write!(f, "{m}"),
            PipeError::NotFound => write!(f, "{}", t("Hörbuch nicht gefunden", &[])),
        }
    }
}

type Res<T> = Result<T, PipeError>;

#[derive(Default)]
struct Watch {
    seen: HashMap<PathBuf, ((u64, u64, f64), f64)>,
    ready_at: HashMap<PathBuf, f64>,
    complete: HashSet<String>,
    force: bool,
}

pub struct Pipeline {
    jobs: Mutex<Vec<JobRef>>,
    stored: Mutex<HashMap<String, String>>,
    save_lock: Mutex<()>,
    file_lock: Mutex<()>,
    watch: Mutex<Watch>,
    wake: Condvar,
    pub last_scan: Mutex<Option<f64>>,
    pub scan_note: Mutex<String>,
    pub stopping: AtomicBool,
    pub inbox: PathBuf,
    pub library: PathBuf,
    pub work_dir: PathBuf,
}

fn real(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn visible(name: &str) -> bool {
    !(name.starts_with('.') || name.starts_with('_') || name.starts_with("@eaDir") || name.starts_with("#recycle"))
}

fn settle_secs(s: &Map<String, Value>) -> i64 {
    match s.get("ab_settle") {
        None | Some(Value::Null) => 120,
        Some(Value::String(x)) if x.is_empty() => 120,
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .or_else(|| v.as_str().and_then(|x| x.trim().parse().ok()))
            .unwrap_or(120),
    }
}

fn sval(s: &Map<String, Value>, k: &str, d: &str) -> String {
    match s.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => d.to_string(),
        Some(other) => other.to_string(),
    }
}

fn sbool(s: &Map<String, Value>, k: &str, d: bool) -> bool {
    s.get(k).map_or(d, |v| match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(x) => !x.is_empty(),
        _ => true,
    })
}

// -- inbox scanning ---------------------------------------------------------------------------------

/// (files, bytes, newest mtime) – or None while something is still downloading.
pub fn signature(path: &Path) -> Option<(u64, u64, f64)> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Some((0, 0, 0.0));
    }
    let (mut count, mut size, mut newest) = (0u64, 0u64, 0.0f64);
    let mut files: Vec<PathBuf> = Vec::new();
    if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        let mut stack = vec![path.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if visible(&name) {
                        stack.push(e.path());
                    }
                } else if !(ft.is_symlink() && e.path().is_dir()) {
                    files.push(e.path());
                }
            }
        }
    }
    for f in files {
        let n = f
            .file_name()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if TEMP_SUFFIXES.iter().any(|x| n.ends_with(x)) {
            return None;
        }
        let m = std::fs::metadata(&f).ok()?;
        count += 1;
        size += m.len();
        newest = newest.max(media::mtime(&f).unwrap_or(0.0));
    }
    Some((count, size, newest))
}

/// Subfolders that are parts of one book (CD1, CD2 / 01, 02) – not volumes of a series.
pub fn are_discs(names: &[String]) -> bool {
    if names.iter().any(|n| {
        rx::is_match(
            rx!(r"(?i)\b(?:b(?:a|ä|ae)nde?|folgen?|vol(?:ume)?s?|books?|buch|episoden?|fall)\.?\s*\d"),
            n,
        )
    }) {
        return false;
    }
    if names.iter().all(|n| {
        rx::is_match(
            rx!(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)[\s_.-]*\d+\b"),
            n,
        )
    }) {
        return true;
    }
    names
        .iter()
        .map(|n| rx::sub(rx!(r"\d+"), n, "#").trim().to_lowercase())
        .collect::<HashSet<_>>()
        .len()
        == 1
}

/// Split an inbox entry into books: a folder with audio (or only CD-like subfolders) is one book; a folder of
/// book folders becomes several.
pub fn book_units(path: &Path) -> Vec<PathBuf> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Vec::new();
    }
    if path.is_file() {
        return if media::is_audio(&base_name(&path.display().to_string())) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    let Ok(rd) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort_by_key(|e| natural_key(e));
    let direct = entries
        .iter()
        .any(|e| media::is_audio(e) && media::is_regular(&path.join(e)));
    let subs: Vec<String> = entries
        .iter()
        .filter(|e| visible(e) && media::is_real_dir(&path.join(e)) && !media::collect_audio(&path.join(e)).is_empty())
        .cloned()
        .collect();
    if direct || subs.is_empty() {
        return if direct || !subs.is_empty() {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    if are_discs(&subs) {
        return vec![path.to_path_buf()];
    }
    subs.iter().flat_map(|s| book_units(&path.join(s))).collect()
}

/// Executables, scripts and links in a download – a sign of a fake or malicious release.
pub fn suspicious_files(source: &Path) -> Vec<String> {
    if source.is_file() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut stack = vec![source.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let p = e.path();
            let link = std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
            if link || SUSPICIOUS_EXTS.iter().any(|x| name.to_lowercase().ends_with(x)) {
                let rel = rel_to(&p.display().to_string(), source);
                out.push(if link {
                    format!("{rel} ({})", t("Verknüpfung", &[]))
                } else {
                    rel
                });
            }
            if !link && p.is_dir() {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

fn safe_track_name(name: &str) -> String {
    let n = rx::sub(rx!(r#"[\\/:*?"<>|\x00-\x1f]"#), name, "-");
    let n = rx::sub(rx!(r"\s+"), n.trim_matches([' ', '.']), " ");
    let n: String = n.chars().take(150).collect();
    if n.is_empty() { "Track".into() } else { n }
}

/// `03 - Name.mp3` for every track (in its own subfolder); returns the new relative paths.
/// The files are merged in the order of their names (CD folders one after the other). When every file has a
/// track number in its tags and, within one folder, these do not count up in that order, something is off
/// (badly named files): which file – else None. Without track numbers nothing can be said.
pub fn order_doubt(files: &[String], infos: &[Value]) -> Option<String> {
    let number = |v: &Value| -> Option<u64> {
        let text = match v {
            Value::String(x) => x.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        text.split('/').next()?.trim().parse().ok()
    };
    let tracks: Vec<Option<u64>> = infos.iter().map(|i| number(&i["tags"]["track"])).collect();
    if files.len() < 2 || tracks.iter().any(Option::is_none) {
        return None;
    }
    let disc = |i: usize| number(&infos[i]["tags"]["disc"]).unwrap_or(0);
    let folder = |f: &str| f.rsplit_once('/').map_or("", |x| x.0).to_string();
    for i in 1..files.len() {
        if folder(&files[i]) != folder(&files[i - 1]) {
            continue; // a new CD folder starts counting again
        }
        if (disc(i), tracks[i]) <= (disc(i - 1), tracks[i - 1]) {
            return Some(t(
                "Track-Nummer in „{file}“ passt nicht zur Reihenfolge der Dateinamen",
                &[("file", &files[i])],
            ));
        }
    }
    None
}

pub fn rename_tracks(folder: &Path, files: &[String], names: &[String]) -> std::io::Result<Vec<String>> {
    let width = files.len().to_string().len().max(2);
    let plan: Vec<(String, String)> = files
        .iter()
        .zip(names)
        .enumerate()
        .map(|(i, (rel, name))| {
            let ext = Path::new(rel)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                .unwrap_or_default();
            let dir = Path::new(rel)
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let file = format!("{:0width$} - {}{ext}", i + 1, safe_track_name(name));
            (rel.clone(), if dir.is_empty() { file } else { format!("{dir}/{file}") })
        })
        .collect();
    // two steps, so swapping names between tracks can't overwrite anything
    let mut temp = Vec::new();
    for (i, (rel, new)) in plan.iter().enumerate() {
        if rel == new {
            temp.push(rel.clone());
            continue;
        }
        let ext = Path::new(rel)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let dir = Path::new(rel)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let tmp_name = format!(".rn-{i}-{}{ext}", std::process::id());
        let tmp = if dir.is_empty() {
            tmp_name
        } else {
            format!("{dir}/{tmp_name}")
        };
        std::fs::rename(folder.join(rel), folder.join(&tmp))?;
        temp.push(tmp);
    }
    let mut out = Vec::new();
    for (tmp, (_, new)) in temp.iter().zip(&plan) {
        let mut new = new.clone();
        if *tmp != new {
            let target = folder.join(&new);
            if target.exists() {
                new = rel_to(&filing::free_path(&target).display().to_string(), folder);
            }
            std::fs::rename(folder.join(tmp), folder.join(&new))?;
        }
        out.push(new);
    }
    Ok(out)
}

fn remove_empty_parents(path: &Path, stop: &Path) {
    let stop = real(stop);
    let mut path = real(path);
    while path.starts_with(&stop) && path != stop {
        let Ok(rd) = std::fs::read_dir(&path) else { return };
        let leftovers: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|f| !f.starts_with('.'))
            .collect();
        if !leftovers.is_empty() || std::fs::remove_dir(&path).is_err() {
            return;
        }
        match path.parent() {
            Some(p) => path = p.to_path_buf(),
            None => return,
        }
    }
}

fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // another disk: copy, then remove
    copy_tree(from, to)?;
    if from.is_dir() {
        std::fs::remove_dir_all(from)
    } else {
        std::fs::remove_file(from)
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn book_from_hints(job: &Job) -> Value {
    let title = [job.hint("title"), job.hint("folder"), job.name.clone()]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    json!({"title": title,
           "authors": if job.hint("author").is_empty() { json!([]) } else { json!([job.hint("author")]) },
           "narrators": if job.hint("narrator").is_empty() { json!([]) } else { json!([job.hint("narrator")]) }})
}

fn strip_match(book: &Value) -> Value {
    let mut b = book.clone();
    if let Some(o) = b.as_object_mut() {
        for k in ["match", "request_id", "request_user"] {
            o.remove(k);
        }
    }
    b
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

impl Pipeline {
    pub fn new(inbox: &Path, library: &Path, work_dir: &Path) -> Self {
        Pipeline {
            jobs: Mutex::new(Vec::new()),
            stored: Mutex::new(HashMap::new()),
            save_lock: Mutex::new(()),
            file_lock: Mutex::new(()),
            watch: Mutex::new(Watch::default()),
            wake: Condvar::new(),
            last_scan: Mutex::new(None),
            scan_note: Mutex::new(String::new()),
            stopping: AtomicBool::new(false),
            inbox: if inbox.as_os_str().is_empty() {
                PathBuf::new()
            } else {
                real(inbox)
            },
            library: if library.as_os_str().is_empty() {
                PathBuf::new()
            } else {
                real(library)
            },
            work_dir: work_dir.to_path_buf(),
        }
    }

    fn list(&self) -> MutexGuard<'_, Vec<JobRef>> {
        self.jobs.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn all(&self) -> Vec<JobRef> {
        self.list().clone()
    }

    pub fn note(&self, text: String) {
        *self.scan_note.lock().unwrap_or_else(|p| p.into_inner()) = text;
    }

    // -- persistence (table "jobs") -----------------------------------------------------------------
    pub fn load(&self, app: &Arc<App>) {
        let rows: Vec<String> = {
            let c = app.db.lock();
            let Ok(mut st) = c.prepare("SELECT data FROM jobs WHERE kind='audiobook' ORDER BY created") else {
                return;
            };
            st.query_map([], |r| r.get::<_, String>(0))
                .map(|it| it.flatten().collect())
                .unwrap_or_default()
        };
        for row in rows {
            let Ok(raw) = serde_json::from_str::<Value>(&row) else {
                continue;
            };
            if s(&raw, "source").is_empty() {
                continue;
            }
            let Ok(mut job) = serde_json::from_value::<Job>(raw.clone()) else {
                continue;
            };
            if job.id.is_empty() {
                job.id = new_id();
            }
            if !matches!(job.stage.as_str(), "prepare" | "convert" | "finish") {
                job.stage = "prepare".into();
            }
            if raw.get("stage").is_none() {
                // written by version 3: derive the stage
                job.stage = if job.m4b.as_deref().is_some_and(|m| Path::new(m).exists()) {
                    "finish".into()
                } else {
                    "prepare".into()
                };
            }
            let slot = Slot::new(job);
            if slot.lock().filing.is_some() {
                self.recover_filing(app, &slot);
            }
            {
                let mut j = slot.lock();
                if ACTIVE.contains(&j.status.as_str()) {
                    j.status = "queued".into();
                    j.info(&t("Nach Neustart erneut eingereiht", &[]));
                }
            }
            self.list().push(slot);
        }
        self.save(app);
    }

    /// Write the jobs that changed since the last save (one transaction).
    pub fn save(&self, app: &App) {
        {
            let _g = self.save_lock.lock().unwrap_or_else(|p| p.into_inner());
            let current: Vec<(String, String, String, String, f64, f64)> = self
                .all()
                .iter()
                .map(|slot| {
                    let j = slot.lock();
                    (
                        j.id.clone(),
                        compact(&j.to_dict(true)),
                        j.status.clone(),
                        j.source.clone(),
                        j.created,
                        j.updated,
                    )
                })
                .collect();
            let mut stored = self.stored.lock().unwrap_or_else(|p| p.into_inner());
            let ids: HashSet<&String> = current.iter().map(|c| &c.0).collect();
            let gone: Vec<String> = stored.keys().filter(|k| !ids.contains(k)).cloned().collect();
            let mut c = app.db.lock();
            if let Ok(tx) = c.transaction() {
                for id in &gone {
                    let _ = tx.execute("DELETE FROM jobs WHERE id=?", [id]);
                }
                let mut written = Vec::new();
                for (id, text, status, source, created, updated) in &current {
                    if stored.get(id) == Some(text) {
                        continue;
                    }
                    let _ = tx.execute(
                        "INSERT OR REPLACE INTO jobs(id, kind, status, source, created, updated, data) VALUES(?,?,?,?,?,?,?)",
                        rusqlite::params![id, "audiobook", status, source, created, updated, text],
                    );
                    written.push((id.clone(), text.clone()));
                }
                if tx.commit().is_ok() {
                    for id in gone {
                        stored.remove(&id);
                    }
                    stored.extend(written);
                }
            }
        }
        app.publish("audiobooks");
    }

    // -- public API --------------------------------------------------------------------------------
    pub fn status(&self, app: &App) -> Value {
        let s = app.settings();
        let mut jobs: Vec<Value> = self.all().iter().map(|slot| slot.lock().to_dict(false)).collect();
        jobs.sort_by(|a, b| {
            b["created"]
                .as_f64()
                .unwrap_or(0.0)
                .partial_cmp(&a["created"].as_f64().unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let watch = self.watch.lock().unwrap_or_else(|p| p.into_inner());
        let waiting: Vec<Value> = watch
            .seen
            .iter()
            .filter(|(p, _)| !self.known(app, p))
            .map(|(p, v)| {
                let ready = watch.ready_at.get(p).copied().unwrap_or(0.0);
                json!({"name": rel_to(&p.display().to_string(), &self.inbox), "since": v.1, "ready_in": (ready - now()).round().max(0.0) as i64})
            })
            .collect();
        drop(watch);
        let mut ignored: Vec<String> = self.ignored(app).keys().cloned().collect();
        ignored.sort();
        let review = jobs.iter().filter(|j| j["status"] == "review").count();
        json!({
            "enabled": sbool(&s, "ab_watch", false) && !self.inbox.as_os_str().is_empty(),
            "inbox": self.inbox, "library": self.library,
            "inbox_ok": !self.inbox.as_os_str().is_empty() && self.inbox.is_dir(),
            "library_ok": !self.library.as_os_str().is_empty() && self.library.is_dir(),
            "last_scan": *self.last_scan.lock().unwrap_or_else(|p| p.into_inner()),
            "scan_note": *self.scan_note.lock().unwrap_or_else(|p| p.into_inner()),
            "waiting": waiting, "jobs": jobs, "review": review, "ignored": ignored,
        })
    }

    pub fn get(&self, id: &str) -> Option<JobRef> {
        self.all().into_iter().find(|s| s.lock().id == id)
    }

    fn need(&self, id: &str) -> Res<JobRef> {
        self.get(id).ok_or(PipeError::NotFound)
    }

    /// origin: inbox (watched folder), manual (originals untouched) or library (an older book in the library
    /// that gets converted and replaces itself).
    pub fn add(
        &self,
        app: &Arc<App>,
        source: &Path,
        origin: &str,
        chapter_names: Option<Vec<String>>,
        rename: bool,
        pack: Option<Value>,
    ) -> JobRef {
        self.add_with(app, source, origin, chapter_names, rename, pack, false)
    }

    /// A book of the library back for a correction – whatever happened to the job that filed it: it is read
    /// and looked up like a new book, then waits for a person to pick the right one; the old files step aside
    /// only when the corrected book is filed.
    pub fn correct(&self, app: &Arc<App>, folder: &Path) -> JobRef {
        self.add_with(app, folder, "library", None, false, None, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_with(
        &self,
        app: &Arc<App>,
        source: &Path,
        origin: &str,
        chapter_names: Option<Vec<String>>,
        rename: bool,
        pack: Option<Value>,
        review_first: bool,
    ) -> JobRef {
        let source = real(source).display().to_string();
        let slot = {
            let mut list = self.list();
            if let Some(j) = list.iter().find(|j| {
                let g = j.lock();
                g.source == source && !FINAL.contains(&g.status.as_str())
            }) {
                return j.clone();
            }
            let rel = match origin {
                "inbox" => rel_to(&source, &self.inbox),
                "library" => rel_to(&source, &self.library),
                _ => base_name(&source),
            };
            let mut job = Job::new(&source, &rel, origin);
            job.chapter_names = chapter_names;
            job.rename_files = rename;
            job.pack = pack;
            job.review_first = review_first;
            let slot = Slot::new(job);
            list.push(slot.clone());
            // only the newest 200 finished jobs stay
            let done: Vec<JobRef> = list.iter().filter(|j| j.lock().status == "done").cloned().collect();
            let drop_n = done.len().saturating_sub(200);
            for d in done.into_iter().take(drop_n) {
                list.retain(|x| !Arc::ptr_eq(x, &d));
            }
            slot
        };
        self.save(app);
        self.schedule(app, &slot);
        slot
    }

    /// Entries that were moved into the inbox complete (an upload): take them at once.
    pub fn arrived(&self, names: &[String]) {
        self.watch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .complete
            .extend(names.iter().cloned());
        self.scan_now();
    }

    pub fn scan_now(&self) {
        self.watch.lock().unwrap_or_else(|p| p.into_inner()).force = true;
        self.wake.notify_all();
    }

    fn requeue_if_waiting(&self, app: &Arc<App>, slot: &JobRef) {
        let requeue = {
            let mut j = slot.lock();
            let r = matches!(j.status.as_str(), "review" | "failed");
            if r {
                j.status = "queued".into();
                j.error = None;
                j.stage = if j.m4b.as_deref().is_some_and(|m| Path::new(m).exists()) {
                    "finish".into()
                } else {
                    "prepare".into()
                };
            }
            r
        };
        self.save(app);
        if requeue {
            self.schedule(app, slot);
        }
    }

    pub fn schedule(&self, app: &Arc<App>, slot: &JobRef) {
        let (status, stage, id) = {
            let j = slot.lock();
            (j.status.clone(), j.stage.clone(), j.id.clone())
        };
        if status == "queued" {
            let (a, s) = (app.clone(), slot.clone());
            app.engine
                .submit(stage_queue(&stage), &id, Box::new(move || a.pipeline.step(&a, &s)));
        }
    }

    fn job_work(&self, id: &str) -> PathBuf {
        self.work_dir.join(id)
    }

    fn cleanup_work(&self, id: &str) {
        let _ = std::fs::remove_dir_all(self.job_work(id));
    }

    pub fn search(&self, app: &Arc<App>, id: &str, query: &str, region: &str) -> Res<JobRef> {
        if crate::links::is_link(query) {
            return Ok(self.add_link(app, id, query)?.0);
        }
        let slot = self.need(id)?;
        let mut hints = slot.lock().hints.clone();
        let cq = audible::clean_query(query);
        hints["query"] = json!(if cq.is_empty() { query.to_string() } else { cq });
        hints["title"] = json!("");
        hints["author"] = json!("");
        hints["asin"] = json!(audible::find_asin(&[query]));
        let found = app
            .providers
            .search(&hints, region, sbool(&app.settings(), "ab_fallback", true))
            .map_err(|e| PipeError::Refused(e.0))?;
        let cands = self.demote_mismatches(&slot, found);
        {
            let mut j = slot.lock();
            j.candidates = cands.into_iter().take(12).collect();
            j.region = Some(region.into());
            let n = j.candidates.len().to_string();
            j.info(&t(
                "{n} Treffer für „{query}“ ({region})",
                &[("n", &n), ("query", query), ("region", region)],
            ));
        }
        self.save(app);
        Ok(slot)
    }

    /// Book data from an Audible/BookBeat/Thalia link becomes the top candidate. Blocking.
    pub fn add_link(&self, app: &Arc<App>, id: &str, url: &str) -> Res<(JobRef, Value)> {
        let slot = self.need(id)?;
        let book = crate::links::fetch(url, &app.providers.client).map_err(|e| PipeError::Refused(e.0))?;
        let hints = slot.lock().hints.clone();
        let book = audible::rank(&[book], &hints).into_iter().next().unwrap_or(Value::Null);
        let cid = candidate_id(&book);
        {
            let mut j = slot.lock();
            let rest: Vec<Value> = j
                .candidates
                .iter()
                .filter(|c| candidate_id(c) != cid)
                .cloned()
                .collect();
            j.candidates = std::iter::once(book.clone()).chain(rest).collect();
            j.info(&t(
                "Daten aus Link: {title} ({source})",
                &[("title", s(&book, "title")), ("source", s(&book, "source"))],
            ));
        }
        self.save(app);
        Ok((slot, book))
    }

    pub fn choose(&self, app: &Arc<App>, id: &str, cand_id: &str, asin: &str, region: &str) -> Res<JobRef> {
        let slot = self.need(id)?;
        let mut book = if cand_id.is_empty() {
            None
        } else {
            slot.lock()
                .candidates
                .iter()
                .find(|c| candidate_id(c) == cand_id)
                .cloned()
        };
        if book.is_none() && !asin.is_empty() {
            book = slot
                .lock()
                .candidates
                .iter()
                .find(|c| s(c, "asin") == asin && s(c, "region") == region)
                .cloned();
            if book.is_none() {
                book = Some(
                    app.providers
                        .client
                        .product(asin, region)
                        .map_err(|e| PipeError::Refused(e.0))?,
                );
            }
        }
        let Some(book) = book else {
            return Err(PipeError::NotFound);
        };
        {
            let mut j = slot.lock();
            j.matched = Some(strip_match(&book));
            j.duplicate = None;
            j.dup_decision = None;
            let source = if s(&book, "source").is_empty() {
                "audible"
            } else {
                s(&book, "source")
            };
            j.info(&t(
                "Ausgewählt: {title} ({source})",
                &[("title", s(&book, "title")), ("source", source)],
            ));
        }
        self.link_request(app, &slot, &book);
        self.requeue_if_waiting(app, &slot);
        Ok(slot)
    }

    /// The user's own data – on top of a candidate they edited, or of the file tags.
    pub fn manual(&self, app: &Arc<App>, id: &str, data: &Value, base_id: &str) -> Res<JobRef> {
        let slot = self.need(id)?;
        let mut d: Map<String, Value> = EDITABLE
            .iter()
            .filter_map(|k| data.get(*k).map(|v| (k.to_string(), v.clone())))
            .collect();
        for k in ["authors", "narrators", "genres"] {
            if let Some(Value::String(x)) = d.get(k).cloned() {
                d.insert(
                    k.into(),
                    json!(
                        x.split(',')
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        let (base, m4b, hints, name) = {
            let j = slot.lock();
            let base = if base_id.is_empty() {
                None
            } else {
                j.candidates.iter().find(|c| candidate_id(c) == base_id).cloned()
            };
            (base, j.m4b.clone(), j.hints.clone(), j.name.clone())
        };
        let mut book = match &base {
            Some(b) => {
                // the form was prefilled with the candidate: what the user left there is what they want,
                // including fields they emptied (e.g. a wrong series)
                let mut book = b.clone();
                if let Some(o) = book.as_object_mut() {
                    o.remove("match");
                }
                for (k, v) in &d {
                    let list = matches!(k.as_str(), "authors" | "narrators" | "genres");
                    let empty = matches!(v, Value::Null | Value::Bool(false))
                        || v == &json!("")
                        || v == &json!([])
                        || v == &json!(0);
                    book[k] = if !list && empty { Value::Null } else { v.clone() };
                }
                book["source"] = json!(if s(b, "source").is_empty() {
                    "audible"
                } else {
                    s(b, "source")
                });
                book["edited"] = json!(true);
                book
            }
            None => {
                let mut book = match m4b.filter(|m| Path::new(m).exists()) {
                    Some(m) => tagger::read_book(Path::new(&m)),
                    None => json!({}),
                };
                for (k, v) in &d {
                    if !(v.is_null() || v == &json!("") || v == &json!([])) {
                        book[k] = v.clone();
                    }
                }
                book["source"] = json!("manual");
                book
            }
        };
        if s(&book, "title").is_empty() {
            let title = hints["title"]
                .as_str()
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .unwrap_or(name);
            book["title"] = json!(title);
        }
        if book["authors"].as_array().is_none_or(|a| a.is_empty()) && !s(&hints, "author").is_empty() {
            book["authors"] = json!([s(&hints, "author")]);
        }
        {
            let mut j = slot.lock();
            j.matched = Some(book);
            j.duplicate = None;
            j.dup_decision = None;
            let msg = match &base {
                Some(b) => t("Eigene Angaben (auf Basis von {source})", &[("source", s(b, "source"))]),
                None => t("Eigene Angaben", &[]),
            };
            j.info(&msg);
        }
        self.requeue_if_waiting(app, &slot);
        Ok(slot)
    }

    /// "Wrong edition": the filed book is another recording than the one asked for. Its request (if any)
    /// blocks this release and searches on; the book comes back for a correction under its real name.
    pub fn wrong_edition(&self, app: &Arc<App>, id: &str) -> Res<JobRef> {
        let slot = self.need(id)?;
        let (rid, release) = {
            let mut j = slot.lock();
            if j.status != "done" {
                return Err(PipeError::Refused(t(
                    "Nur abgelegte Bücher lassen sich korrigieren",
                    &[],
                )));
            }
            let rid = j.hints["request_id"].as_i64();
            // what the request said must not decide again: the book is looked up by its content
            if let Some(h) = j.hints.as_object_mut() {
                h.remove("request_id");
                h.insert("asin".into(), Value::Null);
            }
            (rid, base_name(&j.source))
        };
        let back = self.reopen(app, id)?;
        let why = t("Falsche Fassung (von Hand markiert)", &[]);
        back.say(&why);
        if let Some(rid) = rid {
            app.requests.wrong_edition(app, rid, &release, &why);
            back.say(&t("Anfrage sucht weiter, dieses Release ist gesperrt", &[]));
        }
        Ok(back)
    }

    /// Take a filed book back out of the library to match it again.
    pub fn reopen(&self, app: &Arc<App>, id: &str) -> Res<JobRef> {
        let slot = self.need(id)?;
        {
            let mut j = slot.lock();
            if j.status != "done" {
                return Err(PipeError::Refused(t(
                    "Nur abgelegte Bücher lassen sich korrigieren",
                    &[],
                )));
            }
            let dest = j.dest.clone().filter(|d| Path::new(d).is_file());
            let Some(dest) = dest else {
                // moved or replaced since: the book is looked for in the library instead
                let asin = j
                    .matched
                    .as_ref()
                    .map(|m| s(m, "asin").to_uppercase())
                    .unwrap_or_default();
                let was = j.dest.clone().unwrap_or_default();
                drop(j);
                let found = (!asin.is_empty())
                    .then(|| {
                        app.library
                            .entries()
                            .into_iter()
                            .find(|e| s(e, "asin").to_uppercase() == asin)
                    })
                    .flatten()
                    .map(|e| self.library.join(s(&e, "path")))
                    .filter(|p| p.is_dir());
                return match found {
                    Some(folder) => Ok(self.correct(app, &folder)),
                    None => Err(PipeError::Refused(t(
                        "Die Datei liegt nicht mehr unter {path}, und in der Bibliothek ist das Buch nicht zu finden. Korrigieren geht auch direkt in der Bibliothek beim Buch.",
                        &[("path", &was)],
                    ))),
                };
            };
            let work = self.job_work(&j.id);
            std::fs::create_dir_all(&work)?;
            let folder = Path::new(&dest).parent().unwrap_or(Path::new("")).to_path_buf();
            let m4b = work.join("book.m4b");
            move_path(Path::new(&dest), &m4b)?;
            for name in SIDECARS {
                let p = folder.join(name);
                if p.is_file() {
                    let _ = std::fs::remove_file(p);
                }
            }
            remove_empty_parents(&folder, &self.library);
            app.library.remove(Path::new(&dest));
            j.m4b = Some(m4b.display().to_string());
            j.dest = None;
            j.matched = None;
            j.duplicate = None;
            j.dup_decision = None;
            j.status = "review".into();
            j.stage = "finish".into();
            j.info(&t("Zur Korrektur aus der Bibliothek geholt", &[]));
        }
        self.save(app);
        Ok(slot)
    }

    /// {kbps, duration, size, files} of a book file or folder – what one hears, averaged over all its audio.
    pub fn quality_of(path: &Path) -> Value {
        if !path.exists() {
            return Value::Null;
        }
        let files: Vec<PathBuf> = if path.is_dir() {
            media::collect_audio(path).into_iter().map(|f| path.join(f)).collect()
        } else {
            vec![path.to_path_buf()]
        };
        let (mut size, mut duration) = (0.0f64, 0.0f64);
        for f in files.iter().take(500) {
            let Ok(m) = std::fs::metadata(f) else { continue };
            size += m.len() as f64;
            duration += probe_cached(f)["duration"].as_f64().unwrap_or(0.0);
        }
        if files.is_empty() || duration == 0.0 {
            return Value::Null;
        }
        json!({"kbps": releases::py_round(size * 8.0 / 1000.0 / duration), "duration": releases::py_round(duration), "size": size as i64, "files": files.len()})
    }

    /// The book is already in the library: keep both, replace the old one, or drop the new one.
    pub fn resolve_duplicate(&self, app: &Arc<App>, id: &str, action: &str) -> Res<JobRef> {
        let slot = self.need(id)?;
        if !matches!(action, "keep" | "replace" | "discard") {
            return Err(PipeError::Refused(t("Unbekannte Aktion", &[])));
        }
        let dup =
            slot.lock().duplicate.clone().ok_or_else(|| {
                PipeError::Refused(t("Für dieses Buch gibt es keinen Treffer in der Bibliothek", &[]))
            })?;
        if action == "discard" {
            self.dispose_source(&slot, &app.settings());
            {
                let mut j = slot.lock();
                self.cleanup_work(&j.id);
                j.m4b = None;
                j.status = "cancelled".into();
                j.info(&t(
                    "Verworfen – schon in der Bibliothek: {path}",
                    &[("path", s(&dup, "path"))],
                ));
            }
            self.save(app);
            return Ok(slot);
        }
        {
            let mut j = slot.lock();
            j.dup_decision = Some(action.into());
            let msg = if action == "keep" {
                t("Trotzdem ablegen (zusätzlich)", &[])
            } else {
                t("Ersetzt {path}", &[("path", s(&dup, "path"))])
            };
            j.info(&msg);
        }
        self.requeue_if_waiting(app, &slot);
        Ok(slot)
    }

    /// Back into the queue; the answer is the job as queued (a worker may pick it up at once).
    pub fn retry(&self, app: &Arc<App>, id: &str) -> Res<Value> {
        let slot = self.need(id)?;
        let queued = {
            let mut j = slot.lock();
            if !matches!(j.status.as_str(), "failed" | "cancelled") {
                return Ok(j.to_dict(false));
            }
            let failed = j.dest.clone().filter(|d| j.origin == "inbox" && Path::new(d).exists());
            if let Some(failed) = failed {
                let back = self.inbox.join(&j.rel);
                if !back.exists() {
                    move_path(Path::new(&failed), &back)?;
                    remove_empty_parents(Path::new(&failed).parent().unwrap_or(Path::new("")), &self.inbox);
                    let note = back.join("FEHLER.txt");
                    if note.is_file() {
                        let _ = std::fs::remove_file(note);
                    }
                }
                j.source = back.display().to_string();
                j.dest = None;
            }
            j.status = "queued".into();
            j.error = None;
            j.progress = 0.0;
            j.stage = "prepare".into();
            j.m4b = None;
            j.info(&t("Erneut eingereiht", &[]));
            j.to_dict(false)
        };
        self.save(app);
        self.schedule(app, &slot);
        Ok(queued)
    }

    /// Hörwerk stops (docker stop): running ffmpeg ends now, its job goes back to the queue (not cancelled).
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for slot in self.all() {
            if let Some(r) = slot.runner.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                r.kill();
            }
        }
        self.wake.notify_all();
    }

    pub fn cancel(&self, app: &Arc<App>, id: &str) -> Res<JobRef> {
        let slot = self.need(id)?;
        slot.cancel.store(true, Ordering::SeqCst);
        if let Some(r) = slot.runner.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            r.kill();
        }
        {
            let mut j = slot.lock();
            if matches!(j.status.as_str(), "queued" | "review") {
                j.status = "cancelled".into();
                j.info(&t("Abgebrochen", &[]));
                self.cleanup_work(&j.id);
            }
        }
        app.engine.discard(id);
        self.save(app);
        Ok(slot)
    }

    pub fn remove(&self, app: &Arc<App>, id: &str) -> Res<()> {
        let slot = self.need(id)?;
        if ACTIVE.contains(&slot.lock().status.as_str()) {
            return Err(PipeError::Refused(t("Läuft noch – erst abbrechen", &[])));
        }
        self.cleanup_work(id);
        self.list().retain(|x| !Arc::ptr_eq(x, &slot));
        app.engine.discard(id);
        self.save(app);
        Ok(())
    }

    /// The top-level inbox entry this job came from – only that may be deleted or ignored.
    fn inbox_entry(&self, job: &Job) -> Res<PathBuf> {
        if job.origin != "inbox" || self.inbox.as_os_str().is_empty() {
            return Err(PipeError::Refused(t("Nur für Einträge aus dem Eingang", &[])));
        }
        let src = Path::new(&job.source);
        let r = real(src);
        if r.parent() != Some(self.inbox.as_path())
            || std::fs::symlink_metadata(src).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(PipeError::Refused(t(
                "Teil eines größeren Downloads – bitte den ganzen Ordner im Eingang löschen",
                &[],
            )));
        }
        Ok(r)
    }

    /// Delete the download itself (folder or file) from the inbox – and its entry. Returns the name.
    pub fn delete_from_inbox(&self, app: &Arc<App>, id: &str) -> Res<String> {
        let slot = self.need(id)?;
        let path = {
            let j = slot.lock();
            if ACTIVE.contains(&j.status.as_str()) {
                return Err(PipeError::Refused(t("Läuft noch – erst abbrechen", &[])));
            }
            self.inbox_entry(&j)?
        };
        let gone: Vec<JobRef> = self
            .all()
            .into_iter()
            .filter(|x| {
                let j = x.lock();
                j.origin == "inbox" && real(Path::new(&j.source)) == path
            })
            .collect();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else if path.exists() {
            std::fs::remove_file(&path)?;
        }
        for g in &gone {
            let gid = g.id();
            self.cleanup_work(&gid);
            self.list().retain(|x| !Arc::ptr_eq(x, g));
            app.engine.discard(&gid);
        }
        self.watch.lock().unwrap_or_else(|p| p.into_inner()).seen.remove(&path);
        self.save(app);
        let name = base_name(&path.display().to_string());
        text::info(&name, &t("Aus dem Eingang gelöscht", &[]));
        Ok(name)
    }

    pub fn ignored(&self, app: &App) -> Map<String, Value> {
        app.db
            .get("inbox_ignored")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    /// Leave the download where it is, but never pick it up again (until un-ignored).
    pub fn ignore(&self, app: &Arc<App>, id: &str) -> Res<String> {
        let slot = self.need(id)?;
        let name = {
            let j = slot.lock();
            if ACTIVE.contains(&j.status.as_str()) {
                return Err(PipeError::Refused(t("Läuft noch – erst abbrechen", &[])));
            }
            base_name(&self.inbox_entry(&j)?.display().to_string())
        };
        let mut ig = self.ignored(app);
        ig.insert(name.clone(), json!(now()));
        let _ = app.db.put("inbox_ignored", &Value::Object(ig));
        self.remove(app, id)?;
        text::info(&name, &t("Wird im Eingang ignoriert", &[]));
        Ok(name)
    }

    /// Pick an ignored download up again (or all of them).
    pub fn unignore(&self, app: &App, name: &str) -> usize {
        let ig = self.ignored(app);
        let keep: Map<String, Value> = if name.is_empty() {
            Map::new()
        } else {
            ig.iter()
                .filter(|(k, _)| *k != name)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        let _ = app.db.put("inbox_ignored", &Value::Object(keep.clone()));
        self.scan_now();
        ig.len() - keep.len()
    }

    pub fn clear_finished(&self, app: &App) {
        self.list().retain(|j| j.lock().status != "done");
        self.save(app);
    }

    /// Inbox entries that already have a job (cancelled ones stay known until removed) – or are ignored.
    fn known(&self, app: &App, path: &Path) -> bool {
        if path.parent() == Some(self.inbox.as_path())
            && self.ignored(app).contains_key(&base_name(&path.display().to_string()))
        {
            return true;
        }
        let p = path.display().to_string();
        self.all().iter().any(|x| {
            let j = x.lock();
            j.status != "done" && j.origin == "inbox" && (j.source == p || j.source.starts_with(&format!("{p}/")))
        })
    }

    // -- the inbox watcher -----------------------------------------------------------------------
    pub fn scanner(&self, app: &Arc<App>) {
        while !self.stopping.load(Ordering::SeqCst) {
            let s = app.settings();
            let interval = s
                .get("ab_scan_interval")
                .and_then(|v| v.as_i64())
                .filter(|i| *i != 0)
                .unwrap_or(60)
                .max(10) as f64;
            if sbool(&s, "ab_watch", false) && !self.inbox.as_os_str().is_empty() {
                self.scan_once(app, settle_secs(&s));
            }
            // wake up when the next waiting download counts as complete, not a whole interval later
            let mut watch = self.watch.lock().unwrap_or_else(|p| p.into_inner());
            let due = watch
                .ready_at
                .values()
                .map(|at| (at - now() + 1.0).max(1.0))
                .fold(interval, f64::min);
            if !watch.force {
                watch = self
                    .wake
                    .wait_timeout(watch, Duration::from_secs_f64(due))
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
            watch.force = false;
        }
    }

    /// One pass over the inbox; queues every entry that has been quiet for `settle` seconds.
    pub fn scan_once(&self, app: &Arc<App>, settle: i64) -> Vec<JobRef> {
        *self.last_scan.lock().unwrap_or_else(|p| p.into_inner()) = Some(now());
        if !self.inbox.is_dir() {
            self.note(t("Eingangsordner nicht gefunden", &[]));
            return Vec::new();
        }
        let mut names: Vec<String> = match std::fs::read_dir(&self.inbox) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(e) => {
                // e.g. no read permission (PUID/PGID): shown on the page instead of an empty inbox
                self.note(t("Fehler beim Durchsuchen: {error}", &[("error", &e.to_string())]));
                return Vec::new();
            }
        };
        names.sort_by_key(|n| natural_key(n));
        let mut added = Vec::new();
        let mut present: HashSet<PathBuf> = HashSet::new();
        for name in names {
            if !visible(&name) {
                continue;
            }
            let path = self.inbox.join(&name);
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                continue; // never follow links out of the inbox
            }
            present.insert(path.clone());
            if self.known(app, &path) {
                continue;
            }
            let Some(sig) = signature(&path) else {
                self.watch.lock().unwrap_or_else(|p| p.into_inner()).seen.remove(&path); // still downloading
                continue;
            };
            if sig.0 == 0 {
                continue;
            }
            let grab = settle != 0 && app.usenet.hint_for(&name).is_some();
            let mut wait = if grab { settle.min(GRAB_SETTLE) } else { settle };
            let mut watch = self.watch.lock().unwrap_or_else(|p| p.into_inner());
            if watch.complete.contains(&name) {
                wait = 0;
            }
            let prev = watch.seen.get(&path).cloned();
            if prev.is_none() {
                let msg = if wait != 0 {
                    t(
                        "Neu im Eingang – wird verarbeitet, sobald {n} s lang nichts mehr geschrieben wurde",
                        &[("n", &wait.to_string())],
                    )
                } else {
                    t("Neu im Eingang – wird jetzt verarbeitet", &[])
                };
                text::info(&name, &msg);
            }
            if prev.is_none_or(|p| p.0 != sig) {
                watch.seen.insert(path.clone(), (sig, now()));
                watch.ready_at.insert(path.clone(), now() + wait as f64);
                if wait > 0 {
                    continue;
                }
            }
            let quiet_since = sig.2.max(watch.seen[&path].1);
            watch.ready_at.insert(path.clone(), quiet_since + wait as f64);
            if wait > 0 && now() - quiet_since < wait as f64 {
                continue;
            }
            drop(watch);
            let units = book_units(&path);
            if units.is_empty() {
                continue;
            }
            added.extend(self.add_units(app, &path, &units));
            let mut watch = self.watch.lock().unwrap_or_else(|p| p.into_inner());
            watch.complete.remove(&name);
            watch.seen.remove(&path);
            watch.ready_at.remove(&path);
        }
        {
            let mut watch = self.watch.lock().unwrap_or_else(|p| p.into_inner());
            let gone: Vec<PathBuf> = watch.seen.keys().filter(|p| !present.contains(*p)).cloned().collect();
            for p in gone {
                watch.seen.remove(&p);
                watch.ready_at.remove(&p);
            }
        }
        let ig = self.ignored(app);
        if !ig.is_empty() && ig.keys().any(|n| !present.contains(&self.inbox.join(n))) {
            // deleted by hand meanwhile: no need to remember it
            let keep: Map<String, Value> = ig
                .into_iter()
                .filter(|(n, _)| present.contains(&self.inbox.join(n)))
                .collect();
            let _ = app.db.put("inbox_ignored", &Value::Object(keep));
        }
        self.note(match added.len() {
            0 => t("Nichts Neues", &[]),
            1 => t("1 neues Hörbuch", &[]),
            n => t("{n} neue Hörbücher", &[("n", &n.to_string())]),
        });
        app.publish("audiobooks");
        added
    }

    /// Jobs for one inbox entry; a download with several volumes becomes one job per volume – only the chosen
    /// volumes, and (without a choice) none that are already owned.
    fn add_units(&self, app: &Arc<App>, path: &Path, units: &[PathBuf]) -> Vec<JobRef> {
        if units == [path.to_path_buf()] {
            return vec![self.add(app, path, "inbox", None, false, None)];
        }
        let name = base_name(&path.display().to_string());
        let info = packs::pack_info(&name).unwrap_or(json!({}));
        let grab = app.usenet.hint_for(&name).unwrap_or(json!({}));
        let chosen: Option<Vec<i64>> = grab["volumes"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_i64).collect());
        let author = [s(&info, "author"), s(&grab, "author")]
            .into_iter()
            .find(|x| !x.is_empty())
            .unwrap_or("")
            .to_string();
        let series = s(&info, "series").to_string();
        let owned = if series.is_empty() {
            Default::default()
        } else {
            app.library.series_owned(&series, &author)
        };
        let settings = app.settings();
        let mut jobs = Vec::new();
        for u in units {
            let vol = packs::volume_of(&base_name(&u.display().to_string()), Some(&info));
            if let (Some(c), Some(v)) = (&chosen, vol)
                && !c.contains(&v)
            {
                text::info(
                    &name,
                    &t("Band {n} nicht ausgewählt – übersprungen", &[("n", &v.to_string())]),
                );
                self.dispose_path(u, &settings);
                continue;
            }
            if chosen.is_none()
                && let Some(v) = vol
                && let Some(where_) = owned.get(&v)
            {
                text::info(
                    &name,
                    &t(
                        "Band {n} ist schon in der Bibliothek ({where}) – übersprungen",
                        &[("n", &v.to_string()), ("where", where_)],
                    ),
                );
                self.dispose_path(u, &settings);
                continue;
            }
            let pack =
                json!({"name": name, "volume": vol, "series": series, "author": author, "chosen": chosen.is_some()});
            jobs.push(self.add(app, u, "inbox", None, false, Some(pack)));
        }
        jobs
    }

    // -- running a stage -----------------------------------------------------------------------------
    /// Run the job's current stage (called by an engine worker).
    pub fn step(&self, app: &Arc<App>, slot: &JobRef) {
        let stage = {
            let mut j = slot.lock();
            if j.status != "queued" || !self.list().iter().any(|x| Arc::ptr_eq(x, slot)) {
                return; // cancelled or removed while waiting
            }
            slot.cancel.store(false, Ordering::SeqCst);
            j.status = stage_status(&j.stage).into();
            j.stage.clone()
        };
        self.save(app);
        let result: Res<()> = match stage.as_str() {
            "prepare" => (|| {
                self.prepare(app, slot)?;
                if slot.lock().matched.is_none() {
                    self.do_match(app, slot);
                }
                if !self.skip_owned_volume(app, slot) {
                    let mut j = slot.lock();
                    j.stage = "convert".into();
                    j.status = "queued".into();
                }
                Ok(())
            })(),
            "convert" => (|| {
                self.convert(app, slot)?;
                let mut j = slot.lock();
                j.stage = "finish".into();
                if j.matched.is_none() {
                    j.status = "review".into();
                    let msg = if j.candidates.is_empty() {
                        t("Kein Treffer – bitte suchen oder Angaben eintragen", &[])
                    } else {
                        t("Bitte Treffer auswählen", &[])
                    };
                    j.info(&msg);
                } else {
                    j.status = "queued".into();
                }
                Ok(())
            })(),
            _ => {
                let _g = self.file_lock.lock().unwrap_or_else(|p| p.into_inner());
                self.finish(app, slot)
            }
        };
        let stopping = self.stopping.load(Ordering::SeqCst) && !slot.cancel.load(Ordering::SeqCst);
        match result {
            Ok(()) => {}
            Err(_) if stopping => {
                // Hörwerk shuts down: the step runs again after the next start
                let mut j = slot.lock();
                j.status = "queued".into();
                j.info(&t(
                    "Beim Beenden unterbrochen – geht nach dem nächsten Start weiter",
                    &[],
                ));
            }
            Err(e) if matches!(e, PipeError::Cancelled) || slot.cancel.load(Ordering::SeqCst) => {
                let mut j = slot.lock();
                j.status = "cancelled".into();
                j.info(&t("Abgebrochen", &[]));
                self.cleanup_work(&j.id);
            }
            Err(e) => self.fail(slot, &e.to_string()),
        }
        *slot.runner.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.save(app);
        let status = slot.lock().status.clone();
        if matches!(status.as_str(), "done" | "review" | "failed") {
            book_event(app, slot);
        }
        self.schedule(app, slot);
    }

    fn prepare(&self, app: &Arc<App>, slot: &JobRef) -> Res<()> {
        let (source, rename, names, name, origin, rel, pack) = {
            let j = slot.lock();
            (
                PathBuf::from(&j.source),
                j.rename_files,
                j.chapter_names.clone(),
                j.name.clone(),
                j.origin.clone(),
                j.rel.clone(),
                j.pack.clone(),
            )
        };
        if !source.exists() {
            return Err(PipeError::Failed(t(
                "Quelle nicht mehr vorhanden: {path}",
                &[("path", &source.display().to_string())],
            )));
        }
        {
            let mut j = slot.lock();
            j.status = "matching".into();
            j.say(&t("Lese Dateien …", &[]), Some(0.0), "info");
        }
        let (folder, mut files) = if source.is_file() {
            (
                source.parent().unwrap_or(Path::new("")).to_path_buf(),
                vec![base_name(&source.display().to_string())],
            )
        } else {
            (source.clone(), media::collect_audio(&source))
        };
        if files.is_empty() {
            return Err(PipeError::Failed(t("Keine Audiodateien gefunden", &[])));
        }
        if rename && names.as_ref().is_some_and(|n| n.len() == files.len()) {
            files = rename_tracks(&folder, &files, names.as_deref().unwrap_or(&[]))?;
            let mut j = slot.lock();
            j.rename_files = false; // done – a retry must not rename again
            let msg = if files.len() == 1 {
                t("1 Track umbenannt", &[])
            } else {
                t("{n} Tracks umbenannt", &[("n", &files.len().to_string())])
            };
            j.info(&msg);
        }
        let suspicious = suspicious_files(&source);
        let infos: Vec<Value> = files.iter().map(|f| probe_cached(&folder.join(f))).collect();
        let duration: f64 = infos.iter().map(|i| i["duration"].as_f64().unwrap_or(0.0)).sum();
        let tags = infos.first().map(|i| i["tags"].clone()).unwrap_or(json!({}));
        let mut folder_name = name.clone();
        if rx::is_match(
            rx!(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)?[\s_.-]*\d+\b$"),
            name.trim(),
        ) || rx::sub(rx!(r"[\W\d_]"), &name, "").chars().count() < 3
        {
            let parent = source
                .parent()
                .map(|p| base_name(&p.display().to_string()))
                .unwrap_or_default();
            if !parent.is_empty() {
                folder_name = parent;
            }
        }
        let album = s(&tags, "album").trim().to_string();
        let author = [s(&tags, "album_artist"), s(&tags, "artist")]
            .into_iter()
            .find(|x| !x.is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        let query = if !album.is_empty() && !author.is_empty() {
            audible::clean_query(&format!("{album} {author}"))
        } else if origin == "library" {
            // Author/Series/NN - Title: the whole path says more than the last folder
            audible::clean_query(
                &rel.split('/')
                    .map(|p| rx::sub(rx!(r"^\s*\d+(?:[.,]\d+)?\s*[-–.]\s*"), p, ""))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        } else if !folder_name.trim().contains(' ') {
            // scene-style names have dots instead of spaces
            audible::clean_query(&rx::sub(rx!(r"[._]+"), &folder_name, " "))
        } else {
            audible::clean_query(&folder_name)
        };
        let various = matches!(
            author.to_lowercase().as_str(),
            "various artists" | "various" | "unknown" | "unbekannt"
        );
        let narrator = [s(&tags, "composer"), s(&tags, "narrator")]
            .into_iter()
            .find(|x| !x.is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        let mut hints = json!({
            "query": query, "title": if album.is_empty() { String::new() } else { audible::clean_query(&album) },
            "author": if various { String::new() } else { author.clone() }, "narrator": narrator,
            "asin": audible::find_asin(&[s(&tags, "asin"), s(&tags, "audible_asin"), &name]), "duration": duration, "folder": folder_name,
        });
        let mut notes: Vec<String> = Vec::new();
        if let Some(why) = order_doubt(&files, &infos) {
            // the files are merged in the order of their names – the tags say otherwise: a person looks first
            hints["order_doubt"] = json!(why);
            notes.push(t("Reihenfolge unsicher: {why}", &[("why", &why)]));
        }
        if let Some(p) = &pack {
            // one volume of a pack: author and series from the download, the title from this folder
            let mut title = packs::title_of(if album.is_empty() { &name } else { &album }, Some(p));
            if title.is_empty() {
                title = packs::title_of(&name, Some(p));
            }
            let pa = if s(p, "author").is_empty() {
                hints["author"].as_str().unwrap_or("").to_string()
            } else {
                s(p, "author").to_string()
            };
            let q = audible::clean_query(&format!("{title} {pa}"));
            hints["title"] = json!(title);
            hints["author"] = json!(pa);
            hints["series"] = json!(s(p, "series"));
            hints["series_position"] = p["volume"].clone();
            if !q.is_empty() {
                hints["query"] = json!(q);
            }
            let vol = if p["volume"].is_null() {
                "?".to_string()
            } else {
                p["volume"].to_string()
            };
            notes.push(t("Band {n} aus „{pack}“", &[("n", &vol), ("pack", s(p, "name"))]));
        }
        let grab = if pack.is_none() {
            app.usenet.hint_for(&base_name(&source.display().to_string()))
        } else {
            None
        };
        if let Some(g) = &grab {
            if !s(g, "asin").is_empty() || !s(g, "title").is_empty() {
                // downloaded for a book picked in the shop: its ASIN / title and author are the surest hint
                for k in ["title", "author", "asin"] {
                    if !s(g, k).is_empty() {
                        hints[k] = g[k].clone();
                    }
                }
                let what = [s(g, "title"), s(g, "query"), s(g, "asin")]
                    .into_iter()
                    .find(|x| !x.is_empty())
                    .unwrap_or("");
                notes.push(t("Download für „{title}“", &[("title", what)]));
            }
            if g["request_id"].as_i64().is_some_and(|r| r != 0) {
                hints["request_id"] = g["request_id"].clone(); // the request reports back when this is done
            } else if !s(g, "query").is_empty() {
                let cq = audible::clean_query(s(g, "query"));
                hints["alt_query"] = json!(if cq.is_empty() { s(g, "query").to_string() } else { cq });
                notes.push(t("Download aus der Suche „{query}“", &[("query", s(g, "query"))]));
            }
        }
        let manual = if source.is_dir() {
            tagger::load_manual(&source).map_err(PipeError::Failed)?
        } else {
            None
        };
        let mut j = slot.lock();
        j.files = files;
        if !suspicious.is_empty() {
            let mut list = suspicious.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
            if suspicious.len() > 5 {
                list.push_str(" …");
            }
            j.info(&t(
                "⚠ Verdächtige Dateien im Download (werden weder ausgeführt noch übernommen): {files}",
                &[("files", &list)],
            ));
            hints["suspicious"] = json!(suspicious.iter().take(20).collect::<Vec<_>>());
        }
        j.duration = duration;
        j.hints = hints;
        for n in notes {
            j.info(&n);
        }
        // a correction: the metadata.json beside a filed book is what Hörwerk wrote – the data in question
        if let Some(m) = manual.filter(|_| !j.review_first) {
            let src = s(&m, "source").to_string();
            j.matched = Some(m);
            j.info(&t("Angaben aus {source}", &[("source", &src)]));
        }
        Ok(())
    }

    fn do_match(&self, app: &Arc<App>, slot: &JobRef) {
        let s_ = app.settings();
        let mode = sval(&s_, "ab_match", "auto");
        let order = slot.lock().hints["order_doubt"].as_str().map(str::to_string);
        if mode == "off"
            && let Some(why) = &order
        {
            slot.say(&t("Bitte prüfen: {why}", &[("why", why)]));
            return;
        }
        if mode == "off" && !slot.lock().review_first {
            slot.lock().matched = Some(json!({"source": "tags"}));
            return;
        }
        let region = slot
            .lock()
            .region
            .clone()
            .unwrap_or_else(|| sval(&s_, "ab_region", "de"));
        slot.say(&t("Suche bei Audible ({region}) …", &[("region", &region)]));
        let fallback = sbool(&s_, "ab_fallback", true);
        {
            // a correction: the ASIN in the tags is what was wrong – look up by title and author instead,
            // so that the other editions show up as choices
            let mut j = slot.lock();
            if j.review_first {
                j.hints["asin"] = Value::Null;
            }
        }
        let hints = slot.lock().hints.clone();
        let (mut ranked, mut notes) = app.providers.lookup(&hints, &region, fallback);
        let (name, duration) = {
            let j = slot.lock();
            (j.name.clone(), j.duration)
        };
        if !s(&hints, "asin").is_empty()
            && let Some(top) = ranked.first().cloned()
            && let Some(why) = releases::mismatch(&name, &top, duration)
        {
            // the ASIN of the download (or of the requested book) – but is that really what came?
            slot.say(&t(
                "Passt nicht zu „{title}“ (ASIN {asin}): {why} – suche nach dem Inhalt",
                &[("title", s(&top, "title")), ("asin", s(&hints, "asin")), ("why", &why)],
            ));
            self.wrong_request_download(app, slot, &why);
            slot.lock().hints["asin"] = Value::Null;
            let hints = slot.lock().hints.clone();
            (ranked, notes) = app.providers.lookup(&hints, &region, fallback);
        }
        for n in &notes {
            slot.say(n);
        }
        let threshold = s_
            .get("ab_threshold")
            .and_then(Value::as_f64)
            .filter(|x| *x != 0.0)
            .unwrap_or(0.8);
        let ranked = self.with_requests(app, slot, ranked, &region);
        let mut ranked = self.demote_mismatches(slot, ranked);
        if ranked.is_empty()
            || ranked
                .iter()
                .take(5)
                .all(|c| !c["mismatch"].is_null() && c["mismatch"] != false)
        {
            // the shop found books – but none of them is this one: search by content
            let better = self.search_by_content(app, slot, &region, fallback);
            if !better.is_empty() {
                let ids: HashSet<String> = better.iter().map(candidate_id).collect();
                let rest: Vec<Value> = ranked.into_iter().filter(|c| !ids.contains(&candidate_id(c))).collect();
                ranked = better.into_iter().chain(rest).collect();
            } else if !ranked.is_empty() {
                slot.say(&t(
                    "Kein Treffer passt zu den Dateien – bitte selbst suchen oder eintragen",
                    &[],
                ));
            }
        }
        {
            let mut j = slot.lock();
            j.candidates = ranked.iter().take(12).cloned().collect();
            j.region = Some(region.clone());
        }
        let mut best = if mode == "auto" {
            audible::auto_match(&ranked, threshold)
        } else {
            None
        };
        if mode == "auto" && best.is_none() && ranked.first().is_some_and(|r| !r["request_id"].is_null()) {
            // a requested book from another shop: clearly the best, certain enough → take it
            best = audible::best_match(&ranked, threshold);
        }
        if let Some(why) = &order
            && best.is_some()
        {
            slot.say(&t("Bitte prüfen: {why}", &[("why", why)]));
            best = None;
        }
        if slot.lock().review_first {
            // taken back for a correction: the person who asked for it picks
            if best.is_some() {
                slot.say(&t("Zur Korrektur: bitte den richtigen Treffer wählen", &[]));
            }
            best = None;
        }
        if let Some(b) = best.clone()
            && let Some(why) = self.doubt(app, slot, &b, &region)
        {
            // fits on paper, but something about the recording is off: a person decides
            slot.say(&t("Bitte prüfen: {why}", &[("why", &why)]));
            best = None;
        }
        if let Some(b) = best {
            {
                let mut j = slot.lock();
                j.matched = Some(strip_match(&b));
                let authors = b["authors"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                let score = releases::py_round(b["match"]["score"].as_f64().unwrap_or(0.0) * 100.0).to_string();
                j.info(&t(
                    "Erkannt: {title} – {authors} (Trefferquote {score} %)",
                    &[("title", s(&b, "title")), ("authors", &authors), ("score", &score)],
                ));
            }
            self.link_request(app, slot, &b);
        } else if ranked
            .first()
            .is_some_and(|r| !audible::AUTO_SOURCES.contains(&s(r, "source")))
        {
            slot.say(&t(
                "Nur Treffer außerhalb von Audible – bitte prüfen (ohne Laufzeit und Sprecher)",
                &[],
            ));
        } else if !ranked.is_empty() {
            slot.say(&t(
                "{n} mögliche Treffer, keiner eindeutig",
                &[("n", &ranked.len().to_string())],
            ));
        } else {
            slot.say(&t("Kein Treffer", &[]));
        }
        self.save(app);
    }

    /// Why a match should not be filed without a look (runtime a bit off, another narrator in the tags, the
    /// chapters of a single M4B not lining up with the shop's) – or None.
    fn doubt(&self, app: &Arc<App>, slot: &JobRef, book: &Value, region: &str) -> Option<String> {
        let (duration, narrator, file) = {
            let j = slot.lock();
            let src = PathBuf::from(&j.source);
            let file = (j.files.len() == 1).then(|| {
                if src.is_dir() {
                    src.join(&j.files[0])
                } else {
                    src.clone()
                }
            });
            (j.duration, j.hint("narrator"), file)
        };
        // the chapters as a fingerprint: only for one M4B/M4A with real chapter names (CD tracks say nothing)
        let agreement = file
            .filter(|f| {
                let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                matches!(ext.as_str(), "m4b" | "m4a") && !s(book, "asin").is_empty()
            })
            .and_then(|f| {
                let mine = encoder::file_chapters(&f);
                if mine.len() < 4 || encoder::looks_generic(&mine) {
                    return None;
                }
                let shop = app.providers.client.chapters(s(book, "asin"), region)?;
                let starts: Vec<f64> = shop["chapters"]
                    .as_array()?
                    .iter()
                    .filter_map(|c| c["start"].as_f64())
                    .collect();
                let shop_len = shop["runtime_ms"].as_f64().unwrap_or(0.0) / 1000.0;
                let mine: Vec<f64> = mine.iter().map(|c| c.start).collect();
                releases::chapter_agreement(&mine, &starts, duration, shop_len)
            });
        releases::doubt(book, duration, &narrator, agreement)
    }

    /// Downloaded for a request, but it is another book: the request blocks this release and searches on.
    fn wrong_request_download(&self, app: &Arc<App>, slot: &JobRef, why: &str) {
        let (rid, source) = {
            let mut j = slot.lock();
            let rid = j
                .hints
                .as_object_mut()
                .and_then(|o| o.remove("request_id"))
                .and_then(|v| v.as_i64());
            (rid, base_name(&j.source))
        };
        let Some(rid) = rid else { return };
        slot.say(&t("Anfrage sucht weiter, dieses Release ist gesperrt", &[]));
        app.requests.wrong_download(app, rid, &source, why);
    }

    /// The author from the tags – or from a folder called "Stephenie Meyer-Twilight-Saga 5 - …".
    fn folder_author(job: &Job) -> String {
        if !job.hint("author").is_empty() {
            return job.hint("author");
        }
        let first = rx::split(rx!(r"\s+[-–]\s+|(?<=[a-zäöüß])-(?=[A-ZÄÖÜ])"), &job.name)
            .into_iter()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let words: Vec<String> = first
            .replace('.', ". ")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        if (2..=4).contains(&words.len()) && words.iter().all(|w| rx::is_match(rx!(r"^[A-ZÄÖÜ][\wäöüß'.]*$"), w))
        {
            return first;
        }
        String::new()
    }

    fn content_regions(name: &str, region: &str) -> Vec<String> {
        let lang = crate::usenet::release_info(name)["language"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| releases::guess_language(name));
        let mut out = vec![region.to_string()];
        if lang == "en" && !matches!(region, "us" | "uk" | "au" | "ca" | "in") {
            out.push("us".into());
        }
        if lang == "de" && region != "de" {
            out.push("de".into());
        }
        out
    }

    fn search_by_content(&self, app: &Arc<App>, slot: &JobRef, region: &str, fallback: bool) -> Vec<Value> {
        let (author, query, name, hints) = {
            let j = slot.lock();
            let q = if j.hint("query").is_empty() {
                audible::clean_query(&j.name)
            } else {
                j.hint("query")
            };
            (Self::folder_author(&j), q, j.name.clone(), j.hints.clone())
        };
        let queries = audible::content_queries(&query, &author, 8);
        let mut tried = 0;
        for place in Self::content_regions(&name, region) {
            for q in &queries {
                if tried >= 10 {
                    return Vec::new();
                }
                tried += 1;
                let mut h = hints.clone();
                h["query"] = json!(q);
                h["title"] = json!("");
                h["author"] = json!(author);
                h["asin"] = Value::Null;
                let (found, _) = app.providers.lookup(&h, &place, fallback);
                let fitting: Vec<Value> = self
                    .demote_mismatches(slot, found)
                    .into_iter()
                    .filter(|c| c["mismatch"].is_null())
                    .collect();
                if !fitting.is_empty() {
                    let suffix = if place != region {
                        format!(" ({place})")
                    } else {
                        String::new()
                    };
                    slot.say(&format!(
                        "{}{suffix}",
                        t("Passende Treffer mit „{query}“", &[("query", q)])
                    ));
                    return fitting;
                }
            }
        }
        Vec::new()
    }

    /// Candidates the download clearly can't be (other volume, language, runtime) go down, never auto; a candidate
    /// with exactly the volume the folder names goes up.
    fn demote_mismatches(&self, slot: &JobRef, ranked: Vec<Value>) -> Vec<Value> {
        let (name, duration) = {
            let j = slot.lock();
            (j.name.clone(), j.duration)
        };
        let mut out: Vec<Value> = ranked
            .into_iter()
            .map(|mut c| {
                if let Some(why) = releases::mismatch(&name, &c, duration) {
                    let mut m = c["match"].as_object().cloned().unwrap_or_default();
                    let sc = m.get("score").and_then(Value::as_f64).unwrap_or(0.0);
                    m.insert("score".into(), json!((sc * 0.5 * 1000.0).round_ties_even() / 1000.0));
                    c["mismatch"] = json!(why);
                    c["match"] = Value::Object(m);
                }
                c
            })
            .collect();
        let named = |c: &Value| releases::volumes(c).is_some_and(|v| releases::release_volumes(&name, c).contains(&v));
        out.sort_by_key(|b| (!b["mismatch"].is_null(), !named(b)));
        out
    }

    /// Mark candidates that are an open book request; add requested books the shop search missed.
    fn with_requests(&self, app: &Arc<App>, slot: &JobRef, ranked: Vec<Value>, region: &str) -> Vec<Value> {
        let (hints, name, duration) = {
            let j = slot.lock();
            (j.hints.clone(), j.name.clone(), j.duration)
        };
        if !hints["request_id"].is_null() {
            return ranked; // downloaded for a request already: nothing to guess
        }
        let reqs = app.requests.open_for_matching(app);
        if reqs.is_empty() {
            return ranked;
        }
        let mut by_key: Vec<(String, Value)> = reqs.into_iter().map(|r| (s(&r, "key").to_string(), r)).collect();
        let mut out = Vec::new();
        for mut c in ranked {
            let key = crate::requests::book_key(&c);
            if let Some(i) = by_key.iter().position(|(k, _)| *k == key) {
                let (_, r) = by_key.remove(i);
                c["request_id"] = r["id"].clone();
                c["request_user"] = r["user"].clone();
            }
            out.push(c);
        }
        for (_, r) in by_key {
            let mut book = r["book"].clone();
            if !s(&book, "asin").is_empty()
                && let Ok(full) = app.providers.client.product(s(&book, "asin"), region)
            {
                book = full; // the full Audible data (runtime, narrators, chapters)
                book["source"] = json!("audible");
            }
            let sc = audible::score(&book, &hints);
            if sc["score"].as_f64().unwrap_or(0.0) >= 0.5 && releases::mismatch(&name, &book, duration).is_none() {
                if s(&book, "source").is_empty() {
                    book["source"] = json!("audible");
                }
                book["match"] = sc;
                book["request_id"] = r["id"].clone();
                book["request_user"] = r["user"].clone();
                out.push(book);
            }
        }
        out.sort_by(|a, b| {
            let k = |x: &Value| {
                (
                    -x["match"]["score"].as_f64().unwrap_or(0.0),
                    x["format_type"] != "unabridged",
                    x["request_id"].is_null(),
                )
            };
            let (ka, kb) = (k(a), k(b));
            ka.0.partial_cmp(&kb.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(ka.1.cmp(&kb.1))
                .then(ka.2.cmp(&kb.2))
        });
        out
    }

    fn link_request(&self, app: &Arc<App>, slot: &JobRef, book: &Value) {
        let linked = {
            let mut j = slot.lock();
            if !book["request_id"].is_null() && j.hints["request_id"].is_null() {
                j.hints["request_id"] = book["request_id"].clone();
                let who = if s(book, "request_user").is_empty() {
                    "–"
                } else {
                    s(book, "request_user")
                };
                j.info(&t("Gehört zur Anfrage von {name}", &[("name", who)]));
                Some(j.to_dict(false))
            } else {
                None
            }
        };
        if let Some(job) = linked {
            app.requests.on_job(app, &job);
        }
    }

    fn convert(&self, app: &Arc<App>, slot: &JobRef) -> Res<()> {
        let s_ = app.settings();
        let (id, source, files, chapter_names, matched, name) = {
            let mut j = slot.lock();
            j.status = "converting".into();
            (
                j.id.clone(),
                PathBuf::from(&j.source),
                j.files.clone(),
                j.chapter_names.clone(),
                j.matched.clone(),
                j.name.clone(),
            )
        };
        let work = self.job_work(&id);
        std::fs::create_dir_all(&work)?;
        let folder = if source.is_dir() {
            source.clone()
        } else {
            source.parent().unwrap_or(Path::new("")).to_path_buf()
        };
        let out = work.join("book.m4b");
        let runner = Arc::new(Runner::new(slot.cancel.clone()));
        *slot.runner.lock().unwrap_or_else(|p| p.into_inner()) = Some(runner.clone());
        if self.stopping.load(Ordering::SeqCst) {
            runner.kill();
        }
        let nice = app.cfg.nice.clone();
        let mut chapters_txt: Option<Vec<Chapter>> = None;
        if source.is_dir() {
            let mut names: Vec<String> = std::fs::read_dir(&folder)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            for f in names {
                let p = folder.join(&f);
                if f.to_lowercase().ends_with("chapters.txt")
                    && media::is_regular(&p)
                    && std::fs::metadata(&p).is_ok_and(|m| m.len() <= encoder::MAX_TEXT_FILE)
                {
                    let bytes = std::fs::read(&p)?;
                    chapters_txt = Some(encoder::parse_chapters_txt(&String::from_utf8_lossy(&bytes)));
                    break;
                }
            }
        }
        let first = folder.join(&files[0]);
        let single = files.len() == 1
            && [".m4b", ".m4a", ".mp4"]
                .iter()
                .any(|x| files[0].to_lowercase().ends_with(x));
        let profile = sval(&s_, "audio_quality", "high");
        let drama = quality::is_drama(matched.as_ref().unwrap_or(&json!({})), &name);
        if drama {
            slot.say(&t("Hörspiel erkannt – höhere Qualität beim Umwandeln", &[]));
        }
        let codec = probe_cached(&first)["audio_codec"].as_str().unwrap_or("").to_string();
        if single && (codec == "aac" || quality::keep_codec(&codec, &profile)) {
            slot.lock().say(
                &t(
                    "M4B übernehmen (ohne Neukodierung, {codec}) …",
                    &[("codec", &codec.to_uppercase())],
                ),
                Some(0.2),
                "info",
            );
            let ch = chapters_txt.as_deref().filter(|c| !c.is_empty());
            encoder::remux(&first, &out, ch, &runner, &nice)?;
            tagger::copy_tags(&first, &out)?;
            slot.lock().converted_info = json!({"copied": true});
        } else {
            let msg = if files.len() == 1 {
                t("Konvertiere 1 Datei …", &[])
            } else {
                t("Konvertiere {n} Dateien …", &[("n", &files.len().to_string())])
            };
            slot.lock().say(&msg, Some(0.0), "info");
            let a = app.clone();
            let progress = move |p: f64, msg: String| {
                {
                    let mut j = slot.lock();
                    j.progress = p;
                    j.message = msg;
                    j.updated = now();
                }
                a.publish("audiobooks");
            };
            let bitrate = sval(&s_, "bitrate", "auto");
            let mode = sval(&s_, "chapter_titles", "auto");
            let book = matched.clone().unwrap_or(json!({}));
            let known = Self::known_names(&book);
            let lang = Self::chapter_lang(&book, &name);
            let m = encoder::Merge {
                bitrate: &bitrate,
                mono: sbool(&s_, "force_mono", false),
                jobs: app.cfg.cpu_jobs,
                chapter_mode: &mode,
                known: &known,
                lang: &lang,
                titles: chapter_names.as_deref(),
                nice: &nice,
                profile: &profile,
                drama,
            };
            let info = encoder::merge_to_m4b(&folder, &files, &out, &m, &progress, &runner)?;
            if let Some(ch) = &chapters_txt {
                let tmp = PathBuf::from(format!("{}.tmp.m4b", out.display()));
                encoder::remux(&out, &tmp, Some(ch), &runner, &nice)?;
                std::fs::rename(&tmp, &out)?;
            }
            let mut j = slot.lock();
            j.converted_info = json!({"bitrate": info.bitrate, "chapters": info.chapters.len(), "copied": info.copied,
                                      "repetitive": info.repetitive});
            j.say(
                &t(
                    "M4B fertig: {n} Kapitel, {bitrate}",
                    &[("n", &info.chapters.len().to_string()), ("bitrate", &info.bitrate)],
                ),
                Some(1.0),
                "info",
            );
        }
        let d = probe_cached(&out)["duration"].as_f64().filter(|d| *d != 0.0);
        let cover = if source.is_dir() {
            encoder::find_cover(&folder)
        } else {
            None
        }
        .or_else(|| encoder::extract_cover(&first, &work));
        let mut j = slot.lock();
        if let Some(d) = d {
            j.duration = d;
        }
        if let Some(c) = cover {
            let ext = c
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                .unwrap_or_default();
            let dst = work.join(format!("local-cover{ext}"));
            std::fs::copy(&c, &dst)?;
            j.cover = Some(dst.display().to_string());
        }
        j.m4b = Some(out.display().to_string());
        Ok(())
    }

    fn finish(&self, app: &Arc<App>, slot: &JobRef) -> Res<()> {
        let s_ = app.settings();
        let settings = Value::Object(s_.clone());
        let (m4b, matched, origin, source, dup_decision, id) = {
            let j = slot.lock();
            (
                j.m4b.clone().unwrap_or_default(),
                j.matched.clone().unwrap_or(json!({})),
                j.origin.clone(),
                j.source.clone(),
                j.dup_decision.clone(),
                j.id.clone(),
            )
        };
        let m4b_path = PathBuf::from(&m4b);
        let mut dup_decision = dup_decision;
        // a better release for a book that is there: it replaces exactly that book (the old copy is archived)
        let upgrade = slot.lock().hints["request_id"]
            .as_i64()
            .and_then(|rid| app.requests.get(app, rid))
            .and_then(|r| r["upgrade"]["path"].as_str().map(str::to_string))
            .filter(|p| !p.is_empty() && self.library.join(p).is_dir());
        if dup_decision.is_none()
            && let Some(up) = &upgrade
        {
            let entry = app
                .library
                .entries()
                .into_iter()
                .find(|e| s(e, "path") == up)
                .unwrap_or_else(|| json!({"path": up}));
            let mut dup: Map<String, Value> = ["path", "title", "authors", "asin", "format"]
                .iter()
                .map(|k| (k.to_string(), entry.get(*k).cloned().unwrap_or(Value::Null)))
                .collect();
            dup.insert("why".into(), json!("upgrade"));
            dup.insert(
                "quality".into(),
                json!({"old": Self::quality_of(&self.library.join(up)), "new": Self::quality_of(&m4b_path)}),
            );
            let mut j = slot.lock();
            j.duplicate = Some(Value::Object(dup));
            j.dup_decision = Some("replace".into());
            j.info(&t("Ersetzt das vorhandene Buch: {path}", &[("path", up)]));
            dup_decision = Some("replace".into());
        }
        let mut book = matched;
        if s(&book, "source") == "tags" {
            // no lookup: keep what the files say, fill gaps from the folder name
            let mut b = book_from_hints(&slot.lock());
            for (k, v) in tagger::read_book(&m4b_path).as_object().cloned().unwrap_or_default() {
                let empty = matches!(v, Value::Null | Value::Bool(false)) || v == json!("") || v == json!([]);
                if !empty && !k.starts_with('_') {
                    b[k] = v;
                }
            }
            b["source"] = json!("tags");
            book = b;
        }
        // already in the library? ask (or follow the setting) instead of silently adding a "(2)"
        let dup_mode = sval(&s_, "ab_duplicates", "ask");
        if dup_mode != "keep" && dup_decision.is_none() {
            let own = (origin == "library").then(|| rel_to(&source, &self.library));
            let found: Vec<Value> = app
                .library
                .find(&book)
                .into_iter()
                .filter(|d| Some(s(d, "path")) != own.as_deref())
                .collect();
            if let Some(d) = found.first() {
                let mut dup: Map<String, Value> = ["path", "title", "authors", "asin", "format", "why"]
                    .iter()
                    .map(|k| (k.to_string(), d.get(*k).cloned().unwrap_or(Value::Null)))
                    .collect();
                dup.insert("quality".into(), json!({"old": Self::quality_of(&self.library.join(s(d, "path"))), "new": Self::quality_of(&m4b_path)}));
                let why = match s(d, "why") {
                    "asin" => t("gleiche ASIN", &[]),
                    "title" => t("gleicher Autor und Titel", &[]),
                    _ => t("gleiche Serie und Folge", &[]),
                };
                let where_ = format!("{} ({why})", s(d, "path"));
                if dup_mode == "skip" {
                    self.dispose_source(slot, &s_);
                    self.cleanup_work(&id);
                    let mut j = slot.lock();
                    j.duplicate = Some(Value::Object(dup));
                    j.m4b = None;
                    j.status = "cancelled".into();
                    j.info(&t(
                        "Übersprungen – schon in der Bibliothek: {where}",
                        &[("where", &where_)],
                    ));
                    return Ok(());
                }
                let mut j = slot.lock();
                j.duplicate = Some(Value::Object(dup));
                j.status = "review".into();
                j.info(&t(
                    "Schon in der Bibliothek: {where} – bitte entscheiden",
                    &[("where", &where_)],
                ));
                return Ok(());
            }
        }
        let mut parked: Vec<Option<(PathBuf, PathBuf, String)>> = Vec::new();
        let dup = slot.lock().duplicate.clone();
        if dup_decision.as_deref() == Some("replace")
            && let Some(d) = &dup
        {
            parked.push(self.park(app, &id, s(d, "path"), "archive")?);
            self.journal(app, slot, &parked, None);
        }
        {
            let mut j = slot.lock();
            j.status = "tagging".into();
            j.say(&t("Schreibe Tags …", &[]), Some(1.0), "info");
        }
        // cover: shop (full resolution) > image in the folder > embedded
        let mut cover: Option<Vec<u8>> = None;
        if !s(&book, "cover_url").is_empty() && sval(&s_, "ab_cover_source", "audible") == "audible" {
            match audible::fetch_image(s(&book, "cover_url")) {
                Ok(Some(c)) => cover = Some(c),
                Ok(None) => slot.say(&t(
                    "Cover aus dem Shop übersprungen (keine öffentliche Adresse oder kein JPEG/PNG)",
                    &[],
                )),
                Err(e) => slot.say(&t("Cover-Download fehlgeschlagen: {error}", &[("error", &e.0)])),
            }
        }
        if cover.is_none()
            && let Some(c) = slot.lock().cover.clone().filter(|c| Path::new(c).exists())
        {
            cover = std::fs::read(c).ok();
        }
        if cover.is_none() {
            cover = tagger::read_cover(&m4b_path);
        }
        // chapters from Audible when they fit the file
        let ch_mode = sval(&s_, "ab_chapters", "auto");
        if !s(&book, "asin").is_empty() && ch_mode != "files" {
            self.audible_chapters(app, slot, &book, &ch_mode)?;
        }
        if sval(&s_, "chapter_titles", "auto") == "auto" {
            self.tidy_chapters(app, slot, &book)?;
        }
        tagger::write_tags(&m4b_path, &book, &settings, cover.clone())?;
        let root = self.library.clone();
        if root.as_os_str().is_empty() {
            return Err(PipeError::Failed(t(
                "Kein Bibliotheksordner (LIBRARY_DIR) eingerichtet",
                &[],
            )));
        }
        let title = tagger::clean_title(&book, &settings);
        let (folder, filename) = filing::destination(
            &root,
            &book,
            &sval(&s_, "ab_folder_template", ""),
            &sval(&s_, "ab_file_template", ""),
            Some(&title),
        );
        if origin == "library" {
            // the new M4B replaces the old files of this very book: they step aside first …
            match self.park(
                app,
                &id,
                &rel_to(&source, &root),
                &sval(&s_, "ab_library_originals", "archive"),
            ) {
                Ok(p) => {
                    parked.push(p);
                    self.journal(app, slot, &parked, None);
                }
                Err(e) => {
                    for p in parked.iter().rev() {
                        Self::unpark(p);
                    }
                    slot.lock().filing = None;
                    return Err(e);
                }
            }
        }
        // existing author/series folders win, even when spelled a little differently
        let folder = filing::unique_folder(&filing::reuse_existing(&root, &folder));
        let created = (!folder.exists()).then(|| folder.clone());
        let placed = (|| -> Res<PathBuf> {
            std::fs::create_dir_all(&folder)?;
            tagger::write_sidecars(&folder, &book, &s_, cover.as_deref())?;
            self.journal(app, slot, &parked, Some((&folder, &filename)));
            Ok(filing::place(&m4b_path, &folder, &filename)?)
        })();
        let dest = match placed {
            Ok(d) => d,
            Err(e) => {
                if let Some(c) = created.filter(|c| c.is_dir()) {
                    let _ = std::fs::remove_dir_all(c); // half-made new folder (cover, desc.txt) goes again …
                }
                for p in parked.iter().rev() {
                    Self::unpark(p); // … and the old copy comes back
                }
                slot.lock().filing = None;
                return Err(e);
            }
        };
        slot.lock().dest = Some(dest.display().to_string());
        for p in &parked {
            self.settle(slot, p); // only now: delete or archive the old copy
        }
        if sbool(&s_, "write_abs", true) {
            let d = probe_cached(&dest)["duration"]
                .as_f64()
                .filter(|d| *d != 0.0)
                .unwrap_or_else(|| slot.lock().duration);
            tagger::write_abs_metadata(
                dest.parent().unwrap_or(Path::new("")),
                &book,
                &encoder::file_chapters(&dest),
                d,
            )?;
        }
        app.library.add(&dest, &book);
        {
            let mut j = slot.lock();
            j.m4b = None;
            j.filing = None;
        }
        self.dispose_source(slot, &s_);
        self.cleanup_work(&id);
        let mut j = slot.lock();
        j.status = "done".into();
        j.say(
            &t(
                "Abgelegt: {path}",
                &[("path", &rel_to(&dest.display().to_string(), &root))],
            ),
            Some(1.0),
            "info",
        );
        drop(j);
        crate::mediaservers::request_scans(app);
        Ok(())
    }

    /// Write down what filing is about to change – saved before it happens.
    fn journal(
        &self,
        app: &App,
        slot: &JobRef,
        parked: &[Option<(PathBuf, PathBuf, String)>],
        target: Option<(&Path, &str)>,
    ) {
        {
            let mut j = slot.lock();
            let list: Vec<Value> = parked.iter().flatten().map(|(h, o, m)| json!([h, o, m])).collect();
            let mut f = json!({"parked": list});
            if let Some((folder, file)) = target
                && let Some(size) = j
                    .m4b
                    .as_deref()
                    .and_then(|m| std::fs::metadata(m).ok())
                    .map(|m| m.len())
            {
                f["folder"] = json!(folder);
                f["file"] = json!(file);
                f["size"] = json!(size);
            }
            j.filing = Some(f);
        }
        self.save(app);
    }

    /// After a restart during filing: the M4B made it into the library -> finish; it did not -> put the old
    /// copies back. Nothing is lost or filed twice.
    fn recover_filing(&self, app: &Arc<App>, slot: &JobRef) {
        let (f, m4b, name) = {
            let j = slot.lock();
            (j.filing.clone().unwrap_or(json!({})), j.m4b.clone(), j.name.clone())
        };
        let parked: Vec<Option<(PathBuf, PathBuf, String)>> = f["parked"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                p.as_array().filter(|a| a.len() == 3).map(|a| {
                    Some((
                        PathBuf::from(a[0].as_str().unwrap_or("")),
                        PathBuf::from(a[1].as_str().unwrap_or("")),
                        a[2].as_str().unwrap_or("").to_string(),
                    ))
                })
            })
            .collect();
        let mut placed = None;
        if let Some(folder) = f["folder"].as_str().filter(|x| !x.is_empty()) {
            filing::remove_partials(Path::new(folder));
            let cand = Path::new(folder).join(s(&f, "file"));
            let m4b_there = m4b.as_deref().is_some_and(|m| Path::new(m).exists());
            if !m4b_there && cand.is_file() && std::fs::metadata(&cand).ok().map(|m| m.len()) == f["size"].as_u64() {
                placed = Some(cand);
            }
        }
        let settings = app.settings();
        if let Some(placed) = placed {
            for p in &parked {
                self.settle(slot, p);
            }
            {
                let mut j = slot.lock();
                j.dest = Some(placed.display().to_string());
                j.m4b = None;
                j.status = "done".into();
            }
            self.dispose_source(slot, &settings);
            let (id, matched, duration) = {
                let j = slot.lock();
                (j.id.clone(), j.matched.clone().unwrap_or(json!({})), j.duration)
            };
            self.cleanup_work(&id);
            if sbool(&settings, "write_abs", true) {
                let d = probe_cached(&placed)["duration"]
                    .as_f64()
                    .filter(|d| *d != 0.0)
                    .unwrap_or(duration);
                if let Err(e) = tagger::write_abs_metadata(
                    placed.parent().unwrap_or(Path::new("")),
                    &matched,
                    &encoder::file_chapters(&placed),
                    d,
                ) {
                    text::error(
                        "Pipeline",
                        &t(
                            "Nach Neustart nicht aufgeräumt ({name}): {error}",
                            &[("name", &name), ("error", &e.to_string())],
                        ),
                    );
                }
            }
            let mut j = slot.lock();
            j.say(
                &t(
                    "Nach Neustart: war schon abgelegt – {path}",
                    &[("path", &rel_to(&placed.display().to_string(), &self.library))],
                ),
                Some(1.0),
                "info",
            );
        } else {
            for p in parked.iter().rev() {
                Self::unpark(p);
            }
            if !parked.is_empty() {
                slot.say(&t("Nach Neustart: alte Fassung wieder an ihren Platz gestellt", &[]));
            }
        }
        slot.lock().filing = None;
    }

    /// Put an old book folder aside (hidden, same parent folder – a rename, no copy).
    fn park(&self, app: &App, id: &str, rel: &str, mode: &str) -> Res<Option<(PathBuf, PathBuf, String)>> {
        let old = real(&self.library.join(rel));
        if !old.starts_with(&self.library) || old == self.library || !old.is_dir() {
            return Ok(None);
        }
        let hidden = filing::free_path(
            &old.parent()
                .unwrap_or(Path::new(""))
                .join(format!(".br-alt-{id}-{}", base_name(&old.display().to_string()))),
        );
        std::fs::rename(&old, &hidden)?;
        app.library.remove(&old.join("x"));
        Ok(Some((hidden, old, mode.to_string())))
    }

    fn unpark(parked: &Option<(PathBuf, PathBuf, String)>) {
        if let Some((hidden, old, _)) = parked
            && hidden.is_dir()
            && !old.exists()
        {
            let _ = std::fs::rename(hidden, old);
        }
    }

    /// The new copy is filed: the parked old one is deleted or moved to `_ersetzt`.
    fn settle(&self, slot: &JobRef, parked: &Option<(PathBuf, PathBuf, String)>) {
        let Some((hidden, old, mode)) = parked else { return };
        let rel = rel_to(&old.display().to_string(), &self.library);
        if mode == "delete" {
            let _ = std::fs::remove_dir_all(hidden);
            slot.say(&t("Alte Dateien gelöscht: {path}", &[("path", &rel)]));
        } else {
            let base = if self.inbox.as_os_str().is_empty() {
                self.work_dir.parent().unwrap_or(Path::new("")).to_path_buf()
            } else {
                self.inbox.clone()
            };
            let bin = base.join("_ersetzt");
            let target = filing::free_path(&bin.join(&rel));
            if move_path(hidden, &target).is_ok() {
                let shown = rel_to(&target.display().to_string(), bin.parent().unwrap_or(Path::new("")));
                slot.say(&t("Alte Fassung verschoben nach {path}", &[("path", &shown)]));
            }
        }
        remove_empty_parents(old.parent().unwrap_or(Path::new("")), &self.library);
    }

    fn audible_chapters(&self, app: &Arc<App>, slot: &JobRef, book: &Value, mode: &str) -> Res<()> {
        let region = if s(book, "region").is_empty() {
            "de"
        } else {
            s(book, "region")
        };
        let Some(data) = app.providers.client.chapters(s(book, "asin"), region) else {
            return Ok(());
        };
        let chs = data["chapters"].as_array().cloned().unwrap_or_default();
        let aud = match data["runtime_ms"].as_f64().unwrap_or(0.0) / 1000.0 {
            0.0 => chs.last().and_then(|c| c["start"].as_f64()).unwrap_or(0.0),
            x => x,
        };
        let (m4b, duration) = {
            let j = slot.lock();
            (PathBuf::from(j.m4b.clone().unwrap_or_default()), j.duration)
        };
        let dur = probe_cached(&m4b)["duration"]
            .as_f64()
            .filter(|d| *d != 0.0)
            .unwrap_or(duration);
        if aud == 0.0 || dur == 0.0 {
            return Ok(());
        }
        let diff = (dur - aud).abs() / aud;
        let current = encoder::file_chapters(&m4b);
        if (dur - aud).abs() > (0.02 * aud).max(2.0) {
            slot.say(&t(
                "Audible-Kapitel passen nicht (Laufzeit weicht {diff} % ab) – behalte eigene",
                &[("diff", &format!("{:.1}", diff * 100.0))],
            ));
            return Ok(());
        }
        if mode == "auto" && !encoder::looks_generic(&current) {
            // real names – but made mostly of text every track repeated: with as many chapters as the shop
            // has, its names are the better ones (the starts stay those of the tracks)
            let repetitive = slot.lock().converted_info["repetitive"] == true;
            if repetitive && chs.len() == current.len() {
                let named: Vec<Chapter> = current
                    .iter()
                    .zip(&chs)
                    .map(|(c, a)| Chapter {
                        start: c.start,
                        title: a["title"]
                            .as_str()
                            .map(str::trim)
                            .filter(|x| !x.is_empty())
                            .map_or_else(|| c.title.clone(), str::to_string),
                    })
                    .collect();
                self.rechapter(app, &m4b, &named)?;
                slot.say(&t(
                    "{n} Kapitelnamen von Audible übernommen (Laufzeit und Kapitelzahl passen)",
                    &[("n", &named.len().to_string())],
                ));
            }
            return Ok(());
        }
        let scale = dur / aud;
        let chapters: Vec<Chapter> = chs
            .iter()
            .enumerate()
            .map(|(i, c)| Chapter {
                start: c["start"].as_f64().unwrap_or(0.0) * scale,
                title: c["title"]
                    .as_str()
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| t("Kapitel {n}", &[("n", &(i + 1).to_string())])),
            })
            .collect();
        self.rechapter(app, &m4b, &chapters)?;
        slot.say(&t(
            "{n} Kapitel von Audible übernommen",
            &[("n", &chapters.len().to_string())],
        ));
        Ok(())
    }

    fn rechapter(&self, app: &App, m4b: &Path, chapters: &[Chapter]) -> Res<()> {
        let tmp = PathBuf::from(format!("{}.chapters.m4b", m4b.display()));
        let runner = Runner::new(Arc::new(AtomicBool::new(false)));
        encoder::remux(m4b, &tmp, Some(chapters), &runner, &app.cfg.nice)?;
        tagger::copy_tags(m4b, &tmp)?;
        std::fs::rename(&tmp, m4b)?;
        Ok(())
    }

    /// What a book is called – these go from its chapter names: title, series, authors.
    fn known_names(book: &Value) -> Vec<String> {
        let mut out: Vec<String> = ["title", "series"]
            .iter()
            .map(|k| s(book, k).trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        out.extend(
            book["authors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty()),
        );
        out
    }

    /// The language for "Kapitel N" / "Chapter N": the book's, else the download's.
    fn chapter_lang(book: &Value, name: &str) -> String {
        [
            releases::book_language(book),
            crate::usenet::release_info(name)["language"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            releases::guess_language(name),
        ]
        .into_iter()
        .find(|l| !l.is_empty())
        .unwrap_or_default()
    }

    /// Clean the chapter names of a finished M4B in place (a library book, or "Kapitel aufräumen"): the rules
    /// of `chapters::clean_titles`; the chapter starts stay. (changed, first, last) – or None when nothing changed.
    pub fn tidy_file(&self, app: &App, m4b: &Path, book: &Value, name: &str) -> Res<Option<(usize, String, String)>> {
        let current = encoder::file_chapters(m4b);
        if current.len() < 2 {
            return Ok(None);
        }
        let titles: Vec<String> = current.iter().map(|c| c.title.clone()).collect();
        let cleaned = crate::chapters::clean_titles(&titles, &Self::known_names(book), &Self::chapter_lang(book, name));
        if cleaned.names == titles {
            return Ok(None);
        }
        let changed = cleaned.names.iter().zip(&titles).filter(|(a, b)| a != b).count();
        let wanted: Vec<Chapter> = current
            .iter()
            .zip(&cleaned.names)
            .map(|(c, title)| Chapter {
                start: c.start,
                title: title.clone(),
            })
            .collect();
        self.rechapter(app, m4b, &wanted)?;
        let (first, last) = (wanted[0].title.clone(), wanted[wanted.len() - 1].title.clone());
        Ok(Some((changed, first, last)))
    }

    /// Chapters that are only numbers are numbered 1, 2, 3 … in their real order, in the book's language;
    /// real but cluttered names (one M4B with "Book Title: Chapter One - …") are cleaned like track names.
    fn tidy_chapters(&self, app: &Arc<App>, slot: &JobRef, book: &Value) -> Res<()> {
        let (m4b, name) = {
            let j = slot.lock();
            (PathBuf::from(j.m4b.clone().unwrap_or_default()), j.name.clone())
        };
        let current = encoder::file_chapters(&m4b);
        if current.len() < 2 {
            return Ok(());
        }
        if !encoder::looks_generic(&current) {
            if let Some((n, first, last)) = self.tidy_file(app, &m4b, book, &name)? {
                slot.say(&t(
                    "{n} Kapitelnamen aufgeräumt: {first} … {last}",
                    &[("n", &n.to_string()), ("first", &first), ("last", &last)],
                ));
            }
            return Ok(());
        }
        let lang = Some(Self::chapter_lang(book, &name))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "de".into());
        let wanted = encoder::numbered_chapters(&current, &lang);
        if current.iter().map(|c| &c.title).eq(wanted.iter().map(|c| &c.title)) {
            return Ok(());
        }
        self.rechapter(app, &m4b, &wanted)?;
        slot.say(&t(
            "Kapitel neu nummeriert: {first} … {last}",
            &[("first", &wanted[0].title), ("last", &wanted[wanted.len() - 1].title)],
        ));
        Ok(())
    }

    fn dispose_source(&self, slot: &JobRef, s_: &Map<String, Value>) {
        let (origin, source) = {
            let j = slot.lock();
            (j.origin.clone(), j.source.clone())
        };
        if origin == "inbox" {
            self.dispose_path(Path::new(&source), s_);
        }
    }

    /// An inbox entry is done with: to `_fertig` (default) or deleted.
    fn dispose_path(&self, path: &Path, s_: &Map<String, Value>) {
        if !path.exists() {
            return;
        }
        if sval(s_, "ab_originals", "archive") == "delete" {
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
        } else {
            let target = filing::free_path(
                &self
                    .inbox
                    .join("_fertig")
                    .join(rel_to(&path.display().to_string(), &self.inbox)),
            );
            let _ = move_path(path, &target);
        }
        remove_empty_parents(path.parent().unwrap_or(Path::new("")), &self.inbox);
    }

    /// A pack volume recognised as a book we already have: skip it before converting (chosen ones are not).
    fn skip_owned_volume(&self, app: &Arc<App>, slot: &JobRef) -> bool {
        let (pack, matched, id) = {
            let j = slot.lock();
            (j.pack.clone(), j.matched.clone(), j.id.clone())
        };
        let (Some(pack), Some(m)) = (pack, matched) else {
            return false;
        };
        if pack["chosen"] == true || s(&m, "title").is_empty() {
            return false;
        }
        let found = app.library.find(&m);
        let Some(first) = found.first() else { return false };
        self.dispose_source(slot, &app.settings());
        self.cleanup_work(&id);
        let mut j = slot.lock();
        j.status = "cancelled".into();
        j.info(&t(
            "Übersprungen – schon in der Bibliothek: {where}",
            &[("where", s(first, "path"))],
        ));
        true
    }

    fn fail(&self, slot: &JobRef, msg: &str) {
        let (id, origin, source, rel) = {
            let mut j = slot.lock();
            j.status = "failed".into();
            j.error = Some(msg.to_string());
            j.say(&t("Fehler: {error}", &[("error", msg)]), None, "error");
            j.m4b = None;
            (j.id.clone(), j.origin.clone(), j.source.clone(), j.rel.clone())
        };
        self.cleanup_work(&id);
        let src = Path::new(&source);
        if origin == "inbox" && src.exists() {
            let target = filing::free_path(&self.inbox.join("_failed").join(&rel));
            match move_path(src, &target) {
                Ok(()) => {
                    remove_empty_parents(src.parent().unwrap_or(Path::new("")), &self.inbox);
                    let mut j = slot.lock();
                    j.dest = Some(target.display().to_string());
                    if target.is_dir() {
                        let _ = std::fs::write(target.join("FEHLER.txt"), format!("{msg}\n\n{}\n", j.log.join("\n")));
                    }
                }
                Err(e) => slot.say(&t(
                    "Verschieben nach _failed fehlgeschlagen: {error}",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }
}

/// A job ended up done / review / failed: the request hears about it, and a notification goes out.
fn book_event(app: &Arc<App>, slot: &JobRef) {
    let job = slot.lock().to_dict(true);
    app.requests.on_job(app, &job);
    let m = job["match"].clone();
    let name = if s(&m, "title").is_empty() {
        s(&job, "name").to_string()
    } else {
        s(&m, "title").to_string()
    };
    let who = m["authors"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    match s(&job, "status") {
        "done" => {
            let where_ = job["dest"]
                .as_str()
                .map(|d| rel_to(d, &app.cfg.library))
                .unwrap_or_default();
            let mut text = name.clone();
            if !who.is_empty() {
                text.push_str(&format!(" – {who}"));
            }
            if !where_.is_empty() {
                text.push_str(&format!("\n{where_}"));
            }
            app.notifier.send("filed", &t("Hörbuch abgelegt", &[]), &text, None);
        }
        "review" if !job["duplicate"].is_null() => app.notifier.send(
            "duplicate",
            &t("Schon in der Bibliothek", &[]),
            &format!("{}\n→ {}", s(&job, "name"), s(&job["duplicate"], "path")),
            None,
        ),
        "review" => app.notifier.send(
            "review",
            &t("Hörbuch braucht Prüfung", &[]),
            &format!("{}\n{}", s(&job, "name"), s(&job, "message")),
            None,
        ),
        "failed" => app.notifier.send(
            "failed",
            &t("Fehler bei einem Hörbuch", &[]),
            format!("{}\n{}", s(&job, "name"), s(&job, "error")).trim(),
            Some(4),
        ),
        _ => {}
    }
}

/// Sorted like the inbox listing (natural order of the relative path).
pub fn sort_paths(paths: &mut [String]) {
    paths.sort_by(|a, b| natural_path_cmp(a, b));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_numbers_that_contradict_the_file_order() {
        crate::text::set_language("de");
        let info = |track: &str, disc: &str| json!({"tags": {"track": track, "disc": disc}});
        let files = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // counting up, also "3/12": fine
        assert_eq!(
            order_doubt(&files(&["01.mp3", "02.mp3"]), &[info("1/12", ""), info("2/12", "")]),
            None
        );
        // CD folders start again at 1: fine
        let cds = files(&["CD1/01.mp3", "CD1/02.mp3", "CD2/01.mp3"]);
        assert_eq!(order_doubt(&cds, &[info("1", ""), info("2", ""), info("1", "")]), None);
        // one folder, discs in the tags: fine
        let one = files(&["a.mp3", "b.mp3"]);
        assert_eq!(order_doubt(&one, &[info("9", "1"), info("1", "2")]), None);
        // badly named: the second file is track 1
        let bad = order_doubt(
            &files(&["Kapitel 10.mp3", "Kapitel 2.mp3"]),
            &[info("2", ""), info("1", "")],
        );
        assert!(bad.unwrap().contains("Kapitel 2.mp3"));
        // without track numbers nothing can be said
        assert_eq!(order_doubt(&one, &[json!({"tags": {}}), info("1", "")]), None);
    }
}
