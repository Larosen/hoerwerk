//! Search Newznab indexers and send NZBs to SABnzbd – the Readarr part. API keys stay on the server:
//! the UI only sees whether one is set, and NZB links (which embed the indexer key) never leave the backend.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use rand::RngCore;
use serde_json::{Map, Value, json};

use crate::db::{Db, compact};
use crate::http;
use crate::rx;
use crate::rx::{is_match, search, sub};
use crate::text::{now, t};
use crate::words::match_words;

const NEWZNAB_NS: &str = "http://www.newznab.com/DTD/2010/feeds/attributes/";
pub const DEFAULT_CATEGORIES: &str = "3030";
const RATE_PAUSE: f64 = 15.0 * 60.0;
const MAX_RESPONSE: u64 = 20 * 1024 * 1024;
const CAPS_TTL: f64 = 24.0 * 3600.0;
const SECRET: &str = "apikey";

#[derive(Debug)]
pub enum UsenetError {
    Failed(String),
    /// HTTP 429: the indexer wants a break – seconds (its Retry-After, else RATE_PAUSE)
    RateLimited(String, f64),
}

impl std::fmt::Display for UsenetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsenetError::Failed(m) | UsenetError::RateLimited(m, _) => write!(f, "{m}"),
        }
    }
}

type Result<T> = std::result::Result<T, UsenetError>;

fn fail<T>(m: impl Into<String>) -> Result<T> {
    Err(UsenetError::Failed(m.into()))
}

fn retry_after(value: Option<&str>) -> f64 {
    let Some(v) = value.filter(|v| !v.is_empty()) else {
        return RATE_PAUSE;
    };
    if let Ok(secs) = v.trim().parse::<f64>() {
        return secs.clamp(30.0, 6.0 * 3600.0);
    }
    match parse_http_date(v) {
        Some(when) => (when - now()).clamp(30.0, 6.0 * 3600.0),
        None => RATE_PAUSE,
    }
}

/// GET an indexer or SABnzbd address (blocking).
pub type Getter = dyn Fn(&str) -> Result<Vec<u8>> + Send + Sync;

pub fn http_get(url: &str) -> Result<Vec<u8>> {
    let r = http::get(url)
        .timeout(25)
        .limit(MAX_RESPONSE + 1)
        .send()
        .map_err(|e| UsenetError::Failed(t("nicht erreichbar ({reason})", &[("reason", &e)])))?;
    if r.status == 429 {
        return Err(UsenetError::RateLimited(
            "HTTP 429".into(),
            retry_after(r.retry_after.as_deref()),
        ));
    }
    if !r.ok() {
        return fail(format!("HTTP {}", r.status));
    }
    if r.body.len() as u64 > MAX_RESPONSE {
        return fail(t("Antwort zu groß", &[]));
    }
    Ok(r.body)
}

fn redact(text: &str) -> String {
    sub(rx!(r"(?i)(apikey|api_key|r|i)=[0-9a-z]{6,}"), text, "$1=…")
}

// -- release names --------------------------------------------------------------------------------------

/// What a release name tells us: format, language, bitrate, abridged …
pub fn release_info(title: &str) -> Value {
    let t = title.replace(['_', '.'], " ");
    let fmt = ["M4B", "M4A", "MP3", "FLAC", "AAC", "OPUS", "OGG"]
        .into_iter()
        .find(|f| fancy_regex::Regex::new(&format!(r"(?i)\b{f}\b")).is_ok_and(|re| is_match(&re, &t)));
    let lang = if is_match(rx!(r"(?i)\b(german|deutsch|ger)\b|(?<![a-z])DE(?![a-z])"), &t) {
        json!("de")
    } else if is_match(rx!(r"(?i)\b(english|eng)\b|(?<![a-z])EN(?![a-z])"), &t) {
        json!("en")
    } else {
        Value::Null
    };
    let br = search(rx!(r"(?i)\b(\d{2,3})\s*k(?:bps|bit)?\b"), &t).and_then(|c| c[1].parse::<i64>().ok());
    json!({
        "format": fmt,
        "language": lang,
        "bitrate": br,
        "abridged": is_match(rx!(r"(?i)\bgek(ü|ue)rzt|\babridged"), &t) && !is_match(rx!(r"(?i)ungek(ü|ue)rzt|unabridged"), &t),
        "drama": is_match(rx!(r"(?i)h(ö|oe)rspiel|dramati[sz]"), &t),
    })
}

fn digraph(c: char) -> Option<&'static str> {
    Some(match c {
        'ä' => "ae",
        'ö' => "oe",
        'ü' => "ue",
        'Ä' => "Ae",
        'Ö' => "Oe",
        'Ü' => "Ue",
        'ß' => "ss",
        'ø' => "oe",
        'Ø' => "Oe",
        'å' => "aa",
        'Å' => "Aa",
        'æ' => "ae",
        'Æ' => "Ae",
        'œ' => "oe",
        'Œ' => "Oe",
        _ => return None,
    })
}

fn special(c: char) -> Option<&'static str> {
    Some(match c {
        'ø' => "o",
        'Ø' => "O",
        'æ' => "ae",
        'Æ' => "Ae",
        'œ' => "oe",
        'Œ' => "Oe",
        'ł' => "l",
        'Ł' => "L",
        'đ' => "d",
        'Đ' => "D",
        'ð' => "d",
        'þ' => "th",
        'Þ' => "Th",
        'ı' => "i",
        _ => return None,
    })
}

fn map_chars(s: &str, f: fn(char) -> Option<&'static str>) -> String {
    s.chars()
        .map(|c| f(c).map(str::to_string).unwrap_or_else(|| c.to_string()))
        .collect()
}

fn no_accents(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    use unicode_normalization::char::canonical_combining_class;
    map_chars(s, special)
        .nfkd()
        .filter(|c| canonical_combining_class(*c) == 0)
        .collect()
}

/// Release names spell "Fräulein" as "Fräulein", "Fraeulein" or "Fraulein" – search every spelling.
pub fn query_variants(query: &str) -> Vec<String> {
    let q = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = vec![q.clone()];
    if !q.is_ascii() {
        out.push(no_accents(&map_chars(&q, digraph)));
        let plain: String = q
            .replace('ß', "ss")
            .chars()
            .map(|c| match c {
                'ä' => 'a',
                'ö' => 'o',
                'ü' => 'u',
                'Ä' => 'A',
                'Ö' => 'O',
                'Ü' => 'U',
                x => x,
            })
            .collect();
        out.push(no_accents(&plain));
    }
    let mut seen = Vec::new();
    for v in out {
        if !seen.contains(&v) {
            seen.push(v);
        }
    }
    seen
}

