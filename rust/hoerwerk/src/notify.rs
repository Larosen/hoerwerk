//! Push notifications via ntfy (notify.py): each event on or off on its own, sent in the background; the access
//! token / password stays on the server, never follows a changed address and never follows a redirect.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::{Map, Value, json};

use crate::db::Db;
use crate::http;
use crate::text::{self, t};

/// event -> (switched on by default, ntfy tags shown as emoji)
pub const EVENTS: [(&str, bool, &str); 11] = [
    ("filed", true, "books"),
    ("review", true, "mag"),
    ("duplicate", true, "twisted_rightwards_arrows"),
    ("failed", true, "warning"),
    ("request_new", true, "raising_hand"),
    ("request_failed", true, "x"),
    ("download_sent", false, "inbox_tray"),
    ("download_done", false, "white_check_mark"),
    ("download_failed", true, "x"),
    ("abs_error", true, "warning"),
    ("plex_error", true, "warning"),
];
const SECRETS: [&str; 2] = ["token", "password"];

/// The page a message is about (tapping it in the ntfy app opens it)
fn event_page(event: &str) -> &'static str {
    match event {
        "request_new" | "request_failed" => "/requests",
        "filed" | "review" | "duplicate" | "failed" | "download_done" => "/books",
        "download_sent" | "download_failed" => "/search",
        "abs_error" | "plex_error" => "/settings/health",
        _ => "",
    }
}

fn valid_topic(topic: &str) -> bool {
    (1..=64).contains(&topic.len()) && topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if !url.is_empty() && !url.to_lowercase().starts_with("http://") && !url.to_lowercase().starts_with("https://") {
        format!("https://{url}")
    } else {
        url.to_string()
    }
}

pub type Poster = dyn Fn(&str, &Value, &[(String, String)]) -> Result<(), String> + Send + Sync;

fn post(url: &str, body: &Value, headers: &[(String, String)]) -> Result<(), String> {
    let mut call = http::post(url).json(body).timeout(10).no_redirect().limit(64 * 1024);
    for (k, v) in headers {
        call = call.header(k, v);
    }
    match call.send() {
        Ok(r) if r.ok() => Ok(()),
        Ok(r) if r.status == 401 || r.status == 403 => {
            Err(t("ntfy: Zugang abgelehnt – Token oder Passwort prüfen", &[]))
        }
        Ok(r) if r.status == 429 => Err(t("ntfy: zu viele Nachrichten (Limit des Servers)", &[])),
        Ok(r) => Err(format!("ntfy: HTTP {}", r.status)),
        Err(e) => Err(t("ntfy nicht erreichbar: {reason}", &[("reason", &e)])),
    }
}

pub struct Notifier {
    db: Db,
    poster: Arc<Poster>,
    pub last_error: Arc<Mutex<Option<String>>>,
    pub sent: Arc<AtomicUsize>,
    public_url: Box<dyn Fn() -> String + Send + Sync>,
}

impl Notifier {
    pub fn new(db: Db, public_url: Box<dyn Fn() -> String + Send + Sync>) -> Self {
        Notifier {
            db,
            poster: Arc::new(post),
            last_error: Arc::new(Mutex::new(None)),
            sent: Arc::new(AtomicUsize::new(0)),
            public_url,
        }
    }

    #[cfg(test)]
    pub fn with_poster(db: Db, poster: Arc<Poster>) -> Self {
        Notifier {
            poster,
            ..Notifier::new(db, Box::new(String::new))
        }
    }

