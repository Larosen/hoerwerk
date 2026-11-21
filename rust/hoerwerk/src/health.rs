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

/// The findings a person (or the tool, when it is sure there is nothing to do) can settle for a book: what is
/// settled is not shown, not offered for tidying up and not touched by the automatic matching.
pub const SETTLEABLE: [&str; 5] = ["no_asin", "runtime_off", "no_cover", "no_year", "no_narrator"];
const SETTLED_KEY: &str = "library_settled";
/// What the tool settled by itself (nothing sure to add) is looked at again after this long – new releases appear;
/// what an admin or an uploaded cover settled stays.
const AUTO_SETTLED_FOR: f64 = 14.0 * 86400.0;

/// A decision about a book: which findings are settled, by whom ("admin", "cover", "auto") and when.
#[derive(Clone)]
struct Settled {
    codes: Vec<String>,
    why: String,
    at: f64,
}

#[derive(Clone)]
struct Item {
    mtime: f64,
    score: i64,
    issues: Vec<String>,
    asin: String,
    /// length of the M4B in seconds (0 = unknown)
    duration: f64,
    /// identified as an edition outside Audible (a CD): "no Audible link" is no finding for it
    other: bool,
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
    fn effective(&self, runtimes: &HashMap<String, i64>, settled: &[String]) -> (i64, Vec<String>) {
        let mut issues = self.issues.clone();
        if self.other {
            issues.retain(|i| i != "no_asin");
        }
        if runtimes.get(&self.asin).is_some_and(|m| runtime_off(self.duration, *m)) {
            issues.push("runtime_off".to_string());
        }
        issues.retain(|i| !settled.contains(i));
        (score_of(&issues), issues)
    }
}

