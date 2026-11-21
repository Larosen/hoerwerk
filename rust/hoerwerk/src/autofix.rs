//! The library tidies itself: books without an Audible link are looked up in the background, and where the edition
//! is beyond doubt its ASIN and the missing details (year, narrators, publisher, series, description, cover) are
//! written into the M4B – the same as "Daten ergänzen", only without anybody asking. What could not be settled stays
//! on the tidy-up tiles: that is the short list a person has to look at. A book that was looked at without success is
//! looked at again only when its files changed or after two weeks (new releases appear).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::app::App;
use crate::db::Db;
use crate::enrich::{self, Outcome};
use crate::text::{self, t};

const KEY: &str = "library_autofix";
const RETRY: f64 = 14.0 * 86400.0;
const PAUSE: Duration = Duration::from_millis(1500);
/// Audible and the other sources not answering: this many failures in a row end the run.
const MAX_FAILS: usize = 5;
/// Links corrected (looked up online) per round.
const LINKS_PER_ROUND: usize = 10;
/// Rewriting the tags of a big M4B can mean copying the whole file: so many books per round, then a rest.
const BATCH: usize = 40;
const REST: Duration = Duration::from_secs(180);

#[derive(Clone, Copy)]
struct Tried {
    mtime: f64,
    at: f64,
}

pub struct Autofix {
    tried: Mutex<HashMap<String, Tried>>,
    running: AtomicBool,
    stop: AtomicBool,
    /// (looked at, to look at, settled)
    progress: Mutex<(usize, usize, usize)>,
}