    fn load(&self) -> Map<String, Value> {
        self.db
            .get("ntfy")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    pub fn events(&self) -> Map<String, Value> {
        let c = self.load();
        let chosen = c.get("events").and_then(Value::as_object).cloned().unwrap_or_default();
        EVENTS
            .iter()
            .map(|(k, d, _)| (k.to_string(), json!(chosen.get(*k).map_or(*d, truthy))))
            .collect()
    }

    pub fn configured(&self) -> bool {
        let c = self.load();
        !str_of(&c, "url").is_empty() && !str_of(&c, "topic").is_empty()
    }

    pub fn wants(&self, event: &str) -> bool {
        self.configured() && self.events().get(event).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn public_config(&self) -> Value {
        let c = self.load();
        json!({"url": str_of(&c, "url"), "topic": str_of(&c, "topic"), "user": str_of(&c, "user"),
               "priority": priority(c.get("priority")), "app_url": str_of(&c, "app_url"),
               "token_set": !str_of(&c, "token").is_empty(), "password_set": !str_of(&c, "password").is_empty(),
               "events": self.events(), "configured": self.configured(),
               "last_error": *self.last_error.lock().unwrap_or_else(|p| p.into_inner()),
               "available": EVENTS.iter().map(|e| e.0).collect::<Vec<_>>()})
    }

    pub fn update_config(&self, new: &Value) -> Result<Value, String> {
        let old = self.load();
        let s = |k: &str| match &new[k] {
            Value::Null => String::new(),
            Value::String(x) => x.clone(),
            other => other.to_string(),
        };
        let mut url = base(&s("url"));
        if !url.is_empty() && !http::valid_url(&url) {
            return Err(t("Ungültige ntfy-Adresse", &[]));
        }
        let mut topic = s("topic").trim().to_string();
        let (scheme_host, path) = split_path(&url);
        let trimmed = path.trim_matches('/');
        if !url.is_empty() && !trimmed.is_empty() && topic.is_empty() {
            // "https://ntfy.sh/mytopic" pasted as a whole: split into server and topic
            topic = trimmed.rsplit('/').next().unwrap_or("").to_string();
            let prefix = if trimmed.contains('/') {
                path.rsplit_once('/').map_or("", |x| x.0)
            } else {
                ""
            };
            url = base(&format!("{scheme_host}{prefix}"));
        }
        if !topic.is_empty() && !valid_topic(&topic) {
            return Err(t("Topic: nur Buchstaben, Ziffern, - und _ (max. 64 Zeichen)", &[]));
        }
        let app_url = s("app_url").trim().to_string();
        if !app_url.is_empty() && !http::valid_url(&app_url) {
            return Err(t("Adresse der Oberfläche muss mit http:// oder https:// beginnen", &[]));
        }
        let prio = match &new["priority"] {
            Value::Null => 3,
            Value::Number(n) => n.as_f64().map(|f| f as i64).unwrap_or(3),
            Value::String(x) if x.trim().is_empty() => 3,
            Value::String(x) => x.trim().parse::<i64>().map_err(|_| t("Ungültige Daten", &[]))?,
            Value::Bool(b) => i64::from(*b),
            _ => return Err(t("Ungültige Daten", &[])),
        };
        let prio = if prio == 0 { 3 } else { prio };
        let same_server = http::origin(&str_of(&old, "url")) == http::origin(&url);
        let events: Map<String, Value> = self
            .events()
            .into_iter()
            .map(|(k, v)| {
                let chosen = new["events"].get(&k).map_or(v.as_bool().unwrap_or(false), truthy);
                (k, json!(chosen))
            })
            .collect();
        let mut data = json!({"url": url, "topic": topic, "user": s("user").trim(), "app_url": app_url,
                              "priority": if (1..=5).contains(&prio) { prio } else { 3 }, "events": events});
        for key in SECRETS {
            // empty field = keep the stored one, but never for another server
            let given = s(key).trim().to_string();
            data[key] = json!(if !given.is_empty() {
                given
            } else if same_server {
                str_of(&old, key)
            } else {
                String::new()
            });
        }
        self.db.put("ntfy", &data).map_err(|e| e.to_string())?;
        Ok(self.public_config())
    }

    fn headers(c: &Map<String, Value>) -> Vec<(String, String)> {
        if !str_of(c, "token").is_empty() {
            return vec![("Authorization".into(), format!("Bearer {}", str_of(c, "token")))];
        }
        if !str_of(c, "user").is_empty() && !str_of(c, "password").is_empty() {
            let raw = format!("{}:{}", str_of(c, "user"), str_of(c, "password"));
            return vec![(
                "Authorization".into(),
                format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw)),
            )];
        }
        Vec::new()
    }

    /// Where links in messages point: the own address field, else the public address (Allgemein).
    pub fn link_base(&self, c: &Map<String, Value>) -> String {
        let own = str_of(c, "app_url");
        let url = if own.is_empty() { (self.public_url)() } else { own };
        url.trim_end_matches('/').to_string()
    }

    fn message(
        &self,
        event: &str,
        title: &str,
        message: &str,
        prio: Option<i64>,
    ) -> (String, Value, Vec<(String, String)>) {
        let c = self.load();
        let tags: Vec<&str> = EVENTS.iter().filter(|e| e.0 == event).map(|e| e.2).collect();
        let mut body = json!({"topic": str_of(&c, "topic"), "title": cut(title, 250), "message": cut(message, 3500),
                              "priority": prio.unwrap_or_else(|| priority(c.get("priority"))), "tags": tags});
        let link = self.link_base(&c);
        if !link.is_empty() {
            body["click"] = json!(format!("{link}{}", event_page(event)));
        }
        (str_of(&c, "url"), body, Self::headers(&c))
    }

