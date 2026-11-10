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
const LEVELS: [&str; 3] = ["info", "warn", "error"];
const SIZE: usize = 3000;

struct Log {
    seq: u64,
    entries: VecDeque<Value>,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    seq: 0,
    entries: VecDeque::new(),
});

pub fn log(level: &str, source: &str, text: &str) {
    log_entry(level, source, text, true);
}

/// Into the log the app shows; `echo` = false when the line is printed elsewhere already.
pub fn log_entry(level: &str, source: &str, text: &str, echo: bool) {
    let level = if LEVELS.contains(&level) { level } else { "info" };
    let time = now();
    {
        let mut l = LOG.lock().unwrap_or_else(|p| p.into_inner());
        l.seq += 1;
        let entry = json!({"id": l.seq, "time": time, "level": level, "source": source, "text": text.trim_end()});
        l.entries.push_back(entry);
        while l.entries.len() > SIZE {
            l.entries.pop_front();
        }
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
    fn log_levels() {
        info("t", "eins");
        error("t", "zwei");
        let (errors, last) = since(0, "error", 100);
        assert!(errors.iter().any(|e| e["text"] == "zwei"));
        assert!(errors.iter().all(|e| e["level"] == "error"));
        assert!(last >= 2);
    }
}
