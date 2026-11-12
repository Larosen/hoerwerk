//! Settings the UI can change; the environment only provides the defaults. Defaults, allowed values and the
//! environment variables come from contract/settings.json – the same file the Python version is checked against.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::db::Db;

const CONTRACT: &str = include_str!("../../../contract/settings.json");
pub const OLD_DEFAULT_FOLDER: &str = "{author}/[{series}/][{series_position} - ]{title}"; // v3.0

pub struct Settings {
    pub defaults: Map<String, Value>,
    pub choices: BTreeMap<String, Vec<String>>,
    pub version: String,
}

fn spec() -> Value {
    serde_json::from_str(CONTRACT).expect("contract/settings.json is valid JSON")
}

/// The commit the image was built from (set by the Dockerfile) – short, hex only; None outside an image.
pub fn commit() -> Option<String> {
    let c: String = std::env::var("HOERWERK_COMMIT")
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(7)
        .collect();
    (c.len() == 7).then_some(c)
}

pub fn version() -> String {
    spec()["version"].as_str().unwrap_or("0").to_string()
}

impl Settings {
    pub fn new(env: &BTreeMap<String, String>) -> Self {
        let spec = spec();
        let mut defaults = spec["defaults"].as_object().cloned().unwrap_or_default();
        for (var, how) in spec["env"].as_object().into_iter().flatten() {
            let Some(raw) = env.get(var) else { continue };
            let key = how["key"].as_str().unwrap_or_default().to_string();
            let value = match how["type"].as_str() {
                Some("bool") => json!(matches!(
                    raw.trim().to_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )),
                Some("int") => match raw.trim().parse::<i64>() {
                    Ok(n) => json!(n),
                    Err(_) => continue,
                },
                Some("float") => match raw.trim().parse::<f64>() {
                    Ok(n) => json!(n),
                    Err(_) => continue,
                },
                _ => json!(if how["lower"].as_bool() == Some(true) {
                    raw.to_lowercase()
                } else {
                    raw.clone()
                }),
            };
            defaults.insert(key, value);
        }
        let choices = spec["choices"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect(),
                )
            })
            .collect();
        Settings {
            defaults,
            choices,
            version: version(),
        }
    }

    pub fn get(&self, db: &Db) -> Map<String, Value> {
        let mut saved = db
            .get("settings")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        if saved.get("ab_folder_template").and_then(Value::as_str) == Some(OLD_DEFAULT_FOLDER) {
            saved.remove("ab_folder_template"); // saved with the v3.0 default: follow the new default
        }
        let mut out = self.defaults.clone();
        for (k, v) in saved {
            if out.contains_key(&k) {
                out.insert(k, v);
            }
        }
        out
    }

    pub fn update(&self, db: &Db, changes: Map<String, Value>) -> rusqlite::Result<Map<String, Value>> {
        let mut saved = db
            .get("settings")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        saved.extend(changes);
        db.put("settings", &Value::Object(saved))?;
        Ok(self.get(db))
    }

    /// Clean a change from the browser like the Python version: unknown keys dropped, types as the default's,
    /// allowed values checked. Err(key) for a value that does not fit.
    pub fn clean(&self, data: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let mut out = Map::new();
        for (k, v) in data {
            let Some(default) = self.defaults.get(k) else { continue };
            let value = match default {
                Value::String(_) => match v {
                    Value::String(s) => json!(s),
                    Value::Null => json!(""),
                    other => json!(other.to_string()),
                },
                Value::Bool(_) => match v {
                    Value::Bool(b) => json!(b),
                    Value::Number(n) => json!(n.as_f64() != Some(0.0)),
                    Value::String(s) => json!(!s.is_empty()),
                    _ => return Err(k.clone()),
                },
                Value::Number(n) if n.is_i64() || n.is_u64() => match v {
                    Value::Number(x) => json!(x.as_f64().map(|f| f.trunc() as i64).ok_or(k.clone())?),
                    Value::String(s) => json!(s.trim().parse::<i64>().map_err(|_| k.clone())?),
                    Value::Bool(b) => json!(i64::from(*b)),
                    _ => return Err(k.clone()),
                },
                Value::Number(_) => match v {
                    Value::Number(x) => json!(x.as_f64().ok_or(k.clone())?),
                    Value::String(s) => json!(s.trim().parse::<f64>().map_err(|_| k.clone())?),
                    _ => return Err(k.clone()),
                },
                _ => v.clone(),
            };
            out.insert(k.clone(), value);
        }
        if let Some(Value::String(s)) = out.get_mut("discover_trusted_authors") {
            *s = s.chars().take(4000).collect();
        }
        for (k, allowed) in &self.choices {
            if let Some(v) = out.get(k)
                && !allowed.iter().any(|a| Some(a.as_str()) == v.as_str())
            {
                return Err(k.clone());
            }
        }
        Ok(out)
    }
}