pub fn api_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let base = if is_match(rx!(r"(?i)^https?://"), base) {
        base.to_string()
    } else {
        format!("http://{base}")
    };
    if base.to_lowercase().ends_with("/api") {
        base
    } else {
        format!("{base}/api")
    }
}

fn http_url(url: Option<&str>) -> Option<String> {
    url.filter(|u| is_match(rx!(r#"(?i)^https?://[^\s"'<>]+$"#), u))
        .map(str::to_string)
}

/// Name SABnzbd gets for the download folder: no paths, no hidden or relative names.
pub fn safe_release_name(title: &str) -> String {
    let name = sub(rx!(r#"[\\/:*?"<>|\x00-\x1f]"#), title, "_");
    let name = name.trim_matches([' ', '.']);
    let name: String = name.chars().take(200).collect();
    if name.is_empty() { "download".into() } else { name }
}

/// Parse an indexer answer; DTDs/entities are refused (entity-expansion bombs, XXE).
fn xml_doc(data: &[u8]) -> Result<String> {
    let text = String::from_utf8_lossy(data).into_owned();
    if is_match(rx!(r"(?i)<!(DOCTYPE|ENTITY)"), &text) {
        return fail(t("Antwort enthält eine DTD – abgelehnt", &[]));
    }
    Ok(text)
}

fn parse(text: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(text).map_err(|_| UsenetError::Failed(t("Antwort ist kein Newznab-XML", &[])))
}

fn check_error(root: roxmltree::Node) -> Result<()> {
    if root.tag_name().name() == "error" {
        let code = root.attribute("code").unwrap_or("");
        return fail(
            root.attribute("description")
                .map(str::to_string)
                .unwrap_or_else(|| t("Fehler {code}", &[("code", code)])),
        );
    }
    Ok(())
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace().is_none())
        .map(|c| c.text().unwrap_or(""))
}

/// RFC 2822 dates ("Mon, 02 Jan 2026 15:04:05 +0100") -> unix time.
pub fn parse_http_date(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = s.split_once(',').map_or(s, |x| x.1).trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    let day: i64 = parts[0].parse().ok()?;
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| parts[1].to_lowercase().starts_with(m))? as i64
        + 1;
    let mut year: i64 = parts[2].parse().ok()?;
    if year < 100 {
        year += if year < 50 { 2000 } else { 1900 };
    }
    let hms: Vec<i64> = parts[3].split(':').map(|x| x.parse().unwrap_or(0)).collect();
    let (h, m, sec) = (
        hms.first().copied()?,
        hms.get(1).copied().unwrap_or(0),
        hms.get(2).copied().unwrap_or(0),
    );
    let zone = parts.get(4).copied().unwrap_or("+0000");
    let offset = if let Some(z) = zone.strip_prefix('+').or_else(|| zone.strip_prefix('-')) {
        let n: i64 = z.parse().ok()?;
        let secs = (n / 100) * 3600 + (n % 100) * 60;
        if zone.starts_with('-') { -secs } else { secs }
    } else {
        0
    };
    let days = crate::text::days_from_civil(year, month as u32, day as u32);
    Some((days * 86400 + h * 3600 + m * 60 + sec - offset) as f64)
}

pub fn parse_results(xml: &[u8], indexer: &str) -> Result<Vec<Value>> {
    let text = xml_doc(xml)?;
    let doc = parse(&text)?;
    check_error(doc.root_element())?;
    let mut out = Vec::new();
    for item in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "item" && n.tag_name().namespace().is_none())
    {
        let attrs: Vec<(String, String)> = item
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "attr" && c.tag_name().namespace() == Some(NEWZNAB_NS))
            .map(|c| {
                (
                    c.attribute("name").unwrap_or("").to_string(),
                    c.attribute("value").unwrap_or("").to_string(),
                )
            })
            .collect();
        let attr = |k: &str| attrs.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let cats: Vec<String> = attrs
            .iter()
            .filter(|(n, v)| n == "category" && !v.is_empty())
            .map(|(_, v)| v.clone())
            .collect();
        let enc = item
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "enclosure");
        let link = enc
            .and_then(|e| e.attribute("url"))
            .filter(|u| !u.is_empty())
            .or_else(|| child_text(item, "link"));
        let Some(link) = http_url(link) else { continue };
        let size = attr("size")
            .filter(|s| !s.is_empty())
            .or_else(|| enc.and_then(|e| e.attribute("length")).map(str::to_string))
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let published = child_text(item, "pubDate")
            .filter(|p| !p.is_empty())
            .and_then(parse_http_date);
        let title = child_text(item, "title").unwrap_or("").trim().to_string();
        let details = [
            http_url(child_text(item, "comments")),
            http_url(child_text(item, "guid")),
        ]
        .into_iter()
        .flatten()
        .find(|u| *u != link);
        let grabs = attr("grabs")
            .filter(|g| !g.is_empty() && g.chars().all(|c| c.is_ascii_digit()))
            .and_then(|g| g.parse::<i64>().ok());
        let mut r = json!({
            "title": title, "link": link,
            "size": size, "published": published, "grabs": grabs,
            "category": attr("category").filter(|c| !c.is_empty()).or_else(|| child_text(item, "category").map(str::to_string)),
            "categories": cats, "category_name": child_text(item, "category").unwrap_or("").trim(),
            // the details page only – never the NZB link itself (it carries the indexer's key)
            "details": details,
            "indexer": indexer,
        });
        if let (Some(o), Some(info)) = (r.as_object_mut(), release_info(&title).as_object()) {
            o.extend(info.clone());
        }
        out.push(r);
    }
    Ok(out)
}

fn audiobook_name(name: &str) -> bool {
    is_match(
        rx!(r"(?i)audio\s*-?\s*book|h(?:ö|oe|o)rbu(?:ch|echer|cher)|h(?:ö|oe|o)rspiel|spoken|abook"),
        name,
    )
}

