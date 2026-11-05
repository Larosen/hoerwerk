//! The running application: configuration, database, accounts, settings, live updates and the shutdown signal.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::{Map, Value};
use tokio::sync::{Notify, watch};

use crate::accounts::Accounts;
use crate::audible::Providers;
use crate::config::Config;
use crate::db::Db;
use crate::engine::Engine;
use crate::library::LibraryIndex;
use crate::listening::Listening;
use crate::mediaservers::{Abs, Plex};
use crate::notify::Notifier;
use crate::pipeline::Pipeline;
use crate::requests::Requests;
use crate::settings::Settings;
use crate::stats::Stats;
use crate::text;
use crate::uploads::Uploads;
use crate::usenet::Usenet;
use crate::web::Slots;

pub const MAX_SSE: usize = 32;

pub struct App {
    pub cfg: Config,
    pub db: Db,
    pub library: LibraryIndex,
    /// the first library scan is done: only then is anything processed (duplicates are checked against it)
    pub library_ready: AtomicBool,
    pub library_error: Mutex<String>,
    pub notifier: Notifier,
    pub engine: Arc<Engine>,
    pub pipeline: Pipeline,
    pub providers: Providers,
    pub uploads: Uploads,
    pub abs: Abs,
    pub plex: Plex,
    pub listening: Listening,
    /// the daily database snapshot: when it last worked, why it last failed ("" = fine), when it last ran
    pub db_backup: Mutex<(Option<f64>, String, f64)>,
    pub usenet: Usenet,
    pub requests: Requests,
    pub stats: Stats,
    pub accounts: Accounts,
    pub settings: Settings,
    pub slots: Slots,
    pub refused_hosts: Mutex<HashMap<String, (f64, f64)>>,
    /// proxies in the own network that send X-Forwarded-For but are not trusted: peer -> (last seen, last logged)
    pub untrusted_proxies: Mutex<HashMap<String, (f64, f64)>>,
    pub started: f64,
    /// topic -> version; a change wakes every live connection
    pub versions: Mutex<HashMap<String, u64>>,
    pub changed: Notify,
    pub sse_clients: AtomicUsize,
    pub shutdown: watch::Sender<bool>,
    pub totp: Mutex<Totp>,
    pub setup_code: String,
}

#[derive(Default)]
pub struct Totp {
    pub tickets: HashMap<String, (i64, bool, f64, u32)>, // ticket -> (uid, persistent, expires, tries)
    pub pending: HashMap<i64, String>,
    pub used: HashMap<i64, i64>,
}