impl Autofix {
    pub fn new(db: &Db) -> Self {
        let mut tried = HashMap::new();
        if let Some(d) = db.get(KEY).as_ref().and_then(Value::as_object) {
            for (path, v) in d {
                tried.insert(
                    path.clone(),
                    Tried {
                        mtime: v["m"].as_f64().unwrap_or(0.0),
                        at: v["t"].as_f64().unwrap_or(0.0),
                    },
                );
            }
        }
        Autofix {
            tried: Mutex::new(tried),
            running: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            progress: Mutex::new((0, 0, 0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Tried>> {
        self.tried.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn status(&self) -> Value {
        let (done, total, settled) = *self.progress.lock().unwrap_or_else(|p| p.into_inner());
        json!({"running": self.running.load(Ordering::SeqCst), "done": done, "total": total, "settled": settled})
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn save(&self, db: &Db) {
        let map: serde_json::Map<String, Value> = self
            .lock()
            .iter()
            .map(|(p, i)| (p.clone(), json!({"m": i.mtime, "t": i.at})))
            .collect();
        let _ = db.put(KEY, &Value::Object(map));
    }

    /// Look at the books that need it, in the background, slowly. False: switched off, or already running.
    pub fn start(self: &Arc<Self>, app: &Arc<App>) -> bool {
        let on = app
            .settings()
            .get("library_autofix")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !on || self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let (me, app) = (self.clone(), app.clone());
        std::thread::Builder::new()
            .name("library-autofix".into())
            .spawn(move || {
                // round after round until nothing is left to look at (a rest in between, new books go first)
                while me.run(&app) {
                    std::thread::sleep(REST);
                    if me.stop.load(Ordering::SeqCst) || *app.shutdown.borrow() {
                        break;
                    }
                }
                me.running.store(false, Ordering::SeqCst);
            })
            .map(|_| true)
            .unwrap_or_else(|_| {
                self.running.store(false, Ordering::SeqCst);
                false
            })
    }

    /// Books linked to an Audible edition whose length contradicts the file (probably another work): where another
    /// edition is beyond doubt (title fits, length agrees) the link is corrected and the change is written to the
    /// log and the admin trail, so an admin can look at it. Anything less sure is left alone. A few per round.
    fn correct_links(&self, app: &Arc<App>) {
        let now = text::now();
        let todo: Vec<String> = app
            .health
            .mismatched(&app.ratings.runtimes())
            .into_iter()
            .filter(|rel| {
                let key = format!("~{rel}");
                let mtime = app.library.entry(rel).and_then(|e| e["mtime"].as_f64()).unwrap_or(0.0);
                !self
                    .lock()
                    .get(&key)
                    .is_some_and(|t| (t.mtime - mtime).abs() < 0.001 && now - t.at < RETRY)
            })
            .take(LINKS_PER_ROUND)
            .collect();
        for rel in todo {
            if self.stop.load(Ordering::SeqCst) || *app.shutdown.borrow() {
                break;
            }
            while app.pipeline.busy() && !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(5));
            }
            match enrich::retarget(app, &rel) {
                Ok(found) => {
                    if let Some(r) = found {
                        let msg = t(
                            "Zuordnung korrigiert: {book} war mit {old} verknüpft, die Datei ist aber „{title}“ ({new}) – die Länge passt",
                            &[("book", &rel), ("old", &r.old), ("title", &r.title), ("new", &r.new)],
                        );
                        text::info("Bibliothek", &msg);
                        app.audit("Hörwerk", "", "library_relinked", &msg);
                    }
                    let mtime = app.library.entry(&rel).and_then(|e| e["mtime"].as_f64()).unwrap_or(0.0);
                    self.lock().insert(format!("~{rel}"), Tried { mtime, at: text::now() });
                }
                Err(_) => break, // no answer from the net: later
            }
            std::thread::sleep(PAUSE);
        }
        self.save(&app.db);
    }

    /// One round of at most `BATCH` books. True when more are waiting.
    fn run(&self, app: &Arc<App>) -> bool {
        self.correct_links(app);
        let now = text::now();
        let mut todo: Vec<(String, f64)> = app
            .health
            .needing_help()
            .into_iter()
            .filter_map(|rel| {
                let mtime = app.library.entry(&rel)?["mtime"].as_f64().unwrap_or(0.0);
                let fresh = self
                    .lock()
                    .get(&rel)
                    .is_some_and(|t| (t.mtime - mtime).abs() < 0.001 && now - t.at < RETRY);
                (!fresh).then_some((rel, mtime))
            })
            .collect();
        let more = todo.len() > BATCH;
        todo.truncate(BATCH);
        *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = (0, todo.len(), 0);
        if todo.is_empty() {
            return false;
        }
        let (mut settled, mut changed, mut fails) = (0usize, 0usize, 0usize);
        let mut aborted = false;
        for (n, (rel, mtime)) in todo.iter().enumerate() {
            if self.stop.load(Ordering::SeqCst) || *app.shutdown.borrow() {
                break;
            }
            while app.pipeline.busy() && !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(5));
            }
            let had_asin = app
                .library
                .entry(rel)
                .is_some_and(|e| e["asin"].as_str().is_some_and(|a| !a.is_empty()));
            let outcome = enrich::auto(app, rel);
            match outcome {
                Ok(o) => {
                    fails = 0;
                    let mut now_mtime = *mtime;
                    match o {
                        Outcome::Added {
                            n: added,
                            source,
                            asin,
                            title,
                        } => {
                            changed += 1;
                            settled += 1;
                            now_mtime = app
                                .library
                                .entry(rel)
                                .and_then(|e| e["mtime"].as_f64())
                                .unwrap_or(*mtime);
                            // an edition outside Audible (a CD): there is no ASIN to wait for
                            if !asin && !had_asin {
                                app.health.mark_other(app, rel);
                            }
                            text::info(
                                "Bibliothek",
                                &t(
                                    "Automatisch ergänzt: {book} – {n} Angaben (Fassung: {title}, {source})",
                                    &[
                                        ("book", rel),
                                        ("n", &added.to_string()),
                                        ("title", &title),
                                        ("source", &source),
                                    ],
                                ),
                            );
                        }
                        Outcome::Complete { source } => {
                            settled += 1;
                            if !had_asin && !matches!(source.as_str(), "audible" | "audnexus") {
                                app.health.mark_other(app, rel);
                            }
                            // the edition has nothing more to give: year and narrator that are missing stay missing
                            app.health.settle(&app.db, rel, &["no_year", "no_narrator"], "auto");
                        }
                        // nothing sure enough (a niche work, a recording of one's own, an edition the shop does not
                        // know) or not one M4B: the book is left alone and does not nag as a finding – looked at
                        // again after two weeks, or when the admin asks for it
                        Outcome::Unsure | Outcome::Skipped => {
                            let mut codes = vec!["no_year", "no_narrator"];
                            if !had_asin {
                                codes.push("no_asin");
                            }
                            app.health.settle(&app.db, rel, &codes, "auto");
                        }
                    }
                    self.lock().insert(
                        rel.clone(),
                        Tried {
                            mtime: now_mtime,
                            at: text::now(),
                        },
                    );
                }
                Err(_) => {
                    fails += 1;
                    if fails >= MAX_FAILS {
                        aborted = true;
                        text::warn(
                            "Bibliothek",
                            &t(
                                "Automatisch zuordnen: keine Antwort aus dem Netz – abgebrochen, später erneut versuchen",
                                &[],
                            ),
                        );
                        break;
                    }
                }
            }
            *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = (n + 1, todo.len(), settled);
            if (n + 1) % 20 == 0 {
                self.save(&app.db);
            }
            std::thread::sleep(PAUSE);
        }
        self.save(&app.db);
        if changed > 0 {
            crate::mediaservers::request_scans(app); // once, not per book
        }
        let (done, total, settled) = *self.progress.lock().unwrap_or_else(|p| p.into_inner());
        text::info(
            "Bibliothek",
            &t(
                "Automatisch zuordnen: {done} von {total} Büchern angesehen, {settled} zugeordnet oder ergänzt",
                &[
                    ("done", &done.to_string()),
                    ("total", &total.to_string()),
                    ("settled", &settled.to_string()),
                ],
            ),
        );
        // another round only when this one got somewhere (a book that could not be settled is not asked again soon)
        more && !aborted && !self.stop.load(Ordering::SeqCst)
    }
}
