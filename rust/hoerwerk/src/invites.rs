//! Invitations by link: an admin creates a link, whoever opens it picks their own name and password.
//! Accepting is one transaction – account created and use counted together (same rules as app/invites.py).

use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};

use crate::accounts::{self, PERMS};
use crate::db::{Db, compact};
use crate::text::{now, t};

const MAX_USES: i64 = 100;
const MAX_DAYS: i64 = 365;
const NOTE_MAX: usize = 60;

pub fn valid_token_format(token: &str) -> bool {
    (20..=64).contains(&token.len()) && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn gone() -> String {
    t(
        "Diese Einladung gilt nicht (mehr). Bitte frag nach einem neuen Link.",
        &[],
    )
}

fn public(r: &Row) -> rusqlite::Result<Value> {
    let data: Value = serde_json::from_str(&r.get::<_, String>("data")?).unwrap_or(json!({}));
    let (revoked, uses, max_uses, expires): (i64, i64, i64, Option<f64>) =
        (r.get("revoked")?, r.get("uses")?, r.get("max_uses")?, r.get("expires")?);
    let status = if revoked != 0 {
        "revoked"
    } else if max_uses != 0 && uses >= max_uses {
        "used"
    } else if expires.is_some_and(|e| e < now()) {
        "expired"
    } else {
        "open"
    };
    Ok(
        json!({"id": r.get::<_, i64>("id")?, "token": r.get::<_, String>("token")?, "note": r.get::<_, String>("note")?,
              "created": r.get::<_, f64>("created")?, "created_by": r.get::<_, String>("created_by")?, "expires": expires,
              "max_uses": max_uses, "uses": uses, "quota": data.get("quota").cloned().unwrap_or(Value::Null),
              "perms": data.get("perms").cloned().unwrap_or(json!({})), "used_by": data.get("used_by").cloned().unwrap_or(json!([])),
              "status": status}),
    )
}

pub fn get(db: &Db, id: i64) -> Option<Value> {
    db.lock()
        .query_row("SELECT * FROM invites WHERE id=?", [id], public)
        .optional()
        .ok()
        .flatten()
}

pub fn list(db: &Db) -> Vec<Value> {
    let c = db.lock();
    let mut st = c
        .prepare("SELECT * FROM invites ORDER BY id DESC LIMIT 200")
        .expect("valid SQL");
    st.query_map([], public)
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn int(v: &Value, default: i64) -> Option<i64> {
    match v {
        Value::Null => Some(default),
        Value::String(s) if s.is_empty() => Some(default),
        Value::Number(n) => n.as_f64().map(|f| f as i64),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn create(db: &Db, by: &str, body: &Value) -> Result<Value, String> {
    let bad = || t("Ungültige Daten", &[]);
    let note: String = body["note"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NOTE_MAX)
        .collect();
    let days = int(body.get("days").unwrap_or(&json!(7)), 7).ok_or_else(bad)?;
    let uses = int(body.get("uses").unwrap_or(&json!(1)), 1).ok_or_else(bad)?;
    if !(0..=MAX_DAYS).contains(&days) || !(0..=MAX_USES).contains(&uses) {
        return Err(bad());
    }
    let quota = match &body["quota"] {
        Value::Null => Value::Null,
        Value::String(s) if s.is_empty() => Value::Null,
        v => {
            let q = int(v, 0).ok_or_else(bad)?;
            if !(0..=1000).contains(&q) {
                return Err(bad());
            }
            json!(q)
        }
    };
    let perms = match &body["perms"] {
        Value::Null => json!({}),
        Value::Object(_) => body["perms"].clone(),
        _ => return Err(bad()),
    };
    let clean: serde_json::Map<String, Value> = PERMS
        .iter()
        .map(|p| {
            (
                (*p).to_string(),
                json!(perms.get(*p).and_then(Value::as_bool).unwrap_or(false)),
            )
        })
        .collect();
    let ts = now();
    let token = {
        use base64::Engine;
        use rand::RngCore;
        let mut b = [0u8; 24];
        rand::rng().fill_bytes(&mut b);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    };
    let id = {
        let c = db.lock();
        c.execute(
            "INSERT INTO invites(token, note, created, created_by, expires, max_uses, data) VALUES(?,?,?,?,?,?,?)",
            params![
                token,
                note,
                ts,
                by,
                (days > 0).then_some(ts + days as f64 * 86400.0),
                uses,
                compact(&json!({"quota": quota, "perms": clean, "used_by": []}))
            ],
        )
        .map_err(|e| e.to_string())?;
        c.last_insert_rowid()
    };
    get(db, id).ok_or_else(bad)
}

pub fn revoke(db: &Db, id: i64) -> Option<Value> {
    let n = db
        .lock()
        .execute("UPDATE invites SET revoked=1 WHERE id=?", [id])
        .unwrap_or(0);
    if n == 0 { None } else { get(db, id) }
}

pub fn delete(db: &Db, id: i64) -> bool {
    db.lock().execute("DELETE FROM invites WHERE id=?", [id]).unwrap_or(0) > 0
}

/// The open invitation for this link, else the one message for every reason (nothing leaks).
pub fn check(db: &Db, token: &str) -> Result<Value, String> {
    if !valid_token_format(token) {
        return Err(gone());
    }
    let inv = db
        .lock()
        .query_row("SELECT * FROM invites WHERE token=?", [token], public)
        .optional()
        .ok()
        .flatten();
    match inv {
        Some(i) if i["status"] == "open" => Ok(i),
        _ => Err(gone()),
    }
}

/// Err((true, msg)) = the link does not work; Err((false, msg)) = name/password refused, the link stays valid.
pub fn accept(
    db: &Db,
    accounts: &accounts::Accounts,
    token: &str,
    name: &str,
    password: &str,
) -> Result<Value, (bool, String)> {
    let invite = check(db, token).map_err(|e| (true, e))?;
    let user = accounts
        .create(db, name, password, "user", &invite["perms"], &invite["quota"])
        .map_err(|e| (false, e))?;
    let uid = user["id"].as_i64().unwrap_or(0);
    let origin = match invite["note"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => t("Einladung #{n}", &[("n", &invite["id"].to_string())]),
    };
    let mut used_by = invite["used_by"].as_array().cloned().unwrap_or_default();
    used_by.push(json!({"name": user["name"], "time": now()}));
    let keep = used_by.len().saturating_sub(MAX_USES as usize);
    let data = compact(&json!({"quota": invite["quota"], "perms": invite["perms"], "used_by": used_by[keep..]}));
    let counted = {
        let c = db.lock();
        let _ = c.execute(
            "UPDATE users SET origin=? WHERE id=?",
            params![origin.chars().take(80).collect::<String>(), uid],
        );
        c.execute(
            "UPDATE invites SET uses=uses+1, data=? WHERE id=? AND (max_uses=0 OR uses<max_uses) AND revoked=0",
            params![data, invite["id"].as_i64()],
        )
        .unwrap_or(0)
    };
    if counted != 1 {
        // used up in the meantime: the account goes again
        let _ = db.lock().execute("DELETE FROM users WHERE id=?", [uid]);
        return Err((true, gone()));
    }
    let mut user = user;
    user["origin"] = json!(origin);
    Ok(user)
}