fn audio_name(name: &str) -> bool {
    is_match(rx!(r"(?i)\baudio\b|\bmusi[ck]\b|\bmp3\b|lossless|flac"), name)
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Category ids where an indexer files audiobooks, read from its capabilities (t=caps).
pub fn audiobook_categories(caps_xml: &[u8]) -> Result<Vec<String>> {
    let text = xml_doc(caps_xml)?;
    let doc = parse(&text)?;
    let mut out: Vec<String> = Vec::new();
    for cat in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "category")
    {
        let (cid, name) = (cat.attribute("id").unwrap_or(""), cat.attribute("name").unwrap_or(""));
        let audio_main =
            digits(cid) && (cid.parse::<i64>().is_ok_and(|n| (3000..4000).contains(&n)) || audio_name(name));
        if digits(cid) && (audio_main || audiobook_name(name)) {
            out.push(cid.to_string());
        }
        for subcat in cat
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "subcat")
        {
            let sid = subcat.attribute("id").unwrap_or("");
            if digits(sid) && (audio_main || audiobook_name(subcat.attribute("name").unwrap_or(""))) {
                out.push(sid.to_string());
            }
        }
    }
    let mut seen = Vec::new();
    for c in out {
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    Ok(seen)
}

/// Result from a search over all categories: keep it only if it is in the audio area.
pub fn is_audio(result: &Value, cats: &[String]) -> bool {
    let ids: Vec<String> = match result["categories"].as_array().filter(|a| !a.is_empty()) {
        Some(a) => a
            .iter()
            .map(|c| c.as_str().map(str::to_string).unwrap_or_else(|| c.to_string()))
            .collect(),
        None => result["category"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(|c| vec![c.to_string()])
            .unwrap_or_default(),
    };
    if ids
        .iter()
        .any(|c| cats.contains(c) || (digits(c) && c.parse::<i64>().is_ok_and(|n| (3000..4000).contains(&n))))
    {
        return true;
    }
    let name = result["category_name"].as_str().unwrap_or("");
    if !name.is_empty() {
        return audiobook_name(name) || audio_name(name);
    }
    ids.is_empty()
}

// -- Newznab and SABnzbd ----------------------------------------------------------------------------

/// indexer url -> (when asked, its audiobook categories)
type Caps = HashMap<String, (f64, Vec<String>)>;
static CAPS: LazyLock<Mutex<Caps>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn encode(params: &[(&str, String)]) -> String {
    params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", form(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// urlencode like Python (space -> +)
fn form(v: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b' ');
    percent_encoding::utf8_percent_encode(v, SET)
        .to_string()
        .replace(' ', "+")
}

pub struct Newznab<'a> {
    pub name: String,
    url: String,
    apikey: String,
    categories: String,
    get: &'a Getter,
}

impl<'a> Newznab<'a> {
    pub fn new(name: &str, url: &str, apikey: &str, categories: &str, get: &'a Getter) -> Self {
        let cats: String = categories.chars().filter(|c| c.is_ascii_digit() || *c == ',').collect();
        Newznab {
            name: name.into(),
            url: api_url(url),
            apikey: apikey.into(),
            categories: if cats.is_empty() {
                DEFAULT_CATEGORIES.into()
            } else {
                cats
            },
            get,
        }
    }

    fn call(&self, params: &[(&str, String)]) -> Result<Vec<u8>> {
        let mut p: Vec<(&str, String)> = params.to_vec();
        p.push(("apikey", self.apikey.clone()));
        match (self.get)(&format!("{}?{}", self.url, encode(&p))) {
            Err(UsenetError::RateLimited(m, w)) => {
                Err(UsenetError::RateLimited(format!("{}: {}", self.name, redact(&m)), w))
            }
            Err(UsenetError::Failed(m)) => fail(format!("{}: {}", self.name, redact(&m))),
            ok => ok,
        }
    }

    pub fn test(&self) -> Result<String> {
        let raw = self.call(&[("t", "caps".into())])?;
        let text = xml_doc(&raw)?;
        let doc = parse(&text)?;
        check_error(doc.root_element())?;
        let title = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "server")
            .and_then(|s| s.attribute("title"))
            .filter(|s| !s.is_empty())
            .unwrap_or("OK")
            .to_string();
        let found = audiobook_categories(&raw)?;
        CAPS.lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(self.url.clone(), (now(), found.clone()));
        let mut cats: Vec<String> = self
            .categories
            .split(',')
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .collect();
        for c in found {
            if !cats.contains(&c) {
                cats.push(c);
            }
        }
        Ok(t(
            "{title} · Hörbuch-Kategorien: {cats}",
            &[("title", &title), ("cats", &cats.join(", "))],
        ))
    }

    /// The configured categories plus the indexer's own audiobook categories (asked once a day).
    pub fn audiobook_cats(&self) -> Vec<String> {
        let mut own: Vec<String> = self
            .categories
            .split(',')
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .collect();
        let hit = CAPS.lock().unwrap_or_else(|p| p.into_inner()).get(&self.url).cloned();
        let found = match hit {
            Some((at, f)) if now() - at <= CAPS_TTL => f,
            _ => {
                let f = self
                    .call(&[("t", "caps".into())])
                    .and_then(|raw| audiobook_categories(&raw))
                    .unwrap_or_default();
                CAPS.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(self.url.clone(), (now(), f.clone()));
                f
            }
        };
        for c in found.into_iter().chain(["3000".to_string()]) {
            if !own.contains(&c) {
                own.push(c);
            }
        }
        own
    }

    /// Results reach the browser (without "link"): a details address that carries the key goes.
    fn scrub(&self, mut results: Vec<Value>) -> Vec<Value> {
        for r in &mut results {
            if !self.apikey.is_empty() && r["details"].as_str().is_some_and(|d| d.contains(&self.apikey)) {
                r["details"] = Value::Null;
            }
        }
        results
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Value>> {
        let cats = self.audiobook_cats();
        let found = self.scrub(parse_results(
            &self.call(&[
                ("t", "search".into()),
                ("q", query.into()),
                ("cat", cats.join(",")),
                ("limit", limit.to_string()),
                ("extended", "1".into()),
            ])?,
            &self.name,
        )?);
        if !found.is_empty() {
            return Ok(found);
        }
        // nothing in the audiobook categories: many indexers file German audiobooks elsewhere
        let everything = self.scrub(parse_results(
            &self.call(&[
                ("t", "search".into()),
                ("q", query.into()),
                ("limit", limit.to_string()),
                ("extended", "1".into()),
            ])?,
            &self.name,
        )?);
        Ok(everything.into_iter().filter(|r| is_audio(r, &cats)).collect())
    }
}