    /// Send in the background if this event is switched on.
    pub fn send(&self, event: &str, title: &str, message: &str, prio: Option<i64>) {
        if !self.wants(event) || crate::config::compare_mode() {
            return;
        }
        let (url, body, headers) = self.message(event, title, message, prio);
        let (poster, last, sent) = (self.poster.clone(), self.last_error.clone(), self.sent.clone());
        std::thread::spawn(move || match poster(&url, &body, &headers) {
            Ok(()) => {
                sent.fetch_add(1, Ordering::SeqCst);
                *last.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
            Err(e) => {
                text::warn("ntfy", &e);
                *last.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
            }
        });
    }

    /// A message to another topic on the same ntfy server (a family member's own topic).
    pub fn send_to(&self, topic: &str, title: &str, message: &str) {
        if !self.configured() || !valid_topic(topic) || crate::config::compare_mode() {
            return;
        }
        let c = self.load();
        let mut body =
            json!({"topic": topic, "title": cut(title, 250), "message": cut(message, 3500), "tags": ["books"]});
        let link = self.link_base(&c);
        if !link.is_empty() {
            body["click"] = json!(format!("{link}/requests"));
        }
        let (url, headers, poster, topic) = (
            str_of(&c, "url"),
            Self::headers(&c),
            self.poster.clone(),
            topic.to_string(),
        );
        std::thread::spawn(move || {
            if let Err(e) = poster(&url, &body, &headers) {
                text::warn("Benachrichtigungen", &format!("ntfy ({topic}): {e}"));
            }
        });
    }

    /// Blocking: a test message right away, so errors can be shown.
    pub fn test(&self) -> Result<String, String> {
        if !self.configured() {
            return Err(t("ntfy ist noch nicht eingerichtet (Adresse und Topic)", &[]));
        }
        let (url, body, headers) = self.message(
            "filed",
            &t("Hörwerk: Test", &[]),
            &t("Benachrichtigungen funktionieren.", &[]),
            Some(3),
        );
        (self.poster)(&url, &body, &headers)?;
        self.sent.fetch_add(1, Ordering::SeqCst);
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(t("Testnachricht gesendet", &[]))
    }

    /// Test message to a family member's own topic.
    pub fn test_to(&self, topic: &str) -> Result<String, String> {
        if !self.configured() {
            return Err(t("Der Admin hat noch keinen ntfy-Server eingerichtet", &[]));
        }
        if !valid_topic(topic) {
            return Err(t("Topic: nur Buchstaben, Ziffern, - und _ (max. 64 Zeichen)", &[]));
        }
        let c = self.load();
        let body = json!({"topic": topic, "title": t("Hörwerk: Test", &[]),
                          "message": t("Benachrichtigungen funktionieren.", &[]), "tags": ["books"]});
        (self.poster)(&str_of(&c, "url"), &body, &Self::headers(&c))
            .map_err(|e| t("ntfy: {error}", &[("error", &e)]))?;
        Ok(t("Testnachricht an „{topic}“ gesendet", &[("topic", topic)]))
    }
}

fn split_path(url: &str) -> (String, String) {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = match rest.find('/') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            (format!("{scheme}://{host}"), path.to_string())
        }
        None => (url.to_string(), String::new()),
    }
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn str_of(c: &Map<String, Value>, k: &str) -> String {
    c.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn priority(v: Option<&Value>) -> i64 {
    v.and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .filter(|p| *p != 0)
        .unwrap_or(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_stay_with_their_server() {
        let db = Db::memory();
        type Sent = Vec<(String, Value, Vec<(String, String)>)>;
        let seen: Arc<Mutex<Sent>> = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let n = Notifier::with_poster(
            db,
            Arc::new(move |u: &str, b: &Value, h: &[(String, String)]| {
                log.lock().unwrap().push((u.into(), b.clone(), h.to_vec()));
                Ok(())
            }),
        );
        let cfg = n
            .update_config(&json!({"url": "ntfy.example/familie", "token": "geheim"}))
            .unwrap();
        assert_eq!(
            (cfg["url"].as_str(), cfg["topic"].as_str(), cfg["token_set"].as_bool()),
            (Some("https://ntfy.example"), Some("familie"), Some(true))
        );
        assert!(cfg.get("token").is_none());
        n.update_config(&json!({"url": "https://ntfy.example", "topic": "familie"}))
            .unwrap();
        assert_eq!(n.public_config()["token_set"], true); // same server: kept
        n.update_config(&json!({"url": "https://anderer.example", "topic": "familie"}))
            .unwrap();
        assert_eq!(n.public_config()["token_set"], false); // another server: gone
        assert!(n.update_config(&json!({"url": "https://u:p@x.example"})).is_err());
        assert!(
            n.update_config(&json!({"url": "https://x.example", "topic": "a b"}))
                .is_err()
        );
        n.test().unwrap();
        let sent = seen.lock().unwrap();
        assert_eq!(sent[0].0, "https://anderer.example");
        assert_eq!(sent[0].1["topic"], "familie");
    }
}
