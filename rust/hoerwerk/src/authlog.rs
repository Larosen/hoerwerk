//! Security log (authlog.py): one logfmt line per login event, for people and for CrowdSec.
//!
//!     time=2026-10-31T17:02:03Z app=hoerwerk event=login_failed user="max" ip=203.0.113.7 reason=bad_password
//!
//! The lines go to `/config/logs/auth.log` (rotated at 10 MB, 5 files kept) and to stdout. Values are quoted and
//! stripped of control characters – a user name can't forge a second line.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::rx;
use crate::text::{self, now};

const MAX_BYTES: u64 = 10 << 20;
const KEEP: usize = 5;

static FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn value(v: &str) -> String {
    let text: String = v
        .chars()
        .map(|c| if (c as u32) < 0x20 || c as u32 == 0x7f { '?' } else { c })
        .take(200)
        .collect();
    if rx::is_match(rx!(r"^[A-Za-z0-9_.:/@-]{1,80}$"), &text) {
        text
    } else {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

pub fn line(event: &str, ts: f64, fields: &[(&str, &str)]) -> String {
    let secs = ts as i64;
    let (y, m, d) = text::civil(secs.div_euclid(86400));
    let rest = secs.rem_euclid(86400);
    let mut parts = vec![
        format!(
            "time={y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            rest / 3600,
            rest % 3600 / 60,
            rest % 60
        ),
        "app=hoerwerk".into(),
        format!("event={event}"),
    ];
    parts.extend(
        fields
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{k}={}", value(v))),
    );
    parts.join(" ")
}

/// Open the log file (once). Without a writable folder only stdout is used.
pub fn setup(config_dir: &std::path::Path) {
    let folder = config_dir.join("logs");
    match std::fs::create_dir_all(&folder) {
        Ok(()) => *FILE.lock().unwrap_or_else(|p| p.into_inner()) = Some(folder.join("auth.log")),
        Err(e) => text::warn("Anmeldung", &format!("auth.log nicht beschreibbar: {e}")),
    }
}

fn rotate(path: &std::path::Path) {
    if std::fs::metadata(path).map_or(0, |m| m.len()) < MAX_BYTES {
        return;
    }
    let name = |n: usize| PathBuf::from(format!("{}.{n}", path.display()));
    let _ = std::fs::remove_file(name(KEEP));
    for n in (1..KEEP).rev() {
        let _ = std::fs::rename(name(n), name(n + 1));
    }
    let _ = std::fs::rename(path, name(1));
}

/// Write one event; returns the line.
pub fn event(name: &str, fields: &[(&str, &str)]) -> String {
    let text = line(name, now(), fields);
    if let Some(path) = FILE.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        rotate(path);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{text}");
        }
    }
    println!("{text}");
    let level = if matches!(name, "login_failed" | "login_blocked" | "forbidden") {
        "warn"
    } else {
        "info"
    };
    text::log_entry(level, "Anmeldung", &text, false);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_cannot_be_forged() {
        let l = line(
            "login_failed",
            1_793_811_723.0,
            &[
                ("user", "max\nevent=login_ok"),
                ("ip", "203.0.113.7"),
                ("reason", "bad_password"),
                ("empty", ""),
            ],
        );
        assert_eq!(
            l,
            "time=2026-11-04T17:02:03Z app=hoerwerk event=login_failed user=\"max?event=login_ok\" ip=203.0.113.7 reason=bad_password"
        );
        assert_eq!(value("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