pub struct Sabnzbd<'a> {
    url: String,
    apikey: String,
    category: String,
    get: &'a Getter,
}

impl<'a> Sabnzbd<'a> {
    pub fn new(url: &str, apikey: &str, category: &str, get: &'a Getter) -> Self {
        let url = url.trim().trim_end_matches('/');
        let url = if is_match(rx!(r"(?i)^https?://"), url) {
            url.to_string()
        } else {
            format!("http://{url}")
        };
        let url = if url.ends_with("/api") {
            url
        } else {
            format!("{url}/api")
        };
        Sabnzbd {
            url,
            apikey: apikey.into(),
            category: category.trim().into(),
            get,
        }
    }

    fn call(&self, params: &[(&str, String)]) -> Result<Value> {
        let mut p: Vec<(&str, String)> = params.to_vec();
        p.push(("apikey", self.apikey.clone()));
        p.push(("output", "json".into()));
        let raw = (self.get)(&format!("{}?{}", self.url, encode(&p)))
            .map_err(|e| UsenetError::Failed(format!("SABnzbd: {}", redact(&e.to_string()))))?;
        let data: Value =
            serde_json::from_slice(&raw).map_err(|_| UsenetError::Failed(t("SABnzbd: unerwartete Antwort", &[])))?;
        if data["status"] == false && data["error"].as_str().is_some_and(|e| !e.is_empty()) {
            return fail(format!("SABnzbd: {}", data["error"].as_str().unwrap_or("")));
        }
        Ok(data)
    }

    pub fn test(&self) -> Result<String> {
        let version = self.call(&[("mode", "version".into())])?["version"].clone();
        let cats: Vec<String> = self.call(&[("mode", "get_cats".into())])?["categories"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if !self.category.is_empty() && !cats.contains(&self.category) {
            return fail(t(
                "SABnzbd: Kategorie „{category}“ gibt es nicht (vorhanden: {available})",
                &[("category", &self.category), ("available", &cats.join(", "))],
            ));
        }
        Ok(format!(
            "SABnzbd {}",
            version
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| version.to_string())
        ))
    }

