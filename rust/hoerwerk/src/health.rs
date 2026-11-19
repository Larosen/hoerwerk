//! The library check: every book of the library is looked at once (locally – no request leaves the server) and
//! gets a condition score and a list of what could be better: chapter names that repeat the title or say nothing,
//! no chapters at all, no year, no cover, no narrator, no Audible match, not one M4B yet. The result is kept (a
//! book is looked at again only when its files changed), shown in the library, sortable by condition, and every
//! finding leads to the dialog that fixes it. Nothing is changed by the check itself.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::app::App;
use crate::db::Db;
use crate::encoder::{self, Chapter};
use crate::media;
use crate::pipeline::Pipeline;
use crate::text::{self, t};

const KEY: &str = "library_check";
/// bump when the rules change: every book is looked at again
const VERSION: i64 = 2;
/// A book longer than this without chapters is hard to navigate.
const LONG_BOOK: f64 = 1800.0;

/// (code, points lost) – the score is 100 minus these.
const WEIGHTS: [(&str, i64); 9] = [
    ("chapters", 35),
    ("runtime_off", 25),
    ("chapters_part", 12),
    ("no_chapters", 15),
    ("no_cover", 15),
    ("no_year", 10),
    ("not_m4b", 10),
    ("no_asin", 8),
    ("no_narrator", 5),
];

/// The tidy-up tasks the library offers: a finding belongs to at most one. ("lowrated" is not a finding: it comes
/// from Audible's ratings.)
pub fn group_of(code: &str) -> Option<&'static str> {
    match code {
        "chapters" | "chapters_part" | "no_chapters" => Some("chapters"),
        "no_cover" | "no_year" | "no_narrator" => Some("details"),
        "no_asin" | "runtime_off" => Some("match"),
        _ => None,
    }
}

/// Has the book a finding of this task?
pub fn in_group(issues: &[String], group: &str) -> bool {
    issues.iter().any(|i| group_of(i) == Some(group))
}

pub fn score_of(issues: &[String]) -> i64 {
    let lost: i64 = issues
        .iter()
        .filter_map(|i| WEIGHTS.iter().find(|(c, _)| c == i).map(|(_, w)| *w))
        .sum();
    (100 - lost).max(0)
}

/// What is wrong with one book. `chapters` are its M4B chapters (empty for other formats), `duration` its length
/// in seconds. Pure: the tests feed it directly.
pub fn judge(entry: &Value, book: &Value, chapters: &[Chapter], duration: f64, name: &str) -> Vec<String> {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    let mut out: Vec<&str> = Vec::new();
    let m4b = s(entry, "format") == "m4b";
    if !m4b {
        out.push("not_m4b");
    }
    if entry["cover"] != true {
        out.push("no_cover");
    }
    if entry["asin"].is_null() || s(entry, "asin").is_empty() {
        out.push("no_asin");
    }
    if m4b {
        if s(book, "release_date").is_empty() {
            out.push("no_year");
        }
        if entry["narrators"].as_array().is_none_or(Vec::is_empty) {
            out.push("no_narrator");
        }
        if chapters.len() < 2 {
            if duration > LONG_BOOK {
                out.push("no_chapters");
            }
        } else {
            let titles: Vec<&str> = chapters.iter().map(|c| c.title.as_str()).collect();
            let lang = Pipeline::chapter_lang(entry, name);
            let cleaned = crate::chapters::clean_titles(&titles, &Pipeline::known_names(entry), &lang);
            let changed = cleaned
                .names
                .iter()
                .zip(&titles)
                .filter(|(a, b)| a.as_str() != **b)
                .count();
            if changed * 2 >= titles.len() {
                out.push("chapters");
            } else if changed > 0 {
                out.push("chapters_part");
            }
        }
    }
    out.into_iter().map(str::to_string).collect()
}

