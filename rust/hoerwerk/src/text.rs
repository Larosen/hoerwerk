//! Texts: German is the source, static/i18n/en.json the English catalog (the same file the browser uses),
//! and the in-app log (/api/logs) – a ring buffer of the last entries, also printed to stdout.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
static LANGUAGE: RwLock<String> = RwLock::new(String::new());

pub fn load_catalog(static_dir: &Path) {
    let map = std::fs::read_to_string(static_dir.join("i18n").join("en.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
        .unwrap_or_default();
    let _ = CATALOG.set(map);
}

pub fn set_language(lang: &str) {
    *LANGUAGE.write().unwrap_or_else(|p| p.into_inner()) = if lang == "en" { "en".into() } else { "de".into() };
}

pub fn language() -> String {
    let l = LANGUAGE.read().unwrap_or_else(|p| p.into_inner()).clone();
    if l.is_empty() { "de".into() } else { l }
}

pub fn catalog_json() -> Value {
    if language() == "en" {
        serde_json::to_value(CATALOG.get().cloned().unwrap_or_default()).unwrap_or(json!({}))
    } else {
        json!({})
    }
}

/// Translate a German text; ``{name}`` placeholders are filled from ``values``.
pub fn t(text: &str, values: &[(&str, &str)]) -> String {
    let mut out = if language() == "en" {
        CATALOG
            .get()
            .and_then(|c| c.get(text))
            .cloned()
            .unwrap_or_else(|| text.to_string())
    } else {
        text.to_string()
    };
    for (k, v) in values {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// -- in-app log ---------------------------------------------------------------------
const LEVELS: [&str; 4] = ["debug", "info", "warn", "error"];
const SIZE: usize = 5000;

/// Debug lines are only recorded while switched on (Protokoll page, setting log_debug).
static DEBUG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_debug(on: bool) {
    DEBUG.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn debug_on() -> bool {
    DEBUG.load(std::sync::atomic::Ordering::Relaxed)
}

/// A line for the debug log: costs nothing while debugging is off.
pub fn debug(source: &str, text: &str) {
    if debug_on() {
        log("debug", source, text);
    }
}

struct Log {
    seq: u64,
    entries: VecDeque<Value>,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    seq: 0,
    entries: VecDeque::new(),
});

/// The log also goes to config/logs/hoerwerk.log (one JSON object per line), so a restart or an update keeps it.
static LOG_FILE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
const LOG_MAX_BYTES: u64 = 5 << 20;

/// Open the log file and bring back the newest entries of the earlier runs.
pub fn setup_log(config_dir: &std::path::Path) {
    let folder = config_dir.join("logs");
    if std::fs::create_dir_all(&folder).is_err() {
        return;
    }
    let path = folder.join("hoerwerk.log");
    let mut old: VecDeque<Value> = VecDeque::new();
    for file in [folder.join("hoerwerk.log.1"), path.clone()] {
        for line in std::fs::read_to_string(&file).unwrap_or_default().lines() {
            if let Ok(e) = serde_json::from_str::<Value>(line)
                && e.get("time").is_some()
            {
                old.push_back(e);
                if old.len() > SIZE {
                    old.pop_front();
                }
            }
        }
    }
    let mut l = LOG.lock().unwrap_or_else(|p| p.into_inner());
    let earlier = std::mem::take(&mut l.entries); // lines written before the file was known (the start itself)
    l.seq = 0;
    for mut e in old.into_iter().chain(earlier) {
        l.seq += 1;
        e["id"] = json!(l.seq);
        l.entries.push_back(e);
    }
    while l.entries.len() > SIZE {
        l.entries.pop_front();
    }
    *LOG_FILE.lock().unwrap_or_else(|p| p.into_inner()) = Some(path);
}

fn append_log(entry: &Value) {
    use std::io::Write;
    let guard = LOG_FILE.lock().unwrap_or_else(|p| p.into_inner());
    let Some(path) = guard.as_ref() else { return };
    if std::fs::metadata(path).map_or(0, |m| m.len()) > LOG_MAX_BYTES {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let mut e = entry.clone();
        if let Some(o) = e.as_object_mut() {
            o.remove("id");
        }
        let _ = writeln!(f, "{e}");
    }
}

/// A text that was read as Windows-1252 although it is UTF-8 ("Philosopherâ€™s") gets its characters back; anything
/// else is returned as it is.
pub fn fix_mojibake(text: &str) -> String {
    if !text.contains(['â', 'Ã', 'Â']) {
        return text.to_string();
    }
    const CP1252: [(char, u8); 27] = [
        ('€', 0x80),
        ('‚', 0x82),
        ('ƒ', 0x83),
        ('„', 0x84),
        ('…', 0x85),
        ('†', 0x86),
        ('‡', 0x87),
        ('ˆ', 0x88),
        ('‰', 0x89),
        ('Š', 0x8a),
        ('‹', 0x8b),
        ('Œ', 0x8c),
        ('Ž', 0x8e),
        ('‘', 0x91),
        ('’', 0x92),
        ('“', 0x93),
        ('”', 0x94),
        ('•', 0x95),
        ('–', 0x96),
        ('—', 0x97),
        ('˜', 0x98),
        ('™', 0x99),
        ('š', 0x9a),
        ('›', 0x9b),
        ('œ', 0x9c),
        ('ž', 0x9e),
        ('Ÿ', 0x9f),
    ];
    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        let cp = c as u32;
        if cp < 0x100 {
            bytes.push(cp as u8);
        } else if let Some((_, b)) = CP1252.iter().find(|(x, _)| *x == c) {
            bytes.push(*b);
        } else {
            return text.to_string(); // a character that cannot come from that reading: leave it
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| text.to_string())
}

/// The log itself is locked right now (for the watchdog – which then must not log).
pub fn log_busy() -> bool {
    matches!(LOG.try_lock(), Err(std::sync::TryLockError::WouldBlock))
        || matches!(LOG_FILE.try_lock(), Err(std::sync::TryLockError::WouldBlock))
}

pub fn log(level: &str, source: &str, text: &str) {
    log_entry(level, source, text, true);
}

/// Into the log the app shows; `echo` = false when the line is printed elsewhere already.
pub fn log_entry(level: &str, source: &str, text: &str, echo: bool) {
    use unicode_normalization::UnicodeNormalization;
    let level = if LEVELS.contains(&level) { level } else { "info" };
    let time = now();
    // file names from macOS-style folders arrive decomposed (u + ¨): shown composed
    let (source, text): (String, String) = (source.nfc().collect(), text.nfc().collect());
    let (source, text) = (source.as_str(), text.as_str());
    {
        let mut l = LOG.lock().unwrap_or_else(|p| p.into_inner());
        l.seq += 1;
        let entry = json!({"id": l.seq, "time": time, "level": level, "source": source, "text": text.trim_end()});
        l.entries.push_back(entry.clone());
        while l.entries.len() > SIZE {
            l.entries.pop_front();
        }
        drop(l);
        append_log(&entry);
    }
    if !echo {
        return;
    }
    let tag = if source.is_empty() {
        String::new()
    } else {
        format!(" [{source}]")
    };
    println!("{} {:5}{tag} {}", stamp(time), level.to_uppercase(), text.trim_end());
}

pub fn info(source: &str, text: &str) {
    log("info", source, text);
}

pub fn warn(source: &str, text: &str) {
    log("warn", source, text);
}

pub fn error(source: &str, text: &str) {
    log("error", source, text);
}

/// Entries newer than ``after`` with at least ``level``; and the id of the newest entry.
pub fn since(after: u64, level: &str, limit: usize) -> (Vec<Value>, u64) {
    let floor = LEVELS.iter().position(|l| *l == level).unwrap_or(0);
    let l = LOG.lock().unwrap_or_else(|p| p.into_inner());
    let out: Vec<Value> = l
        .entries
        .iter()
        .filter(|e| e["id"].as_u64().unwrap_or(0) > after)
        .filter(|e| LEVELS.iter().position(|x| Some(*x) == e["level"].as_str()).unwrap_or(0) >= floor)
        .cloned()
        .collect();
    let skip = out.len().saturating_sub(limit);
    (out[skip..].to_vec(), l.seq)
}

/// "YYYY-MM-DD" today, local time.
pub fn today() -> String {
    stamp(now()).chars().take(10).collect()
}

/// "YYYY-MM-DD HH:MM:SS" in the container's local time (TZ), like the Python log lines.
fn stamp(t: f64) -> String {
    let tm = local(t);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Days since 1970-01-01 -> (year, month, day) (Howard Hinnant's algorithm, UTC).
pub fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (year, month, day) -> days since 1970-01-01 (inverse of `civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Local time of the container (TZ), like Python's datetime.fromtimestamp.
pub fn local(ts: f64) -> libc::tm {
    let t = ts.floor() as libc::time_t;
    // SAFETY: localtime_r only writes into the struct we own
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

/// "YYYY-MM" of a moment in local time.
pub fn local_month(ts: f64) -> (i32, u32) {
    let tm = local(ts);
    (tm.tm_year + 1900, tm.tm_mon as u32 + 1)
}

/// The first moment of the local calendar month of `ts`.
pub fn local_month_start(ts: f64) -> f64 {
    let mut tm = local(ts);
    tm.tm_mday = 1;
    tm.tm_hour = 0;
    tm.tm_min = 0;
    tm.tm_sec = 0;
    tm.tm_isdst = -1;
    // SAFETY: mktime reads and normalises the struct we own
    unsafe { libc::mktime(&mut tm) as f64 }
}

/// "HH:MM" in local time.
/// The next local midnight after `ts` (when indexers reset their daily limit).
pub fn next_local_midnight(ts: f64) -> f64 {
    let tm = local(ts);
    ts.floor() - (tm.tm_hour as f64 * 3600.0 + tm.tm_min as f64 * 60.0 + tm.tm_sec as f64) + 86400.0
}

/// "10.11.2026" in local time.
pub fn local_date(ts: f64) -> String {
    let tm = local(ts);
    format!("{:02}.{:02}.{:04}", tm.tm_mday, tm.tm_mon + 1, tm.tm_year + 1900)
}

pub fn local_hm(ts: f64) -> String {
    let tm = local(ts);
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_and_dates() {
        assert_eq!(t("Hallo {name}", &[("name", "Eva")]), "Hallo Eva");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_000), (2024, 10, 4));
    }

    #[test]
    fn mojibake_is_repaired_and_clean_text_kept() {
        assert_eq!(
            fix_mojibake("Harry Potter and the Philosopherâ€™s Stone"),
            "Harry Potter and the Philosopher’s Stone"
        );
        assert_eq!(fix_mojibake("GrÃ¶ÃŸe"), "Größe");
        assert_eq!(fix_mojibake("Größe – Ärger"), "Größe – Ärger");
        assert_eq!(fix_mojibake("Â"), "Â");
    }

    #[test]
    fn log_levels() {
        info("t", "eins");
        error("t", "zwei");
        let (errors, last) = since(0, "error", 100);
        assert!(errors.iter().any(|e| e["text"] == "zwei"));
        assert!(errors.iter().all(|e| e["level"] == "error"));
        assert!(last >= 2);
    }
}