    pub fn add(&self, nzb_url: &str, name: &str) -> Result<String> {
        let data = self.call(&[
            ("mode", "addurl".into()),
            ("name", nzb_url.into()),
            ("nzbname", name.into()),
            ("cat", self.category.clone()),
        ])?;
        let id = data["nzo_ids"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        match id {
            Some(i) if data["status"].as_bool().unwrap_or(false) => Ok(i),
            _ => fail(t("SABnzbd hat den Download nicht angenommen", &[])),
        }
    }

    pub fn history_ids(&self, limit: usize) -> Result<Vec<String>> {
        let hist = self.call(&[
            ("mode", "history".into()),
            ("limit", limit.to_string()),
            ("category", self.category.clone()),
        ])?;
        Ok(hist["history"]["slots"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s["nzo_id"].as_str().filter(|x| !x.is_empty()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Take finished downloads of our category out of SABnzbd's history – the files stay (del_files=0).
    pub fn remove_history(&self, ids: &[String]) -> Result<usize> {
        let ours = self.history_ids(500)?;
        let mut todo: Vec<String> = Vec::new();
        for i in ids {
            if ours.contains(i) && !todo.contains(i) {
                todo.push(i.clone());
            }
        }
        for chunk in todo.chunks(50) {
            self.call(&[
                ("mode", "history".into()),
                ("name", "delete".into()),
                ("value", chunk.join(",")),
                ("del_files", "0".into()),
            ])?;
        }
        Ok(todo.len())
    }

    pub fn activity(&self, limit: usize) -> Result<Value> {
        let queue = self.call(&[("mode", "queue".into()), ("limit", "50".into())])?["queue"].clone();
        let hist = self.call(&[
            ("mode", "history".into()),
            ("limit", limit.to_string()),
            ("category", self.category.clone()),
        ])?["history"]
            .clone();
        let mut items = Vec::new();
        for s in queue["slots"].as_array().cloned().unwrap_or_default() {
            if !self.category.is_empty() && !(s["cat"].is_null() || s["cat"].as_str() == Some(&self.category)) {
                continue;
            }
            let pct = s["percentage"]
                .as_str()
                .and_then(|p| p.parse::<f64>().ok())
                .or_else(|| s["percentage"].as_f64())
                .unwrap_or(0.0);
            items.push(
                json!({"id": s["nzo_id"], "name": s["filename"], "status": s["status"], "progress": pct / 100.0,
                              "size": s["size"], "left": s["timeleft"], "done": false}),
            );
        }
        for s in hist["slots"].as_array().cloned().unwrap_or_default() {
            let error = s["fail_message"]
                .as_str()
                .filter(|m| !m.is_empty())
                .map(Value::from)
                .unwrap_or(Value::Null);
            items.push(json!({"id": s["nzo_id"], "name": s["name"], "status": s["status"],
                              "progress": if s["status"] == "Completed" { 1.0 } else { 0.0 }, "size": s["size"], "error": error,
                              "path": s["storage"], "done": true, "finished": s["completed"]}));
        }
        Ok(json!({"paused": queue["paused"].as_bool().unwrap_or(false), "speed": queue["speed"], "items": items}))
    }
}

// -- configuration + search service ------------------------------------------------------------------

/// scheme://host:port of a configured URL – keys are bound to it.
fn host(url: &str) -> String {
    let u = url.trim();
    if u.is_empty() {
        return String::new();
    }
    let u = if is_match(rx!(r"(?i)^https?://"), u) {
        u.to_string()
    } else {
        format!("http://{u}")
    };
    http::origin(&u)
}

fn hostname(url: &str) -> String {
    http::origin(url)
        .split("://")
        .nth(1)
        .and_then(|h| h.rsplit_once(':'))
        .map(|x| x.0.to_string())
        .unwrap_or_default()
}

fn truthy(v: Option<&Value>, default: bool) -> bool {
    match v {
        None => default,
        Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Indexers + SABnzbd (stored in the database, keys never leave the server), search cache, grab log.
pub struct Usenet {
    db: Db,
    get: Box<Getter>,
    lock: Mutex<()>,
    pauses: Mutex<HashMap<String, f64>>,
    results: Mutex<HashMap<String, Value>>,
}

fn dedupe_key(r: &Value) -> String {
    let title = r["title"].as_str().unwrap_or("").to_lowercase();
    format!(
        "{}{}",
        sub(rx!(r"[\W_]+"), &title, ""),
        r["size"].as_i64().unwrap_or(0) >> 20
    )
}

impl Usenet {
    pub fn new(db: Db) -> Self {
        Self::with_getter(db, Box::new(http_get))
    }

    pub fn with_getter(db: Db, get: Box<Getter>) -> Self {
        Usenet {
            db,
            get,
            lock: Mutex::new(()),
            pauses: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
        }
    }

    fn load(&self) -> Map<String, Value> {
        let mut data = self
            .db
            .get("sources")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        if !data.get("indexers").is_some_and(Value::is_array) {
            data.insert("indexers".into(), json!([]));
        }
        if !data.get("sab").is_some_and(Value::is_object) {
            data.insert("sab".into(), json!({}));
        }
        data
    }

    /// Config for the UI – keys replaced by a flag.
    pub fn public_config(&self) -> Value {
        let data = self.load();
        let hide = |v: &Value| {
            let mut o = v.as_object().cloned().unwrap_or_default();
            let set = truthy(o.remove(SECRET).as_ref(), false);
            o.insert("apikey_set".into(), json!(set));
            Value::Object(o)
        };
        let indexers: Vec<Value> = data["indexers"].as_array().into_iter().flatten().map(hide).collect();
        json!({"indexers": indexers, "sab": hide(&data["sab"]), "configured": self.configured()})
    }

    /// Empty or missing API keys keep the stored ones – but never for another server.
    pub fn update_config(&self, new: &Value) -> Value {
        {
            let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            let old = self.load();
            let old_idx: HashMap<String, Value> = old["indexers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| (text_of(&i["id"]), i.clone()))
                .collect();
            let mut indexers = Vec::new();
            for i in new["indexers"].as_array().into_iter().flatten() {
                if !i.is_object() || text_of(&i["url"]).trim().is_empty() {
                    continue;
                }
                let url = text_of(&i["url"]).trim().to_string();
                let mut prev = old_idx.get(&text_of(&i["id"])).cloned().unwrap_or(json!({}));
                if host(&text_of(&prev["url"])) != host(&url) {
                    prev = json!({}); // new address: the stored key must not travel to another server
                }
                let id = if truthy(i.get("id"), false) {
                    text_of(&i["id"])
                } else {
                    let mut b = [0u8; 4];
                    rand::rng().fill_bytes(&mut b);
                    hex::encode(b)
                };
                let name = text_of(&i["name"]).trim().to_string();
                let name = if name.is_empty() {
                    Some(hostname(&api_url(&url)))
                        .filter(|h| !h.is_empty())
                        .unwrap_or_else(|| "Indexer".into())
                } else {
                    name
                };
                let cats_raw = if truthy(i.get("categories"), false) {
                    text_of(&i["categories"])
                } else {
                    DEFAULT_CATEGORIES.into()
                };
                let cats: String = cats_raw.chars().filter(|c| c.is_ascii_digit() || *c == ',').collect();
                let key = text_of(&i[SECRET]).trim().to_string();
                // at most one preferred indexer: it is asked first, the others only when it has nothing fitting
                let preferred = truthy(i.get("preferred"), false)
                    && truthy(i.get("enabled"), true)
                    && !indexers.iter().any(|x: &Value| x["preferred"] == true);
                indexers.push(json!({"id": id, "name": name, "url": url,
                                     "categories": if cats.is_empty() { DEFAULT_CATEGORIES.to_string() } else { cats },
                                     "enabled": truthy(i.get("enabled"), true), "preferred": preferred,
                                     SECRET: if key.is_empty() { text_of(&prev[SECRET]) } else { key }}));
            }
            let sab_new = &new["sab"];
            let sab_url = text_of(&sab_new["url"]).trim().to_string();
            let old_key = if host(&text_of(&old["sab"]["url"])) == host(&sab_url) {
                text_of(&old["sab"][SECRET])
            } else {
                String::new()
            };
            let key = text_of(&sab_new[SECRET]).trim().to_string();
            let sab = json!({"url": sab_url, "category": text_of(&sab_new["category"]).trim(),
                             SECRET: if key.is_empty() { old_key } else { key }});
            let _ = self.db.put("sources", &json!({"indexers": indexers, "sab": sab}));
        }
        self.public_config()
    }

    pub fn configured(&self) -> Value {
        let data = self.load();
        let indexers = data["indexers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| truthy(i.get("enabled"), true) && truthy(i.get(SECRET), false))
            .count();
        json!({"indexers": indexers, "sab": truthy(data["sab"].get("url"), false) && truthy(data["sab"].get(SECRET), false)})
    }

    pub fn has_indexers(&self) -> bool {
        self.configured()["indexers"].as_u64().unwrap_or(0) > 0
    }

    pub fn has_sab(&self) -> bool {
        self.configured()["sab"].as_bool().unwrap_or(false)
    }

    /// The enabled indexers in the order they are asked: the preferred one alone first (if one is marked and
    /// there are others), then the rest together. Without a preferred one: all at once.
    fn tiers(&self) -> Vec<Vec<Newznab<'_>>> {
        let (mut first, mut rest) = (Vec::new(), Vec::new());
        for i in self.load()["indexers"].as_array().into_iter().flatten() {
            if !truthy(i.get("enabled"), true) {
                continue;
            }
            let idx = Newznab::new(
                &text_of(&i["name"]),
                &text_of(&i["url"]),
                &text_of(&i[SECRET]),
                &text_of(&i["categories"]),
                &*self.get,
            );
            if truthy(i.get("preferred"), false) && first.is_empty() {
                first.push(idx);
            } else {
                rest.push(idx);
            }
        }
        if first.is_empty() || rest.is_empty() {
            first.extend(rest);
            return if first.is_empty() { Vec::new() } else { vec![first] };
        }
        vec![first, rest]
    }

    fn sab(&self) -> Result<Sabnzbd<'_>> {
        let s = self.load()["sab"].clone();
        if !truthy(s.get("url"), false) || !truthy(s.get(SECRET), false) {
            return fail(t("SABnzbd ist noch nicht eingerichtet", &[]));
        }
        Ok(Sabnzbd::new(
            &text_of(&s["url"]),
            &text_of(&s[SECRET]),
            &text_of(&s["category"]),
            &*self.get,
        ))
    }

    pub fn test(&self, kind: &str, index_id: &str) -> Result<String> {
        if kind == "sab" {
            return self.sab()?.test();
        }
        for i in self.load()["indexers"].as_array().into_iter().flatten() {
            if text_of(&i["id"]) == index_id {
                return Newznab::new(
                    &text_of(&i["name"]),
                    &text_of(&i["url"]),
                    &text_of(&i[SECRET]),
                    &text_of(&i["categories"]),
                    &*self.get,
                )
                .test();
            }
        }
        fail(t("Indexer nicht gefunden", &[]))
    }

    /// Until when an indexer gets no searches (it answered 429) – 0 when it may be asked.
    pub fn paused(&self, name: &str) -> f64 {
        let until = self
            .pauses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .copied()
            .unwrap_or(0.0);
        if until > now() { until } else { 0.0 }
    }

    /// Search for the query (plus `extra`), in all umlaut spellings (`variants` = false: exactly these). With a
    /// preferred indexer only that one is asked – the others follow when it found nothing, or at once with
    /// `everywhere`. The answer names who was asked (`asked`) and who was not needed (`skipped`).
    pub fn search(
        &self,
        query: &str,
        language: &str,
        extra: &[String],
        variants: bool,
        limit: usize,
        everywhere: bool,
    ) -> Result<Value> {
        let mut tiers = self.tiers();
        if tiers.is_empty() {
            return fail(t("Noch kein Indexer eingerichtet", &[]));
        }
        if everywhere {
            tiers = vec![tiers.into_iter().flatten().collect()];
        }
        let names = |t: &[Newznab]| t.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        let mut asked: Vec<String> = Vec::new();
        let mut errors: Vec<Value> = Vec::new();
        let mut out = json!({});
        let mut skipped: Vec<String> = Vec::new();
        let mut it = tiers.into_iter();
        while let Some(tier) = it.next() {
            asked.extend(names(&tier));
            out = self.search_in(&tier, query, language, extra, variants, limit)?;
            errors.extend(out["errors"].as_array().cloned().unwrap_or_default());
            if out["results"].as_array().is_some_and(|r| !r.is_empty()) {
                skipped = it.flat_map(|t| names(&t)).collect();
                break;
            }
        }
        out["errors"] = json!(errors);
        out["asked"] = json!(asked);
        out["skipped"] = json!(skipped);
        Ok(out)
    }

    fn search_in(
        &self,
        indexers: &[Newznab<'_>],
        query: &str,
        language: &str,
        extra: &[String],
        variants: bool,
        limit: usize,
    ) -> Result<Value> {
        let mut queries: Vec<String> = Vec::new();
        for q in std::iter::once(query.to_string()).chain(extra.iter().filter(|e| !e.trim().is_empty()).cloned()) {
            let vs = if variants {
                query_variants(&q)
            } else {
                vec![q.split_whitespace().collect::<Vec<_>>().join(" ")]
            };
            for v in vs {
                if !queries.contains(&v) {
                    queries.push(v);
                }
            }
        }
        queries.truncate(limit.max(1));
        let errors: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
        // one query after the other per indexer – a burst of parallel queries is what makes indexers answer 429
        let found: Vec<Vec<Value>> = std::thread::scope(|scope| {
            let handles: Vec<_> = indexers
                .iter()
                .enumerate()
                .map(|(n, idx)| {
                    let (queries, errors) = (&queries, &errors);
                    scope.spawn(move || {
                        let mut found = Vec::new();
                        for q in queries {
                            if self.paused(&idx.name) > 0.0 {
                                break;
                            }
                            match idx.search(q, 100) {
                                Ok(r) => found.extend(r),
                                Err(UsenetError::RateLimited(_, wait)) => {
                                    self.pauses
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .insert(idx.name.clone(), now() + wait);
                                    break;
                                }
                                Err(UsenetError::Failed(e)) => {
                                    let msg = if e.starts_with(&idx.name) {
                                        e
                                    } else {
                                        format!("{}: {e}", idx.name)
                                    };
                                    let mut errs = errors.lock().unwrap_or_else(|p| p.into_inner());
                                    errs.retain(|(i, _)| *i != n);
                                    errs.push((n, msg));
                                }
                            }
                        }
                        let paused = self.paused(&idx.name);
                        if paused > 0.0 {
                            let mut errs = errors.lock().unwrap_or_else(|p| p.into_inner());
                            errs.retain(|(i, _)| *i != n);
                            errs.push((
                                n,
                                t(
                                    "{name}: zu viele Anfragen – Pause bis {time}",
                                    &[("name", &idx.name), ("time", &crate::text::local_hm(paused))],
                                ),
                            ));
                        }
                        found
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        let mut errs = errors.into_inner().unwrap_or_else(|p| p.into_inner());
        errs.sort_by_key(|(i, _)| *i);
        // one entry per release (several indexers often carry the same one)
        let mut order: Vec<String> = Vec::new();
        let mut seen: HashMap<String, Value> = HashMap::new();
        for r in found.into_iter().flatten() {
            let key = dedupe_key(&r);
            if let Some(first) = seen.get_mut(&key) {
                let ix = r["indexer"].clone();
                let also = first["also"].as_array().cloned().unwrap_or_default();
                if ix != first["indexer"] && !also.contains(&ix) {
                    let mut also = also;
                    also.push(ix);
                    first["also"] = json!(also);
                }
                continue;
            }
            order.push(key.clone());
            seen.insert(key, r);
        }
        let wanted = std::iter::once(query.to_string())
            .chain(extra.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let mut ranked: Vec<Value> = order.into_iter().filter_map(|k| seen.remove(&k)).collect();
        // stable sort by the rank tuple, best first (like Python's sorted(reverse=True), which keeps ties in order)
        let keyed: Vec<(RankKey, Value)> = ranked.drain(..).map(|r| (rank_key(&r, &wanted, language), r)).collect();
        let mut keyed = keyed;
        keyed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::new();
        {
            let mut cache = self.results.lock().unwrap_or_else(|p| p.into_inner());
            if cache.len() > 2000 {
                cache.clear();
            }
            for (_, r) in keyed {
                let mut b = [0u8; 6];
                rand::rng().fill_bytes(&mut b);
                let rid = hex::encode(b);
                let mut stored = r.clone();
                stored["query"] = json!(query);
                cache.insert(rid.clone(), stored);
                let mut shown = r;
                if let Some(o) = shown.as_object_mut() {
                    o.remove("link");
                }
                shown["id"] = json!(rid);
                out.push(shown);
            }
        }
        Ok(json!({"results": out, "errors": errs.into_iter().map(|e| e.1).collect::<Vec<_>>(), "queries": queries}))
    }

    /// Search round by round until `fits` likes the results so far; never more than `budget` queries per indexer.
    /// With a preferred indexer: all rounds there first – the other indexers only when nothing fitting came up.
    pub fn search_stages(
        &self,
        stages: &[Vec<String>],
        fits: &dyn Fn(&[Value]) -> bool,
        language: &str,
        budget: usize,
        everywhere: bool,
    ) -> Result<Value> {
        let mut tiers = self.tiers();
        if tiers.is_empty() {
            return fail(t("Noch kein Indexer eingerichtet", &[]));
        }
        if everywhere {
            tiers = vec![tiers.into_iter().flatten().collect()];
        }
        let names = |t: &[Newznab]| t.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        let mut results: Vec<(String, Value)> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut queries: Vec<String> = Vec::new();
        let (mut who, mut skipped): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        let mut rounds = 0;
        let mut it = tiers.into_iter();
        while let Some(tier) = it.next() {
            who.extend(names(&tier));
            let res = self.stages_in(&tier, stages, fits, language, budget, &mut results)?;
            rounds += res.0;
            for e in res.1 {
                if !errors.contains(&e) {
                    errors.push(e);
                }
            }
            for q in res.2 {
                if !queries.contains(&q) {
                    queries.push(q);
                }
            }
            let found: Vec<Value> = results.iter().map(|x| x.1.clone()).collect();
            if !found.is_empty() && fits(&found) {
                skipped = it.flat_map(|t| names(&t)).collect();
                break;
            }
        }
        Ok(
            json!({"results": results.into_iter().map(|x| x.1).collect::<Vec<_>>(), "errors": errors,
                  "queries": queries, "rounds": rounds, "asked": who, "skipped": skipped}),
        )
    }

    /// The rounds on one group of indexers: (rounds, errors, queries asked); results are added to `results`.
    fn stages_in(
        &self,
        indexers: &[Newznab<'_>],
        stages: &[Vec<String>],
        fits: &dyn Fn(&[Value]) -> bool,
        language: &str,
        budget: usize,
        results: &mut Vec<(String, Value)>,
    ) -> Result<(usize, Vec<String>, Vec<String>)> {
        let mut errors: Vec<String> = Vec::new();
        let mut asked: Vec<String> = Vec::new();
        let mut rounds = 0;
        for stage in stages {
            let mut todo: Vec<String> = Vec::new();
            for v in stage.iter().flat_map(|q| query_variants(q)) {
                if !todo.contains(&v) && !asked.contains(&v) {
                    todo.push(v);
                }
            }
            todo.truncate(budget.saturating_sub(asked.len()));
            if todo.is_empty() {
                continue;
            }
            let res = self.search_in(indexers, &todo[0], language, &todo[1..], false, todo.len())?;
            rounds += 1;
            asked.extend(todo);
            for e in res["errors"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !errors.iter().any(|x| x == e) {
                    errors.push(e.to_string());
                }
            }
            let got = res["results"].as_array().cloned().unwrap_or_default();
            for r in &got {
                let k = dedupe_key(r);
                if !results.iter().any(|(x, _)| *x == k) {
                    results.push((k, r.clone()));
                }
            }
            let found: Vec<Value> = results.iter().map(|x| x.1.clone()).collect();
            let all_failed = got.is_empty() && res["errors"].as_array().map_or(0, Vec::len) >= indexers.len();
            if (!found.is_empty() && fits(&found)) || all_failed || asked.len() >= budget {
                break;
            }
        }
        Ok((rounds, errors, asked))
    }

    pub fn send(&self, result_id: &str, hints: &Value) -> Result<Value> {
        let r = self
            .results
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(result_id)
            .cloned();
        let Some(r) = r else {
            return fail(t("Suchergebnis abgelaufen – bitte neu suchen", &[]));
        };
        let name = safe_release_name(r["title"].as_str().unwrap_or(""));
        let nzo = self.sab()?.add(r["link"].as_str().unwrap_or(""), &name)?;
        let mut info = json!({"query": r["query"]});
        if let (Some(o), Some(h)) = (info.as_object_mut(), hints.as_object()) {
            o.extend(h.clone());
        }
        info["nzo_id"] = json!(nzo);
        info["time"] = json!(now());
        self.remember(&name, &info);
        Ok(json!({"nzo_id": nzo, "title": name}))
    }

    fn remember(&self, name: &str, info: &Value) {
        let mut c = self.db.lock();
        if let Ok(tx) = c.transaction() {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO grabs(name, key, time, data) VALUES(?,?,?,?)",
                rusqlite::params![
                    name,
                    grab_key(name),
                    info["time"].as_f64().unwrap_or_else(now),
                    compact(info)
                ],
            );
            let _ = tx.execute(
                "DELETE FROM grabs WHERE name NOT IN (SELECT name FROM grabs ORDER BY time DESC LIMIT 500)",
                [],
            );
            let _ = tx.commit();
        }
    }

    /// Search info for a download folder (SABnzbd names it after the release).
    pub fn hint_for(&self, folder_name: &str) -> Option<Value> {
        let data: Option<String> = self
            .db
            .lock()
            .query_row(
                "SELECT data FROM grabs WHERE key=? ORDER BY time DESC LIMIT 1",
                [grab_key(folder_name)],
                |r| r.get(0),
            )
            .ok();
        data.and_then(|d| serde_json::from_str(&d).ok())
    }

    pub fn activity(&self, limit: usize) -> Result<Value> {
        self.sab()?.activity(limit)
    }

    /// Remove finished downloads from SABnzbd's history (all of our category, or just `ids`), except `keep`.
    pub fn clear_finished(&self, ids: Option<Vec<String>>, keep: &[String]) -> Result<Value> {
        let sab = self.sab()?;
        let wanted = match ids {
            Some(i) if !i.is_empty() => i,
            _ => sab.history_ids(500)?,
        };
        let kept = wanted.iter().filter(|i| keep.contains(i)).count();
        let todo: Vec<String> = wanted.into_iter().filter(|i| !keep.contains(i)).collect();
        let removed = sab.remove_history(&todo)?;
        Ok(json!({"removed": removed, "kept": kept}))
    }
}

impl Usenet {
    /// Results in search order for `query` (best first; equal ones keep their order).
    pub fn rank_results(&self, list: &mut [Value], query: &str, language: &str) -> Vec<Value> {
        let mut keyed: Vec<(RankKey, Value)> = list.iter().map(|r| (rank_key(r, query, language), r.clone())).collect();
        keyed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        keyed.into_iter().map(|x| x.1).collect()
    }
}

pub fn grab_key(name: &str) -> String {
    sub(rx!(r"[\W_]+"), &name.to_lowercase(), "")
}

type RankKey = (f64, i64, i64, i64, i64, f64);

/// Order of search results: share of the query's words, language, unabridged, format, grabs, date.
fn rank_key(r: &Value, query: &str, language: &str) -> RankKey {
    let mut words: Vec<String> = Vec::new();
    for w in match_words(query) {
        if w.chars().count() > 2 && !words.contains(&w) {
            words.push(w);
        }
    }
    let title = match_words(r["title"].as_str().unwrap_or("")).join(" ");
    let hits = words.iter().filter(|w| title.contains(w.as_str())).count() as f64 / words.len().max(1) as f64;
    let rl = r["language"].as_str().unwrap_or("");
    let lang = if !language.is_empty() && rl == language {
        1
    } else if rl.is_empty() {
        0
    } else if !language.is_empty() {
        -1
    } else {
        0
    };
    let fmt = match r["format"].as_str().unwrap_or("") {
        "M4B" => 2,
        "M4A" | "MP3" | "FLAC" => 1,
        _ => 0,
    };
    let abridged = i64::from(r["abridged"].as_bool().unwrap_or(false));
    (
        (hits * 10.0).round_ties_even() / 10.0,
        lang,
        -abridged,
        fmt,
        r["grabs"].as_i64().unwrap_or(0),
        r["published"].as_f64().unwrap_or(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Two indexers "a.example" and "b.example"; `has` says which of them knows the release.
    fn usenet(has: &'static [&'static str], asked: Arc<Mutex<Vec<String>>>) -> Usenet {
        let u = Usenet::with_getter(
            Db::memory(),
            Box::new(move |url: &str| {
                let host = if url.contains("a.example") { "a" } else { "b" };
                if url.contains("t=caps") {
                    return Ok(br#"<caps><categories><category id="3000" name="Audio"><subcat id="3030" name="Audiobook"/></category></categories></caps>"#.to_vec());
                }
                asked.lock().unwrap().push(host.to_string());
                let item = if has.contains(&host) {
                    format!(
                        r#"<item><title>Theodor Storm - Der Schimmelreiter ({host})</title><link>https://{host}.example/nzb/1</link><newznab:attr name="size" value="300000000"/><newznab:attr name="category" value="3030"/></item>"#
                    )
                } else {
                    String::new()
                };
                Ok(format!(
                    r#"<rss xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/"><channel>{item}</channel></rss>"#
                )
                .into_bytes())
            }),
        );
        u.update_config(&json!({"indexers": [
            {"name": "A", "url": "https://a.example", "apikey": "k1", "preferred": true},
            {"name": "B", "url": "https://b.example", "apikey": "k2", "preferred": true}], "sab": {}}));
        u
    }

    #[test]
    fn preferred_indexer_first_the_others_only_when_needed() {
        crate::text::set_language("de");
        let asked = Arc::new(Mutex::new(Vec::new()));
        let u = usenet(&["a", "b"], asked.clone());
        // only one indexer can be the preferred one
        let cfg = u.public_config();
        assert_eq!(
            (
                cfg["indexers"][0]["preferred"].clone(),
                cfg["indexers"][1]["preferred"].clone()
            ),
            (json!(true), json!(false))
        );
        // A has it: B is never asked
        let res = u.search("Schimmelreiter", "", &[], false, 1, false).unwrap();
        assert_eq!(
            (res["asked"].clone(), res["skipped"].clone()),
            (json!(["A"]), json!(["B"]))
        );
        assert_eq!(res["results"].as_array().unwrap().len(), 1);
        assert!(!asked.lock().unwrap().contains(&"b".to_string()));
        // "also ask the others"
        let res = u.search("Schimmelreiter", "", &[], false, 1, true).unwrap();
        assert_eq!(
            (res["asked"].clone(), res["skipped"].clone()),
            (json!(["A", "B"]), json!([]))
        );
        assert_eq!(res["results"].as_array().unwrap().len(), 2);

        // A has nothing: B is asked as the fallback
        let asked = Arc::new(Mutex::new(Vec::new()));
        let u = usenet(&["b"], asked.clone());
        let res = u.search("Schimmelreiter", "", &[], false, 1, false).unwrap();
        assert_eq!(
            (res["asked"].clone(), res["skipped"].clone()),
            (json!(["A", "B"]), json!([]))
        );
        assert_eq!(res["results"][0]["indexer"], json!("B"));
        // round by round: A's result does not fit -> B is asked too; it fits -> done
        let u = usenet(&["a", "b"], Arc::new(Mutex::new(Vec::new())));
        let stages = vec![vec!["Schimmelreiter".to_string()]];
        let only_b = |found: &[Value]| found.iter().any(|r| r["indexer"] == "B");
        let res = u.search_stages(&stages, &only_b, "", 4, false).unwrap();
        assert_eq!(
            (res["asked"].clone(), res["results"].as_array().unwrap().len()),
            (json!(["A", "B"]), 2)
        );
        let any = |found: &[Value]| !found.is_empty();
        let res = u.search_stages(&stages, &any, "", 4, false).unwrap();
        assert_eq!(
            (res["asked"].clone(), res["skipped"].clone()),
            (json!(["A"]), json!(["B"]))
        );
        // without a preferred one: all at once, as before
        u.update_config(&json!({"indexers": [
            {"name": "A", "url": "https://a.example", "apikey": "k1"},
            {"name": "B", "url": "https://b.example", "apikey": "k2"}], "sab": {}}));
        let res = u.search("Schimmelreiter", "", &[], false, 1, false).unwrap();
        assert_eq!(
            (res["asked"].clone(), res["skipped"].clone()),
            (json!(["A", "B"]), json!([]))
        );
    }
}
