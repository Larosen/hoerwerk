//! The running application: configuration, database, accounts, settings, live updates and the shutdown signal.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value, json};
use tokio::sync::{Notify, watch};

use crate::accounts::Accounts;
use crate::config::Config;
use crate::db::Db;
use crate::settings::Settings;
use crate::text;
use crate::web::Slots;

pub const MAX_SSE: usize = 32;

pub struct App {
    pub cfg: Config,
    pub db: Db,
    pub accounts: Accounts,
    pub settings: Settings,
    pub slots: Slots,
    pub refused_hosts: Mutex<HashMap<String, (f64, f64)>>,
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
            accounts: Accounts::new(),
            settings,
            db,
            cfg,
            slots: Slots::new(),
            refused_hosts: Mutex::new(HashMap::new()),
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
            "audiobooks" => json!({
                "enabled": self.setting_bool("ab_watch"), "inbox": self.cfg.inbox, "library": self.cfg.library,
                "inbox_ok": self.cfg.inbox.is_dir(), "library_ok": self.cfg.library.is_dir(), "last_scan": null,
                "scan_note": "", "waiting": [], "ignored": [], "jobs": [], "review": 0,
                "queues": {"encode": {"workers": 1, "running": 0, "waiting": 0}, "io": {"workers": 2, "running": 0, "waiting": 0}},
            }),
            "library" => json!({"books": 0, "m4b": 0, "other": 0, "last_scan": null, "scanning": false}),
            _ => Value::Null,
        }
    }

    pub fn topics(&self) -> [&'static str; 2] {
        ["audiobooks", "library"]
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