/// ``https://host[:port]`` (no path, no trailing slash) or "" – anything else is no usable address.
pub fn public_url(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let with_scheme = if text.contains("://") {
        text.to_string()
    } else {
        format!("https://{text}")
    };
    let Some((scheme, rest)) = with_scheme.split_once("://") else {
        return String::new();
    };
    let scheme = scheme.to_lowercase();
    if scheme != "http" && scheme != "https" {
        return String::new();
    }
    let authority = rest.trim_end_matches('/');
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', ' ']) {
        return String::new();
    }
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        match stripped.split_once(']') {
            Some((h, p)) => (format!("[{h}]"), p.strip_prefix(':').map(String::from)),
            None => return String::new(),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.to_string())),
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty() || host.trim_matches(['[', ']']).is_empty() {
        return String::new();
    }
    let host = host.to_lowercase();
    match port {
        Some(p) if !p.is_empty() => match p.parse::<u16>() {
            Ok(p) => format!("{scheme}://{host}:{p}"),
            Err(_) => String::new(),
        },
        Some(_) => String::new(),
        None => format!("{scheme}://{host}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_environment() {
        let env: BTreeMap<String, String> = [
            ("MATCH_MODE".into(), "off".into()),
            ("SETTLE_SECONDS".into(), "0".into()),
            ("AUDIOBOOK_WATCH".into(), "no".into()),
        ]
        .into();
        let s = Settings::new(&env);
        assert_eq!(s.defaults["ab_match"], "off");
        assert_eq!(s.defaults["ab_settle"], 0);
        assert_eq!(s.defaults["ab_watch"], false);
        assert_eq!(s.defaults["audio_quality"], "high");
    }

    #[test]
    fn cleaning_like_python() {
        let s = Settings::new(&BTreeMap::new());
        let ok = s
            .clean(&serde_json::from_str(r#"{"audio_quality":"original","ab_settle":"30","x":1}"#).unwrap())
            .unwrap();
        assert_eq!(ok["audio_quality"], "original");
        assert_eq!(ok["ab_settle"], 30);
        assert!(!ok.contains_key("x"));
        assert!(
            s.clean(&serde_json::from_str(r#"{"audio_quality":"laut"}"#).unwrap())
                .is_err()
        );
        assert!(
            s.clean(&serde_json::from_str(r#"{"ab_region":"xx"}"#).unwrap())
                .is_err()
        );
    }

    #[test]
    fn public_addresses() {
        assert_eq!(public_url("hoerbuecher.example.de"), "https://hoerbuecher.example.de");
        assert_eq!(public_url("https://HB.example.de:8443/"), "https://hb.example.de:8443");
        for bad in [
            "javascript:alert(1)",
            "https://x.de/pfad",
            "ftp://x.de",
            "https://user@x.de",
            "https://x.de:99999",
        ] {
            assert_eq!(public_url(bad), "", "{bad}");
        }
    }
}