impl App {
    pub fn new(cfg: Config, db: Db) -> Self {
        let settings = Settings::new(&cfg.env);
        let (shutdown, _) = watch::channel(false);
        App {
            library: LibraryIndex::new(&cfg.library, db.clone()),
            library_ready: AtomicBool::new(false),
            library_error: Mutex::new(String::new()),
            notifier: Notifier::new(db.clone(), {
                let (db, settings) = (db.clone(), Settings::new(&cfg.env));
                Box::new(move || {
                    settings
                        .get(&db)
                        .get("public_url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                })
            }),
            usenet: Usenet::new(db.clone()),
            engine: Arc::new(Engine::new(cfg.encode_workers, cfg.io_workers)),
            pipeline: Pipeline::new(&cfg.inbox, &cfg.library, &cfg.work_dir),
            providers: Providers::default(),
            uploads: Uploads::new(&cfg.inbox),
            abs: Abs::new(db.clone()),
            plex: Plex::new(db.clone()),
            listening: Listening::default(),
            db_backup: Mutex::new((None, String::new(), 0.0)),
            requests: Requests::new(),
            stats: Stats::default(),
            accounts: Accounts::new(),
            settings,
            db,
            cfg,
            slots: Slots::new(),
            refused_hosts: Mutex::new(HashMap::new()),
            untrusted_proxies: Mutex::new(HashMap::new()),
            started: text::now(),
            versions: Mutex::new(HashMap::new()),
            changed: Notify::new(),
            sse_clients: AtomicUsize::new(0),
            shutdown,
            totp: Mutex::new(Totp::default()),
            setup_code: String::new(),
        }
    }

    pub fn settings(&self) -> Map<String, Value> {
        self.settings.get(&self.db)
    }

    pub fn setting_bool(&self, key: &str) -> bool {
        self.settings().get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    /// Host of the public address from the settings ("" when none): "https://hb.example.de:8443" -> "hb.example.de".
    pub fn public_host(&self) -> String {
        let url = self
            .settings()
            .get("public_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let rest = url.split_once("://").map(|x| x.1).unwrap_or("");
        let host = match rest.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => rest.split([':', '/']).next().unwrap_or(""),
        };
        host.to_lowercase()
    }

    pub fn note_refused_host(&self, name: &str) {
        let now = text::now();
        let mut seen = self.refused_hosts.lock().unwrap_or_else(|p| p.into_inner());
        if seen.len() > 50 && !seen.contains_key(name) {
            return;
        }
        let logged = seen.get(name).map_or(0.0, |x| x.1);
        let log_now = now - logged >= 3600.0;
        seen.insert(name.to_string(), (now, if log_now { now } else { logged }));
        drop(seen);
        if log_now {
            text::warn(
                "Anmeldung",
                &text::t(
                    "Aufruf unter dem Namen „{host}“ abgelehnt (Schutz gegen DNS-Rebinding). Wenn das deine Adresse ist: unter Einstellungen → Allgemein als öffentliche Adresse eintragen oder ALLOWED_HOSTS={host} setzen.",
                    &[("host", name)],
                ),
            );
        }
    }

    pub fn note_untrusted(&self, peer: &str) {
        let now = text::now();
        let mut seen = self.untrusted_proxies.lock().unwrap_or_else(|p| p.into_inner());
        if seen.len() > 50 && !seen.contains_key(peer) {
            return;
        }
        let logged = seen.get(peer).map_or(0.0, |x| x.1);
        let log_now = now - logged >= 3600.0;
        seen.insert(peer.to_string(), (now, if log_now { now } else { logged }));
        drop(seen);
        if log_now {
            text::warn(
                "Anmeldung",
                &text::t(
                    "{peer} schickt X-Forwarded-For, steht aber nicht in TRUSTED_PROXIES – alle Besucher über diesen Proxy erscheinen mit seiner Adresse (Anmelde-Sperre, CrowdSec). TRUSTED_PROXIES={peer} setzen.",
                    &[("peer", peer)],
                ),
            );
        }
    }

    pub fn audit(&self, user: &str, ip: &str, action: &str, detail: &str) {
        let _ = self.db.lock().execute(
            "INSERT INTO audit(time, user, ip, action, detail) VALUES(?,?,?,?,?)",
            rusqlite::params![
                text::now(),
                user,
                ip,
                action,
                detail.chars().take(500).collect::<String>()
            ],
        );
    }

    pub fn publish(&self, topic: &str) {
        *self
            .versions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(topic.to_string())
            .or_default() += 1;
        self.changed.notify_waiters();
    }

    pub fn version(&self, topic: &str) -> u64 {
        self.versions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(topic)
            .copied()
            .unwrap_or(0)
    }

    /// The state of a live topic (what the pages show).
    pub fn snapshot(&self, topic: &str) -> Value {
        match topic {
            "audiobooks" => {
                let mut s = self.pipeline.status(self);
                s["queues"] = self.engine.stats();
                s
            }
            "library" => self.library.stats(),
            _ => Value::Null,
        }
    }

    pub fn topics(&self) -> [&'static str; 2] {
        ["audiobooks", "library"]
    }

    /// Live updates of the library index reach the browser (called once the app is shared).
    pub fn wire(app: &Arc<App>) {
        let weak = Arc::downgrade(app);
        app.library.on_change(Box::new(move || {
            if let Some(a) = weak.upgrade() {
                a.publish("library");
            }
        }));
        let weak = Arc::downgrade(app);
        app.engine.on_change(Box::new(move || {
            if let Some(a) = weak.upgrade() {
                a.publish("audiobooks");
            }
        }));
    }

    /// Once a day a consistent snapshot (backups/hoerwerk-<weekday>.db, the last seven days) and the WAL moved
    /// into the main file; after a failure again in an hour. Blocking.
    pub fn maintain_db(&self, now: f64) -> Option<std::path::PathBuf> {
        let mut state = self.db_backup.lock().unwrap_or_else(|p| p.into_inner());
        let wait = if state.1.is_empty() { 23.0 * 3600.0 } else { 3600.0 };
        if now - state.2 < wait {
            return None;
        }
        state.2 = now;
        let day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][text::local(now).tm_wday.clamp(0, 6) as usize];
        let dir = self.cfg.config_dir.join("backups");
        let target = dir.join(format!("hoerwerk-{day}.db"));
        let done = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
            })
            .and_then(|_| self.db.backup(&target));
        match done {
            Ok(()) => {
                self.db.checkpoint();
                *state = (Some(now), String::new(), now);
                Some(target)
            }
            Err(e) => {
                text::warn(
                    "Datenbank",
                    &text::t(
                        "Datenbank-Sicherung fehlgeschlagen, neuer Versuch in einer Stunde: {error}",
                        &[("error", &e)],
                    ),
                );
                state.1 = e;
                None
            }
        }
    }

    /// The newest pipeline job of a request.
    pub fn job_for_request(&self, rid: i64) -> Option<Value> {
        self.pipeline
            .all()
            .iter()
            .rev()
            .map(|j| j.lock().to_dict(true))
            .find(|j| j["hints"]["request_id"].as_i64() == Some(rid))
    }

    pub fn stopping(&self) -> bool {
        *self.shutdown.borrow()
    }

    pub fn sse_open(&self) -> bool {
        self.sse_clients.fetch_add(1, Ordering::SeqCst) < MAX_SSE || {
            self.sse_clients.fetch_sub(1, Ordering::SeqCst);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_snapshot() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hw-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let env: std::collections::BTreeMap<String, String> = [
            ("CONFIG_DIR", dir.join("config")),
            ("LIBRARY_DIR", dir.join("lib")),
            ("INBOX_DIR", dir.join("in")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.display().to_string()))
        .collect();
        let cfg = Config::from_map(env).unwrap();
        let db = Db::open(&cfg.config_dir.join("hoerwerk.db")).unwrap();
        let app = App::new(cfg, db);
        let t0 = 1_800_000_000.0;
        let target = app.maintain_db(t0).expect("snapshot");
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o600);
        let copy = rusqlite::Connection::open(&target).unwrap();
        let v: i64 = copy.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 5);
        assert!(app.maintain_db(t0 + 3600.0).is_none()); // once a day
        assert!(app.maintain_db(t0 + 23.0 * 3600.0).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