pub struct Health {
    items: Mutex<HashMap<String, Item>>,
    settled: Mutex<HashMap<String, Settled>>,
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
                        other: v["o"].as_bool().unwrap_or(false),
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
        let mut settled = HashMap::new();
        if let Some(map) = db.get(SETTLED_KEY).as_ref().and_then(Value::as_object) {
            for (path, v) in map {
                settled.insert(
                    path.clone(),
                    Settled {
                        codes: v["c"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect(),
                        why: v["w"].as_str().unwrap_or("admin").to_string(),
                        at: v["t"].as_f64().unwrap_or(0.0),
                    },
                );
            }
        }
        Health {
            items: Mutex::new(items),
            settled: Mutex::new(settled),
            running: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            progress: Mutex::new((0, 0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Item>> {
        self.items.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_settled(&self) -> std::sync::MutexGuard<'_, HashMap<String, Settled>> {
        self.settled.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The findings that are settled for a book right now (what the tool settled by itself runs out after a while).
    fn settled_codes(&self, rel: &str) -> Vec<String> {
        match self.lock_settled().get(rel) {
            Some(s) if s.why != "auto" || text::now() - s.at < AUTO_SETTLED_FOR => s.codes.clone(),
            _ => Vec::new(),
        }
    }

    /// The books a person decided about.
    pub fn settled_by_hand(&self) -> std::collections::HashSet<String> {
        self.lock_settled()
            .iter()
            .filter(|(_, s)| s.why != "auto" && !s.codes.is_empty())
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Settle findings of a book. `why`: "admin" (said so), "cover" (uploaded an own cover: the book is the
    /// admin's own work) or "auto" (the tool looked and has nothing sure to add). A decision by hand is never
    /// replaced by an automatic one, only widened.
    pub fn settle(&self, db: &Db, rel: &str, codes: &[&str], why: &str) {
        {
            let mut all = self.lock_settled();
            let entry = all.entry(rel.to_string()).or_insert_with(|| Settled {
                codes: Vec::new(),
                why: why.to_string(),
                at: text::now(),
            });
            for c in codes {
                if !entry.codes.iter().any(|x| x == c) {
                    entry.codes.push((*c).to_string());
                }
            }
            if why != "auto" || entry.why == "auto" {
                entry.why = if why == "auto" {
                    entry.why.clone()
                } else {
                    why.to_string()
                };
            }
            entry.at = text::now();
        }
        self.save_settled(db);
    }

    /// The book is to be looked at again like any other (an admin took the decision back).
    pub fn unsettle(&self, db: &Db, rel: &str) {
        self.lock_settled().remove(rel);
        self.save_settled(db);
    }

    fn save_settled(&self, db: &Db) {
        let map: serde_json::Map<String, Value> = self
            .lock_settled()
            .iter()
            .map(|(p, s)| (p.clone(), json!({"c": s.codes, "w": s.why, "t": s.at})))
            .collect();
        let _ = db.put(SETTLED_KEY, &Value::Object(map));
    }

    /// Books whose Audible length contradicts the file and that nobody settled: the file is probably another work
    /// than the linked edition.
    pub fn mismatched(&self, runtimes: &HashMap<String, i64>) -> Vec<String> {
        let mut out: Vec<String> = self
            .lock()
            .iter()
            .filter(|(p, i)| {
                i.effective(runtimes, &self.settled_codes(p))
                    .1
                    .iter()
                    .any(|c| c == "runtime_off")
            })
            .map(|(p, _)| p.clone())
            .collect();
        out.sort();
        out
    }

    /// path -> (score, findings) of the books looked at so far; `runtimes` (asin -> Audible's minutes) adds the
    /// online finding where Audible's data is known.
    pub fn snapshot(&self, runtimes: &HashMap<String, i64>) -> HashMap<String, (i64, Vec<String>)> {
        self.lock()
            .iter()
            .map(|(p, i)| (p.clone(), i.effective(runtimes, &self.settled_codes(p))))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Look at one book again now (after its chapters were renamed) so the library shows the new condition.
    pub fn recheck(&self, app: &App, rel: &str) {
        if let Some(entry) = app.library.entry(rel) {
            let mut item = check_entry(app, &entry);
            let mut items = self.lock();
            item.other = items.get(rel).is_some_and(|old| old.other);
            items.insert(rel.to_string(), item);
            drop(items);
            self.save(&app.db);
        }
    }

    /// The book was identified as an edition outside Audible (a CD): "no Audible link" is no finding for it any more.
    pub fn mark_other(&self, app: &App, rel: &str) {
        if let Some(item) = self.lock().get_mut(rel) {
            item.other = true;
        }
        self.save(&app.db);
    }

    /// The books whose last look found something the library can try to settle by itself: no Audible link, no cover,
    /// no year, no narrator (not the ones identified as an edition outside Audible).
    pub fn needing_help(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .lock()
            .iter()
            .filter(|(p, i)| {
                let settled = self.settled_codes(p);
                i.issues
                    .iter()
                    .filter(|c| !settled.contains(c))
                    .any(|c| match c.as_str() {
                        "no_asin" => !i.other,
                        "no_cover" | "no_year" | "no_narrator" => true,
                        _ => false,
                    })
            })
            .map(|(p, _)| p.clone())
            .collect();
        out.sort();
        out
    }

    fn save(&self, db: &Db) {
        let items: serde_json::Map<String, Value> = self
            .lock()
            .iter()
            .map(|(p, i)| {
                (
                    p.clone(),
                    json!({"m": i.mtime, "s": i.score, "i": i.issues, "a": i.asin, "d": i.duration, "o": i.other}),
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
        for (p, i) in items.iter().filter(|(p, _)| known.contains(*p)) {
            let settled = self.settled_codes(p);
            let (_, issues) = i.effective(&runtimes, &settled);
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
               "books": known.len(), "tiles": tiles, "autofix": app.autofix.status()})
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
        // now it is known what is missing: the library tries to settle that by itself
        app.autofix.start(app);
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
        other: false,
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
    fn a_cd_identified_outside_audible_has_no_link_to_wait_for() {
        let mut item = Item {
            mtime: 0.0,
            score: 92,
            issues: vec!["no_asin".into(), "no_cover".into()],
            asin: String::new(),
            duration: 3600.0,
            other: false,
        };
        assert_eq!(item.effective(&HashMap::new(), &[]).1, ["no_asin", "no_cover"]);
        item.other = true; // found at the national library or Apple Music: not an Audible book
        assert_eq!(item.effective(&HashMap::new(), &[]).1, ["no_cover"]);
    }

    #[test]
    fn what_is_settled_is_not_a_finding_any_more() {
        let item = Item {
            mtime: 0.0,
            score: 60,
            issues: vec!["no_asin".into(), "no_cover".into(), "no_year".into(), "chapters".into()],
            asin: String::new(),
            duration: 3600.0,
            other: false,
        };
        // "fine as it is": the match and the details are settled, the chapters are another task
        let settled: Vec<String> = SETTLEABLE.iter().map(|c| c.to_string()).collect();
        assert_eq!(item.effective(&HashMap::new(), &settled).1, ["chapters"]);
        // a length that contradicts Audible is settled as well
        let mut runtimes = HashMap::new();
        let with_asin = Item {
            asin: "B0X".into(),
            issues: vec![],
            ..item.clone()
        };
        runtimes.insert("B0X".to_string(), 600);
        assert_eq!(with_asin.effective(&runtimes, &[]).1, ["runtime_off"]);
        assert!(with_asin.effective(&runtimes, &settled).1.is_empty());
    }

    #[test]
    fn a_decision_by_hand_stays_and_one_by_the_tool_runs_out() {
        let db = Db::memory();
        let health = Health::new(&db);
        health.settle(&db, "A/Buch", &["no_asin"], "auto");
        assert_eq!(health.settled_codes("A/Buch"), ["no_asin"]);
        assert!(!health.settled_by_hand().contains("A/Buch"));
        // the tool's own decision runs out after two weeks
        health.lock_settled().get_mut("A/Buch").unwrap().at -= 15.0 * 86400.0;
        assert!(health.settled_codes("A/Buch").is_empty());
        // an admin's stays, and the tool cannot take it over or shorten it
        health.settle(&db, "A/Buch", &["no_asin", "no_year"], "admin");
        health.settle(&db, "A/Buch", &["no_narrator"], "auto");
        health.lock_settled().get_mut("A/Buch").unwrap().at -= 400.0 * 86400.0;
        assert_eq!(health.settled_codes("A/Buch"), ["no_asin", "no_year", "no_narrator"]);
        assert!(health.settled_by_hand().contains("A/Buch"));
        // it survives a restart
        assert!(Health::new(&db).settled_by_hand().contains("A/Buch"));
        health.unsettle(&db, "A/Buch");
        assert!(health.settled_codes("A/Buch").is_empty());
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
            other: false,
        };
        let runtimes: HashMap<String, i64> = [("B1".to_string(), 600)].into();
        assert_eq!(item.effective(&runtimes, &[]), (75, vec!["runtime_off".to_string()]));
        assert_eq!(item.effective(&HashMap::new(), &[]), (100, vec![]));
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
