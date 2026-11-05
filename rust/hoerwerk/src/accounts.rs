//! Accounts, passwords, login sessions and the brake against password guessing – compatible with the Python
//! version's database: passwords ``scrypt$N$r$p$salt$hash`` (base64), sessions as SHA-256 of a random token.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rand::RngCore;
use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::text::{now, t};

pub const ROLES: [&str; 2] = ["admin", "user"];
pub const PERMS: [&str; 3] = ["see_all_requests", "auto_approve", "interactive_search"];
const MIN_PASSWORD: usize = 12;
const PASSPHRASE: usize = 20;
const MAX_PASSWORD: usize = 256;
pub const NAME_MAX: usize = 40;
pub const SESSION_SHORT: f64 = 12.0 * 3600.0;
pub const SESSION_LONG: f64 = 30.0 * 24.0 * 3600.0;
const TOUCH_EVERY: f64 = 300.0;
const IP_FAILS: usize = 5;
const WINDOW: f64 = 15.0 * 60.0;
const MAX_TRACKED: usize = 10_000;
const OBVIOUS: [&str; 13] = [
    "hörwerk",
    "hoerwerk",
    "horwerk",
    "passwort",
    "password",
    "kennwort",
    "123456",
    "qwertz",
    "qwerty",
    "asdfgh",
    "hörbuch",
    "hoerbuch",
    "audiobook",
];

#[derive(Debug)]
pub struct AuthError {
    pub message: String,
    pub status: u16,
    pub reason: &'static str,
}

fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::rng().fill_bytes(&mut v);
    v
}

pub fn hash_password(password: &str) -> String {
    let salt = random(16);
    let params = scrypt::Params::new(15, 8, 1, 32).expect("valid scrypt parameters");
    let mut out = [0u8; 32];
    scrypt::scrypt(password.as_bytes(), &salt, &params, &mut out).expect("scrypt output length");
    format!("scrypt${}${}${}${}${}", 1 << 15, 8, 1, b64(&salt), b64(&out))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let [kind, n, r, p, salt, digest] = parts[..] else {
        return false;
    };
    if kind != "scrypt" {
        return false;
    }
    let (Ok(n), Ok(r), Ok(p)) = (n.parse::<u64>(), r.parse::<u32>(), p.parse::<u32>()) else {
        return false;
    };
    let (Ok(salt), Ok(want)) = (STANDARD.decode(salt), STANDARD.decode(digest)) else {
        return false;
    };
    if !n.is_power_of_two() || want.is_empty() || want.len() > 64 {
        return false;
    }
    let Ok(params) = scrypt::Params::new(n.trailing_zeros() as u8, r, p, want.len()) else {
        return false;
    };
    let mut got = vec![0u8; want.len()];
    if scrypt::scrypt(password.as_bytes(), &salt, &params, &mut got).is_err() {
        return false;
    }
    got.iter().zip(&want).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0 // constant time
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// What is wrong with a new password (empty = fine) – the same rules as the browser checklist.
pub fn password_problems(password: &str, name: &str) -> Vec<String> {
    if password.chars().count() > MAX_PASSWORD {
        return vec![t("Das Passwort ist zu lang", &[])];
    }
    let mut out = vec![];
    let low = password.to_lowercase();
    let len = password.chars().count();
    if len < MIN_PASSWORD {
        out.push(t("mindestens {n} Zeichen", &[("n", &MIN_PASSWORD.to_string())]));
    }
    let kinds = [
        password.chars().any(|c| c.is_ascii_lowercase() || "äöüß".contains(c)),
        password.chars().any(|c| c.is_ascii_uppercase() || "ÄÖÜ".contains(c)),
        password.chars().any(|c| c.is_ascii_digit()),
        password
            .chars()
            .any(|c| !(c.is_alphanumeric()) || c == '_' || c.is_whitespace()),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if kinds < 3 && len < PASSPHRASE {
        out.push(t(
            "drei von vier: Kleinbuchstaben, Großbuchstaben, Ziffern, Sonderzeichen – oder ein Satz ab {n} Zeichen",
            &[("n", &PASSPHRASE.to_string())],
        ));
    }
    let clean = name.trim().to_lowercase();
    if clean.chars().count() >= 3 && low.contains(&clean) {
        out.push(t("nicht der eigene Name", &[]));
    }
    if OBVIOUS.iter().any(|w| low.contains(w)) {
        out.push(t("nicht „Hörwerk“, „Passwort“, „123456“ oder Ähnliches", &[]));
    }
    // one word with numbers/signs around it ("Pudding2024!")
    let trimmed = password.trim();
    let letters: Vec<(usize, char)> = trimmed.char_indices().filter(|(_, c)| c.is_alphabetic()).collect();
    if let (Some(first), Some(last)) = (letters.first(), letters.last()) {
        let word = &trimmed[first.0..last.0 + last.1.len_utf8()];
        if word.chars().all(char::is_alphabetic) && word.chars().count() <= 10 {
            out.push(t("nicht nur ein Wort mit Zahlen oder Zeichen drumherum", &[]));
        }
    }
    if low.chars().collect::<std::collections::HashSet<_>>().len() < 6 {
        out.push(t("nicht immer dieselben Zeichen", &[]));
    }
    out
}

pub fn check_password(password: &str, name: &str) -> Result<(), String> {
    let problems = password_problems(password, name);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(t(
            "Passwort zu schwach – es braucht: {rules}",
            &[("rules", &problems.join("; "))],
        ))
    }
}

pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(|c| (c as u32) < 32) {
        return Err(t("Ungültiger Name", &[]));
    }
    Ok(name.to_string())
}

