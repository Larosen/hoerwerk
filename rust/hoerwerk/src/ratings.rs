//! Audible's data for the books of the library: the listeners' rating and the runtime, fetched once per book (by
//! its ASIN) in the background, slowly, kept for a month. The rating sorts the library – so badly rated books can
//! be found – and the runtime shows books that are another recording than the one Audible lists. Books without an
//! ASIN have neither.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::app::App;
use crate::db::Db;
use crate::text::{self, t};

const KEY: &str = "library_ratings";
/// ratings change slowly: looked up again after this long
const MAX_AGE: f64 = 30.0 * 86400.0;
/// pause between two requests (Audible's public catalog is not hammered)
const PAUSE: Duration = Duration::from_millis(700);
/// this many failures in a row (Audible not reachable): the run ends
const MAX_FAILS: usize = 5;

#[derive(Clone, Copy)]
struct Item {
    rating: f64,
    count: i64,
    /// Audible's runtime of the title in minutes (0 = unknown)
    runtime_min: i64,
    at: f64,
}

/// Ratings with fewer votes than this say little: the library sorts them after the others.
pub const MIN_VOTES: i64 = 5;

/// "Badly rated": below this average with at least this many votes (a handful of votes is not a verdict).
pub const LOW_RATING: f64 = 3.4;
pub const LOW_VOTES: i64 = 20;

pub fn is_low(rating: f64, votes: i64) -> bool {
    votes >= LOW_VOTES && rating > 0.0 && rating < LOW_RATING
}

pub struct Ratings {
    items: Mutex<HashMap<String, Item>>,
    running: AtomicBool,
    stop: AtomicBool,
    progress: Mutex<(usize, usize)>,
}

impl Ratings {
    pub fn new(db: &Db) -> Self {
        let mut items = HashMap::new();
        if let Some(d) = db.get(KEY).as_ref().and_then(Value::as_object) {
            for (asin, v) in d {
                items.insert(
                    asin.clone(),
                    Item {
                        rating: v["r"].as_f64().unwrap_or(0.0),
                        count: v["n"].as_i64().unwrap_or(0),
                        runtime_min: v["m"].as_i64().unwrap_or(0),
                        // an older entry without the runtime is fetched again
                        at: if v["m"].is_null() {
                            0.0
                        } else {
                            v["t"].as_f64().unwrap_or(0.0)
                        },
                    },
                );
            }
        }
        Ratings {
            items: Mutex::new(items),
            running: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            progress: Mutex::new((0, 0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Item>> {
        self.items.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// asin -> (average, number of ratings) of the titles that have ratings.
    pub fn snapshot(&self) -> HashMap<String, (f64, i64)> {
        self.lock()
            .iter()
            .filter(|(_, i)| i.count > 0)
            .map(|(a, i)| (a.clone(), (i.rating, i.count)))
            .collect()
    }

    /// asin -> Audible's runtime in minutes, where known.
    pub fn runtimes(&self) -> HashMap<String, i64> {
        self.lock()
            .iter()
            .filter(|(_, i)| i.runtime_min > 0)
            .map(|(a, i)| (a.clone(), i.runtime_min))
            .collect()
    }

    fn save(&self, db: &Db) {
        let map: serde_json::Map<String, Value> = self
            .lock()
            .iter()
            .map(|(a, i)| {
                (
                    a.clone(),
                    json!({"r": i.rating, "n": i.count, "m": i.runtime_min, "t": i.at}),
                )
            })
            .collect();
        let _ = db.put(KEY, &Value::Object(map));
    }

    pub fn status(&self, app: &App) -> Value {
        let asins: HashSet<String> = app
            .library
            .entries()
            .iter()
            .filter_map(|e| e["asin"].as_str().filter(|a| !a.is_empty()).map(str::to_string))
            .collect();
        let items = self.lock();
        let have = asins
            .iter()
            .filter(|a| items.get(*a).is_some_and(|i| i.count > 0))
            .count();
        let (done, total) = *self.progress.lock().unwrap_or_else(|p| p.into_inner());
        json!({"running": self.running.load(Ordering::SeqCst), "done": done, "total": total,
               "with_asin": asins.len(), "rated": have})
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Fetch the ratings that are missing or older than a month, in the background. False: already running.
    pub fn start(self: &Arc<Self>, app: &Arc<App>) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        let (me, app) = (self.clone(), app.clone());
        std::thread::Builder::new()
            .name("library-ratings".into())
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

    fn run(&self, app: &Arc<App>) {
        let now = text::now();
        let region = app
            .settings()
            .get("ab_region")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
            .unwrap_or("de")
            .to_string();
        let mut todo: Vec<String> = app
            .library
            .entries()
            .iter()
            .filter_map(|e| e["asin"].as_str().filter(|a| !a.is_empty()).map(str::to_string))
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|a| self.lock().get(a).is_none_or(|i| now - i.at > MAX_AGE))
            .collect();
        todo.sort();
        *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = (0, todo.len());
        let mut fails = 0;
        for (n, asin) in todo.iter().enumerate() {
            if self.stop.load(Ordering::SeqCst) || *app.shutdown.borrow() {
                break;
            }
            while app.pipeline.busy() && !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(5));
            }
            match app.providers.client.basics(asin, &region) {
                Ok(b) => {
                    fails = 0;
                    self.lock().insert(
                        asin.clone(),
                        Item {
                            rating: b.rating,
                            count: b.ratings,
                            runtime_min: b.runtime_min,
                            at: text::now(),
                        },
                    );
                }
                // a title Audible does not know: remembered as "no rating" for a month
                Err(e) if e.0.contains("nicht gefunden") => {
                    fails = 0;
                    self.lock().insert(
                        asin.clone(),
                        Item {
                            rating: 0.0,
                            count: 0,
                            runtime_min: 0,
                            at: text::now(),
                        },
                    );
                }
                Err(_) => {
                    fails += 1;
                    if fails >= MAX_FAILS {
                        text::warn(
                            "Bibliothek",
                            &t(
                                "Audible-Daten: Audible antwortet nicht – abgebrochen, später erneut versuchen",
                                &[],
                            ),
                        );
                        break;
                    }
                }
            }
            *self.progress.lock().unwrap_or_else(|p| p.into_inner()) = (n + 1, todo.len());
            if (n + 1) % 25 == 0 {
                self.save(&app.db);
            }
            std::thread::sleep(PAUSE);
        }
        self.save(&app.db);
        let (done, total) = *self.progress.lock().unwrap_or_else(|p| p.into_inner());
        text::info(
            "Bibliothek",
            &t(
                "Audible-Daten: {done} von {total} Titeln abgefragt",
                &[("done", &done.to_string()), ("total", &total.to_string())],
            ),
        );
    }
}