#[derive(Clone)]
struct Item {
    mtime: f64,
    score: i64,
    issues: Vec<String>,
    asin: String,
    /// length of the M4B in seconds (0 = unknown)
    duration: f64,
}

/// Audible lists another length than the file has: more than 8 % and 5 minutes apart – most likely another
/// recording or an abridged/unabridged mix-up. Audible's own figures are rounded, so small gaps are normal.
pub fn runtime_off(duration_s: f64, audible_min: i64) -> bool {
    let rt = audible_min as f64 * 60.0;
    duration_s > 0.0 && rt > 0.0 && (duration_s - rt).abs() > (rt * 0.08).max(300.0)
}

impl Item {
    /// Score and findings including the online one: the length compared with Audible's (known once the Audible
    /// data of the book was fetched).
    fn effective(&self, runtimes: &HashMap<String, i64>) -> (i64, Vec<String>) {
        let mut issues = self.issues.clone();
        if runtimes.get(&self.asin).is_some_and(|m| runtime_off(self.duration, *m)) {
            issues.push("runtime_off".to_string());
        }
        (score_of(&issues), issues)
    }
}

pub struct Health {
    items: Mutex<HashMap<String, Item>>,
    running: AtomicBool,
    stop: AtomicBool,
    progress: Mutex<(usize, usize)>,
}