/// What the login brake counts: an IPv4 address, or the /64 network of an IPv6 address.
pub fn ip_bucket(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
        Err(_) => ip.to_string(),
    }
}

fn quota(value: &Value) -> Result<Option<i64>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) if s.is_empty() => Ok(None),
        other => {
            let n = match other {
                Value::Number(n) => n.as_f64().map(|f| f as i64),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            }
            .ok_or_else(|| t("Ungültiges Kontingent", &[]))?;
            if (0..=1000).contains(&n) {
                Ok(Some(n))
            } else {
                Err(t("Ungültiges Kontingent", &[]))
            }
        }
    }
}

pub fn public(row: &Row) -> rusqlite::Result<Value> {
    let perms: Value = serde_json::from_str(&row.get::<_, String>("perms")?).unwrap_or(json!({}));
    let mut p = Map::new();
    for k in PERMS {
        p.insert(k.into(), json!(perms.get(k).and_then(Value::as_bool).unwrap_or(false)));
    }
    Ok(json!({
        "id": row.get::<_, i64>("id")?, "name": row.get::<_, String>("name")?, "role": row.get::<_, String>("role")?,
        "perms": p, "quota": row.get::<_, Option<i64>>("quota")?, "disabled": row.get::<_, i64>("disabled")? != 0,
        "created": row.get::<_, f64>("created")?, "last_login": row.get::<_, Option<f64>>("last_login")?,
        "totp": row.get::<_, Option<String>>("totp")?.is_some_and(|s| !s.is_empty()),
        "origin": row.get::<_, String>("origin")?,
    }))
}

pub fn me(user: &Value) -> Value {
    let mut out = Map::new();
    for k in ["id", "name", "role", "perms", "quota", "totp"] {
        out.insert(k.into(), user.get(k).cloned().unwrap_or(Value::Null));
    }
    Value::Object(out)
}

#[derive(Default)]
struct Brake {
    ip_fails: HashMap<String, VecDeque<f64>>,
    user_fails: HashMap<String, u32>,
}

pub struct Accounts {
    brake: Mutex<Brake>,
    dummy: String,
}

impl Default for Accounts {
    fn default() -> Self {
        Self::new()
    }
}

impl Accounts {
    pub fn new() -> Self {
        Accounts {
            brake: Mutex::new(Brake::default()),
            dummy: hash_password(&URL_SAFE_NO_PAD.encode(random(16))),
        }
    }