impl Health {
    pub fn new(db: &Db) -> Self {
        let mut items = HashMap::new();
        if let Some(d) = db.get(KEY)
            && d["v"] == VERSION
            && let Some(map) = d["items"].as_object()
        {
            for (path, v) in map {
                items.insert(
                    path.clone(),
                    Item {
                        mtime: v["m"].as_f64().unwrap_or(0.0),
                        score: v["s"].as_i64().unwrap_or(0),
                        asin: v["a"].as_str().unwrap_or("").to_string(),
                        duration: v["d"].as_f64().unwrap_or(0.0),
                        issues: v["i"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect(),
                    },
                );
            }
        }
        Health {
            items: Mutex::new(items),
            running: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            progress: Mutex::new((0, 0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Item>> {
        self.items.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// path -> (score, findings) of the books looked at so far; `runtimes` (asin -> Audible's minutes) adds the
    /// online finding where Audible's data is known.
    pub fn snapshot(&self, runtimes: &HashMap<String, i64>) -> HashMap<String, (i64, Vec<String>)> {
        self.lock()
            .iter()
            .map(|(p, i)| (p.clone(), i.effective(runtimes)))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Look at one book again now (after its chapters were renamed) so the library shows the new condition.
    pub fn recheck(&self, app: &App, rel: &str) {
        if let Some(entry) = app.library.entry(rel) {
            let item = check_entry(app, &entry);
            self.lock().insert(rel.to_string(), item);
            self.save(&app.db);
        }
    }

    fn save(&self, db: &Db) {
        let items: serde_json::Map<String, Value> = self
            .lock()
            .iter()
            .map(|(p, i)| {
                (
                    p.clone(),
                    json!({"m": i.mtime, "s": i.score, "i": i.issues, "a": i.asin, "d": i.duration}),
                )
            })
            .collect();
        let _ = db.put(KEY, &json!({"v": VERSION, "items": items}));
    }

    /// Progress and how many books each tidy-up task has: chapters, details, match (from the check) and
    /// lowrated (from Audible's ratings).
    pub fn status(&self, app: &App) -> Value {
        let items = self.lock();
        let entries = app.library.entries();
        let known: std::collections::HashSet<String> = entries
            .iter()
            .filter_map(|e| e["path"].as_str().map(str::to_string))
            .collect();
        let runtimes = app.ratings.runtimes();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut n = 0;
        for (_, i) in items.iter().filter(|(p, _)| known.contains(*p)) {
            let (_, issues) = i.effective(&runtimes);
            n += 1;
            for g in ["chapters", "details", "match"] {
                if in_group(&issues, g) {
                    *counts.entry(g).or_default() += 1;
                }
            }
        }
        let rated = app.ratings.snapshot();
        let low = entries
            .iter()
            .filter(|e| {
                e["asin"]
                    .as_str()
                    .and_then(|a| rated.get(a))
                    .is_some_and(|(r, v)| crate::ratings::is_low(*r, *v))
            })
            .count();
        let (done, total) = *self.progress.lock().unwrap_or_else(|p| p.into_inner());
        let tiles: Vec<Value> = ["chapters", "details", "match"]
            .iter()
            .map(|g| json!({"key": g, "count": counts.get(g).copied().unwrap_or(0)}))
            .chain([json!({"key": "lowrated", "count": low})])
            .collect();
        json!({"running": self.running.load(Ordering::SeqCst), "done": done, "total": total, "checked": n,
               "books": known.len(), "tiles": tiles})
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Look at every book that has no result yet (or whose files changed), in the background: one at a time,
    /// with a pause in between, and never while the server is busy with new books. False: already running.
    pub fn start(self: &Arc<Self>, app: &Arc<App>) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let (me, app) = (self.clone(), app.clone());
        std::thread::Builder::new()
            .name("library-check".into())
            .spawn(move || {
                me.run(&app);
                me.running.store(false, Ordering::SeqCst);
            })
            .map(|_| true)
            .unwrap_or_else(|_| {
                self.running.store(false, Ordering::SeqCst);
                false
            })
    }

    /// The first time (no results yet) the library is looked at without being asked: that is "a library was added".
    pub fn start_if_new(self: &Arc<Self>, app: &Arc<App>) {
        if self.is_empty() && !app.library.entries().is_empty() && self.start(app) {
            text::info(
                "Bibliothek",
                &t("Bibliothek wird auf Verbesserungen geprüft (im Hintergrund)", &[]),
            );
        }
    }

    fn run(&self, app: &Arc<App>) {
        let mut todo: Vec<Value> = app
            .library
            .entries()
            .into_iter()
            .filter(|e| {
                let p = e["path"].as_str().unwrap_or("");
                !p.is_empty()
                    && self
                        .lock()
                        .get(p)
                        .is_none_or(|i| (i.mtime - e["mtime"].as_f64().unwrap_or(0.0)).abs() > 0.001)
            })
            .collect();
        todo.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = (0, todo.len());
        for (n, entry) in todo.iter().enumerate() {
            if self.stop.load(Ordering::SeqCst) || *app.shutdown.borrow() {
                break;
            }
            // new books and downloads go first
            while app.pipeline.busy() && !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(5));
            }
            let path = entry["path"].as_str().unwrap_or("").to_string();
            let item = check_entry(app, entry);
            self.lock().insert(path, item);
            *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = (n + 1, todo.len());
            if (n + 1) % 50 == 0 {
                self.save(&app.db);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        self.save(&app.db);
        let (done, total) = *self.progress.lock().unwrap_or_else(|p| p.into_inner());
        text::info(
            "Bibliothek",
            &t(
                "Bibliotheks-Check: {done} von {total} Büchern angesehen",
                &[("done", &done.to_string()), ("total", &total.to_string())],
            ),
        );
    }
}

fn check_entry(app: &App, entry: &Value) -> Item {
    let rel = entry["path"].as_str().unwrap_or("");
    let folder = app.cfg.library.join(rel);
    let (mut book, mut chapters, mut duration) = (json!({}), Vec::new(), 0.0);
    if entry["format"] == "m4b"
        && let Some(file) = media::collect_audio(&folder).first().map(|f| folder.join(f))
        && Path::new(&file).is_file()
    {
        book = media::read_book(&file);
        chapters = encoder::file_chapters(&file);
        duration = media::probe_cached(&file)["duration"].as_f64().unwrap_or(0.0);
    }
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let issues = judge(entry, &book, &chapters, duration, &name);
    Item {
        mtime: entry["mtime"].as_f64().unwrap_or(0.0),
        score: score_of(&issues),
        issues,
        asin: entry["asin"].as_str().unwrap_or("").to_string(),
        duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(titles: &[&str]) -> Vec<Chapter> {
        titles
            .iter()
            .enumerate()
            .map(|(i, t)| Chapter {
                start: i as f64 * 600.0,
                title: t.to_string(),
            })
            .collect()
    }

    fn entry() -> Value {
        json!({"format": "m4b", "cover": true, "asin": "B004V7TW3Y", "title": "Der Federmann",
               "authors": ["Sebastian Fitzek"], "narrators": ["Simon Jäger"]})
    }

    #[test]
    fn a_clean_book_has_no_findings() {
        let book = json!({"release_date": "2016"});
        let c = ch(&["Prolog", "Der Anruf", "Mittwoch Vormittag", "Epilog"]);
        assert!(judge(&entry(), &book, &c, 9000.0, "Der Federmann").is_empty());
        assert_eq!(score_of(&[]), 100);
    }

    #[test]
    fn chapters_that_repeat_the_title_are_the_worst_finding() {
        let book = json!({"release_date": "2016"});
        let titles: Vec<String> = (1..=12).map(|i| format!("Der Federmann{i:03}")).collect();
        let c = ch(&titles.iter().map(String::as_str).collect::<Vec<_>>());
        let issues = judge(&entry(), &book, &c, 9000.0, "Der Federmann");
        assert_eq!(issues, ["chapters"]);
        assert_eq!(score_of(&issues), 65);
        // a few cluttered names among good ones are a smaller finding
        let c = ch(&[
            "Prolog",
            "Der Anruf",
            "Mittwoch Vormittag",
            "Das Finale",
            "Der Federmann005",
            "Epilog",
        ]);
        assert!(
            judge(&entry(), &book, &c, 9000.0, "Der Federmann")
                .iter()
                .all(|i| i != "chapters")
        );
    }

    #[test]
    fn another_length_than_audible_lists_is_found() {
        // 10 h file against 10 h 20 min at Audible: rounding, fine; 6 h against 10 h: another recording
        assert!(!runtime_off(36_000.0, 620));
        assert!(runtime_off(21_600.0, 600));
        // short books: five minutes of slack
        assert!(!runtime_off(4_000.0, 70));
        assert!(runtime_off(3_000.0, 70));
        // unknown on either side: no finding
        assert!(!runtime_off(0.0, 600) && !runtime_off(3_600.0, 0));
        let item = Item {
            mtime: 0.0,
            score: 100,
            issues: vec![],
            asin: "B1".into(),
            duration: 21_600.0,
        };
        let runtimes: HashMap<String, i64> = [("B1".to_string(), 600)].into();
        assert_eq!(item.effective(&runtimes), (75, vec!["runtime_off".to_string()]));
        assert_eq!(item.effective(&HashMap::new()), (100, vec![]));
    }

    #[test]
    fn missing_details_and_other_formats_are_found() {
        let e = json!({"format": "mp3", "cover": false, "asin": null, "narrators": []});
        let issues = judge(&e, &json!({}), &[], 0.0, "Alt");
        assert_eq!(issues, ["not_m4b", "no_cover", "no_asin"]);
        let issues = judge(
            &json!({"format": "m4b", "cover": true, "asin": "X", "narrators": []}),
            &json!({}),
            &[],
            7200.0,
            "Lang",
        );
        assert_eq!(issues, ["no_year", "no_narrator", "no_chapters"]);
        // a short book without chapters is fine
        assert!(
            !judge(&entry(), &json!({"release_date": "2020"}), &[], 600.0, "Kurz").contains(&"no_chapters".to_string())
        );
        assert!(score_of(&["no_cover".into(), "no_year".into(), "chapters".into()]) < 50);
    }
}