    pub fn count(&self, db: &Db) -> i64 {
        db.lock()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn get(&self, db: &Db, uid: i64) -> Option<Value> {
        db.lock()
            .query_row("SELECT * FROM users WHERE id=?", [uid], public)
            .optional()
            .ok()
            .flatten()
    }

    pub fn by_name(&self, db: &Db, name: &str) -> Option<Value> {
        db.lock()
            .query_row("SELECT * FROM users WHERE name=?", [name.trim()], public)
            .optional()
            .ok()
            .flatten()
    }

    pub fn list(&self, db: &Db) -> Vec<Value> {
        let c = db.lock();
        let mut st = c
            .prepare("SELECT * FROM users ORDER BY name COLLATE NOCASE")
            .expect("valid SQL");
        st.query_map([], |r| {
            let mut u = public(r)?;
            let seen = [u["last_login"].as_f64(), r.get::<_, Option<f64>>("last_seen")?]
                .into_iter()
                .flatten()
                .reduce(f64::max);
            u["last_seen"] = json!(seen);
            Ok(u)
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn create(
        &self,
        db: &Db,
        name: &str,
        password: &str,
        role: &str,
        perms: &Value,
        quota_v: &Value,
    ) -> Result<Value, String> {
        let name = check_name(name)?;
        check_password(password, &name)?;
        if !ROLES.contains(&role) {
            return Err(t("Unbekannte Rolle", &[]));
        }
        if self.by_name(db, &name).is_some() {
            return Err(t("Den Namen gibt es schon", &[]));
        }
        let q = quota(quota_v)?;
        let mut clean = Map::new();
        for k in PERMS {
            clean.insert(k.into(), json!(perms.get(k).and_then(Value::as_bool).unwrap_or(false)));
        }
        let ts = now();
        let id = {
            let c = db.lock();
            c.execute(
                "INSERT INTO users(name, pw, role, perms, quota, created, pw_changed) VALUES(?,?,?,?,?,?,?)",
                params![
                    name,
                    hash_password(password),
                    role,
                    Value::Object(clean).to_string(),
                    q,
                    ts,
                    ts
                ],
            )
            .map_err(|e| e.to_string())?;
            c.last_insert_rowid()
        };
        self.get(db, id).ok_or_else(|| "user vanished".into())
    }

    fn admins(&self, db: &Db, exclude: i64) -> i64 {
        db.lock()
            .query_row(
                "SELECT COUNT(*) FROM users WHERE role='admin' AND disabled=0 AND id<>?",
                [exclude],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// Ok(None) = no such account.
    pub fn update(&self, db: &Db, uid: i64, changes: &Map<String, Value>) -> Result<Option<Value>, String> {
        let Some(user) = self.get(db, uid) else { return Ok(None) };
        let mut sets: Vec<(&str, rusqlite::types::Value)> = vec![];
        if let Some(v) = changes.get("name") {
            let name = check_name(v.as_str().unwrap_or(""))?;
            if self.by_name(db, &name).is_some_and(|o| o["id"] != uid) {
                return Err(t("Den Namen gibt es schon", &[]));
            }
            sets.push(("name", name.into()));
        }
        if let Some(v) = changes.get("role") {
            let role = v.as_str().unwrap_or("");
            if !ROLES.contains(&role) {
                return Err(t("Unbekannte Rolle", &[]));
            }
            if user["role"] == "admin" && role != "admin" && self.admins(db, uid) == 0 {
                return Err(t("Der letzte Admin kann nicht herabgestuft werden", &[]));
            }
            sets.push(("role", role.to_string().into()));
        }
        if let Some(Value::Object(p)) = changes.get("perms") {
            let mut merged = user["perms"].as_object().cloned().unwrap_or_default();
            for (k, v) in p {
                if PERMS.contains(&k.as_str()) {
                    merged.insert(k.clone(), json!(v.as_bool().unwrap_or(false)));
                }
            }
            sets.push(("perms", Value::Object(merged).to_string().into()));
        }
        if let Some(v) = changes.get("quota") {
            sets.push(("quota", quota(v)?.map_or(rusqlite::types::Value::Null, Into::into)));
        }
        let mut disable = false;
        if let Some(v) = changes.get("disabled") {
            let off = v.as_bool().unwrap_or(false);
            if off && user["role"] == "admin" && self.admins(db, uid) == 0 {
                return Err(t("Der letzte Admin kann nicht gesperrt werden", &[]));
            }
            disable = off;
            sets.push(("disabled", i64::from(off).into()));
        }
        if !sets.is_empty() {
            let mut c = db.lock();
            let tx = c.transaction().map_err(|e| e.to_string())?;
            for (col, val) in sets {
                // fixed column names, values bound
                tx.execute(&format!("UPDATE users SET {col}=? WHERE id=?"), params![val, uid])
                    .map_err(|e| e.to_string())?;
            }
            if disable {
                tx.execute("DELETE FROM sessions WHERE user_id=?", [uid])
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        Ok(self.get(db, uid))
    }

    pub fn set_password(&self, db: &Db, uid: i64, password: &str) -> Result<(), String> {
        let user = self.get(db, uid).ok_or_else(|| t("Nicht gefunden", &[]))?;
        check_password(password, user["name"].as_str().unwrap_or(""))?;
        let mut c = db.lock();
        let tx = c.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE users SET pw=?, pw_changed=? WHERE id=?",
            params![hash_password(password), now(), uid],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sessions WHERE user_id=?", [uid])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    /// Ok(false) = no such account.
    pub fn delete(&self, db: &Db, uid: i64) -> Result<bool, String> {
        let Some(user) = self.get(db, uid) else {
            return Ok(false);
        };
        if user["role"] == "admin" && self.admins(db, uid) == 0 {
            return Err(t("Der letzte Admin kann nicht gelöscht werden", &[]));
        }
        db.lock()
            .execute("DELETE FROM users WHERE id=?", [uid])
            .map_err(|e| e.to_string())?;
        Ok(true)
    }

    // -- login ---------------------------------------------------------------------
    pub fn blocked(&self, ip: &str) -> bool {
        let ts = now();
        let mut b = self.brake.lock().unwrap_or_else(|p| p.into_inner());
        match b.ip_fails.get_mut(&ip_bucket(ip)) {
            Some(q) => {
                while q.front().is_some_and(|x| *x < ts - WINDOW) {
                    q.pop_front();
                }
                q.len() >= IP_FAILS
            }
            None => false,
        }
    }

    pub fn fail(&self, ip: &str, name: &str) {
        let ts = now();
        let mut b = self.brake.lock().unwrap_or_else(|p| p.into_inner());
        b.ip_fails.entry(ip_bucket(ip)).or_default().push_back(ts);
        *b.user_fails.entry(name.to_lowercase()).or_default() += 1;
        if b.ip_fails.len() > MAX_TRACKED {
            b.ip_fails.retain(|_, q| q.back().is_some_and(|x| *x >= ts - WINDOW));
        }
        if b.user_fails.len() > MAX_TRACKED {
            b.user_fails.clear();
        }
    }

    /// Growing pause after failed logins of one account (0, 0, 0, 1, 2, 4 … max 8 s).
    pub fn delay_for(&self, name: &str) -> f64 {
        let b = self.brake.lock().unwrap_or_else(|p| p.into_inner());
        let n = b.user_fails.get(&name.to_lowercase()).copied().unwrap_or(0);
        if n < 3 {
            0.0
        } else {
            f64::from(8u32.min(1 << (n - 3).min(4)))
        }
    }

    pub fn authenticate(&self, db: &Db, name: &str, password: &str, ip: &str) -> Result<Value, AuthError> {
        if self.blocked(ip) {
            return Err(AuthError {
                message: t("Zu viele Fehlversuche – bitte in 15 Minuten erneut versuchen", &[]),
                status: 429,
                reason: "ip_blocked",
            });
        }
        let name: String = name.trim().chars().take(NAME_MAX).collect();
        let row: Option<(String, Value)> = if name.is_empty() {
            None
        } else {
            db.lock()
                .query_row("SELECT * FROM users WHERE name=?", [&name], |r| {
                    Ok((r.get::<_, String>("pw")?, public(r)?))
                })
                .optional()
                .ok()
                .flatten()
        };
        let ok = verify_password(password, row.as_ref().map_or(&self.dummy, |r| &r.0));
        let wrong = || AuthError {
            message: t("Name oder Passwort falsch", &[]),
            status: 401,
            reason: "bad_password",
        };
        let unknown = row.is_none();
        let Some((_, user)) = row.filter(|_| ok) else {
            self.fail(ip, &name);
            return Err(AuthError {
                reason: if unknown { "unknown_user" } else { "bad_password" },
                ..wrong()
            });
        };
        if user["disabled"] == true {
            self.fail(ip, &name);
            return Err(AuthError {
                reason: "disabled",
                ..wrong()
            });
        }
        self.brake
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .user_fails
            .remove(&name.to_lowercase());
        let _ = db.lock().execute(
            "UPDATE users SET last_login=? WHERE id=?",
            params![now(), user["id"].as_i64()],
        );
        Ok(user)
    }

    pub fn start_session(&self, db: &Db, uid: i64, persistent: bool, ip: &str, agent: &str) -> String {
        let token = URL_SAFE_NO_PAD.encode(random(32));
        let ts = now();
        let life = if persistent { SESSION_LONG } else { SESSION_SHORT };
        let mut c = db.lock();
        if let Ok(tx) = c.transaction() {
            let _ = tx.execute("DELETE FROM sessions WHERE expires < ?", [ts]);
            let _ = tx.execute(
                "INSERT INTO sessions(token_hash, user_id, created, last_seen, expires, persistent, ip, agent) VALUES(?,?,?,?,?,?,?,?)",
                params![
                    token_hash(&token),
                    uid,
                    ts,
                    ts,
                    ts + life,
                    i64::from(persistent),
                    ip.chars().take(64).collect::<String>(),
                    agent.chars().take(200).collect::<String>()
                ],
            );
            let _ = tx.execute("UPDATE users SET last_seen=? WHERE id=?", params![ts, uid]);
            let _ = tx.commit();
        }
        token
    }

    /// The signed-in user for a cookie token (None if unknown, expired or locked); with ``session`` = id prefix.
    pub fn session_user(&self, db: &Db, token: &str, ip: &str) -> Option<Value> {
        if token.is_empty() || token.len() > 100 {
            return None;
        }
        let th = token_hash(token);
        let row = db
            .lock()
            .query_row(
                "SELECT s.user_id, s.expires, s.last_seen, s.persistent, s.ip, u.disabled FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash=?",
                [&th],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, f64>(2)?,
                        r.get::<_, i64>(3)? != 0,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)? != 0,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()?;
        let (uid, expires, last_seen, persistent, old_ip, disabled) = row;
        let ts = now();
        if expires < ts || disabled {
            return None;
        }
        if ts - last_seen > TOUCH_EVERY {
            let new_expires = if persistent {
                expires.max(ts + SESSION_LONG)
            } else {
                expires
            };
            let ip = if ip.is_empty() {
                old_ip
            } else {
                ip.chars().take(64).collect()
            };
            let c = db.lock();
            let _ = c.execute(
                "UPDATE sessions SET last_seen=?, expires=?, ip=? WHERE token_hash=?",
                params![ts, new_expires, ip, th],
            );
            let _ = c.execute("UPDATE users SET last_seen=? WHERE id=?", params![ts, uid]);
        }
        let mut user = self.get(db, uid)?;
        user["session"] = json!(th[..16]);
        Some(user)
    }

    pub fn end_session(&self, db: &Db, token: &str) {
        let _ = db
            .lock()
            .execute("DELETE FROM sessions WHERE token_hash=?", [token_hash(token)]);
    }

    pub fn sessions(&self, db: &Db, uid: i64) -> Vec<Value> {
        let c = db.lock();
        let mut st = c
            .prepare("SELECT token_hash, created, last_seen, expires, ip, agent FROM sessions WHERE user_id=? AND expires>=? ORDER BY last_seen DESC")
            .expect("valid SQL");
        st.query_map(params![uid, now()], |r| {
            Ok(
                json!({"id": r.get::<_, String>(0)?[..16].to_string(), "created": r.get::<_, f64>(1)?,
                      "last_seen": r.get::<_, f64>(2)?, "expires": r.get::<_, f64>(3)?, "ip": r.get::<_, String>(4)?,
                      "agent": r.get::<_, String>(5)?}),
            )
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn end_sessions(&self, db: &Db, uid: i64, keep: &str) -> usize {
        db.lock()
            .execute(
                "DELETE FROM sessions WHERE user_id=? AND substr(token_hash, 1, 16)<>?",
                params![uid, keep],
            )
            .unwrap_or(0)
    }

    pub fn end_session_id(&self, db: &Db, uid: i64, sid: &str) -> bool {
        db.lock()
            .execute(
                "DELETE FROM sessions WHERE user_id=? AND substr(token_hash, 1, 16)=?",
                params![uid, sid],
            )
            .unwrap_or(0)
            > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_hashes_verify() {
        // made by app/accounts.py hash_password("Schimmelreiter-Kaffee-713") (scrypt n=2^15 r=8 p=1)
        let from_python = "scrypt$32768$8$1$Qm4lF74Elu/HCcpfTHTW2A==$jCpk8rhdtAw24Kv2W7G4qfCooQxVIl+H+2VaGkWO1x4=";
        assert!(verify_password("Schimmelreiter-Kaffee-713", from_python));
        assert!(!verify_password("Schimmelreiter-Kaffee-714", from_python));
        let h = hash_password("Schimmelreiter-Kaffee-713");
        assert!(verify_password("Schimmelreiter-Kaffee-713", &h));
        assert!(!verify_password("falsch", &h));
        assert!(!verify_password("x", "bcrypt$1$2$3$4$5"));
    }

    #[test]
    fn password_rules_like_python() {
        assert!(password_problems("Kaffee Garten Hund Fahrrad", "Eva").is_empty());
        assert!(password_problems("Schimmelreiter-Kaffee-713", "Alt-Admin").is_empty());
        assert!(!password_problems("Pudding2024!", "").is_empty()); // one word with signs around it
        assert!(!password_problems("kurz", "").is_empty());
        assert!(!password_problems("Kaffee Garten Hund Mama", "Mama").is_empty()); // contains the name
        assert!(!password_problems("Passwort-Garten-17", "").is_empty());
    }

    #[test]
    fn sessions_and_brake() {
        let db = Db::memory();
        let a = Accounts::new();
        let u = a
            .create(
                &db,
                "Eva",
                "Kaffee Garten Hund Fahrrad",
                "user",
                &json!({}),
                &Value::Null,
            )
            .unwrap();
        let uid = u["id"].as_i64().unwrap();
        let tok = a.start_session(&db, uid, false, "1.2.3.4", "test");
        assert_eq!(a.session_user(&db, &tok, "").unwrap()["name"], "Eva");
        a.end_session(&db, &tok);
        assert!(a.session_user(&db, &tok, "").is_none());
        for _ in 0..5 {
            assert!(a.authenticate(&db, "Eva", "falsch", "9.9.9.9").is_err());
        }
        assert_eq!(
            a.authenticate(&db, "Eva", "Kaffee Garten Hund Fahrrad", "9.9.9.9")
                .unwrap_err()
                .status,
            429
        );
        assert!(
            a.authenticate(&db, "Eva", "Kaffee Garten Hund Fahrrad", "8.8.8.8")
                .is_ok()
        );
        assert_eq!(ip_bucket("2001:db8:1:2:3:4:5:6"), "2001:db8:1:2::/64");
    }
}
