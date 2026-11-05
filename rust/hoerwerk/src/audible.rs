//! Audible metadata (search, details, chapters) and the other shops as fallbacks (audible.py, providers.py).
//! Search and details come from the Audible catalog API of the chosen marketplace; chapters from Audnexus.

use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::difflib;
use crate::http;
use crate::rx;
use crate::rx::{is_match, search, sub};
use crate::text::{self, now, t};

pub const REGIONS: [(&str, &str); 10] = [
    ("de", "de"),
    ("us", "com"),
    ("uk", "co.uk"),
    ("au", "com.au"),
    ("ca", "ca"),
    ("fr", "fr"),
    ("it", "it"),
    ("es", "es"),
    ("in", "in"),
    ("jp", "co.jp"),
];
const RESPONSE_GROUPS: &str =
    "contributors,product_attrs,product_desc,product_extended_attrs,series,media,category_ladders,rating";
const AUDNEXUS: &str = "https://api.audnex.us";
pub const SERIES_MAX: usize = 60;
const USER_AGENT: &str = "Hoerwerk/7 (+https://github.com/Larosen/hoerwerk)";
const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
const MAX_IMAGE: u64 = 15 * 1024 * 1024;
pub const AUTO_SOURCES: [&str; 2] = ["audible", "audnexus"];

#[derive(Debug, Clone)]
pub struct AudibleError(pub String);

impl std::fmt::Display for AudibleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

type Result<T> = std::result::Result<T, AudibleError>;

fn err<T>(m: String) -> Result<T> {
    Err(AudibleError(m))
}

fn domain(region: &str) -> &'static str {
    REGIONS.iter().find(|r| r.0 == region).map_or("com", |r| r.1)
}

pub fn api_host(region: &str) -> String {
    format!("https://api.audible.{}", domain(region))
}

pub fn store_url(asin: &str, region: &str) -> String {
    format!("https://www.audible.{}/pd/{asin}", domain(region))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// urlencode like Python (space -> +)
pub fn form(params: &[(&str, String)]) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b' ');
    params
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}={}",
                percent_encoding::utf8_percent_encode(v, SET)
                    .to_string()
                    .replace(' ', "+")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

// -- HTTP -------------------------------------------------------------------------------------------

fn global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (18..20).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return global(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            (seg[0] & 0xe000) == 0x2000
                && !(seg[0] == 0x2001 && seg[1] == 0x0db8)
                && !(seg[0] == 0x2001 && seg[1] < 0x0200)
        }
    }
}

fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let netloc = rest.split(['/', '?', '#']).next()?;
    if netloc.contains('@') {
        return None;
    }
    let default = if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 };
    match netloc.strip_prefix('[') {
        Some(v6) => {
            let (h, p) = v6.split_once(']')?;
            Some((
                h.to_string(),
                p.strip_prefix(':').and_then(|x| x.parse().ok()).unwrap_or(default),
            ))
        }
        None => match netloc.rsplit_once(':') {
            Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
            None => Some((netloc.to_string(), default)),
        },
    }
}

fn http_scheme(url: &str) -> bool {
    let l = url.to_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// http(s) URL whose host resolves only to public addresses (no LAN, localhost, metadata IPs).
pub fn is_public_url(url: &str) -> bool {
    if !http_scheme(url) {
        return false;
    }
    let Some((host, port)) = host_port(url).filter(|(h, _)| !h.is_empty()) else {
        return false;
    };
    match (host.as_str(), port).to_socket_addrs() {
        Ok(addrs) => {
            let all: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
            !all.is_empty() && all.into_iter().all(global)
        }
        Err(_) => false,
    }
}

fn hostname(url: &str) -> String {
    host_port(url).map(|x| x.0).unwrap_or_else(|| url.to_string())
}

/// GET with retries (Retry-After on 429), gives up at once on 400/403/404; `allow` checks the URL and every
/// redirect target; answers capped at `max_bytes`.
pub fn http_get(
    url: &str,
    timeout: u64,
    retries: u32,
    accept_json: bool,
    max_bytes: u64,
    allow: Option<&dyn Fn(&str) -> bool>,
) -> Result<Vec<u8>> {
    if !http_scheme(url) {
        return err(t("Nur http(s)-Adressen erlaubt", &[]));
    }
    let ok = |u: &str| allow.map_or(http_scheme(u), |f| f(u));
    if allow.is_some() && !ok(url) {
        return err(t("Adresse nicht erlaubt: {host}", &[("host", &hostname(url))]));
    }
    let mut delay = 1.5f64;
    let mut last = String::new();
    for attempt in 0..retries {
        let mut target = url.to_string();
        let mut answer = None;
        for _ in 0..6 {
            let r = http::get(&target)
                .header("User-Agent", USER_AGENT)
                .header("Accept", if accept_json { "application/json" } else { "*/*" })
                .timeout(timeout)
                .no_redirect()
                .limit(max_bytes + 1)
                .send();
            match r {
                Ok(resp) if (300..400).contains(&resp.status) && resp.location.is_some() => {
                    let loc = resp.location.clone().unwrap_or_default();
                    let next = if http_scheme(&loc) {
                        loc
                    } else {
                        join_url(&target, &loc)
                    };
                    if !ok(&next) {
                        return err(t("Weiterleitung zu {host} abgelehnt", &[("host", &hostname(&next))]));
                    }
                    target = next;
                }
                other => {
                    answer = Some(other);
                    break;
                }
            }
        }
        match answer {
            Some(Ok(resp)) if resp.ok() => {
                if resp.body.len() as u64 > max_bytes {
                    return err(t(
                        "Antwort zu groß (> {mb} MB)",
                        &[("mb", &(max_bytes >> 20).to_string())],
                    ));
                }
                return Ok(resp.body);
            }
            Some(Ok(resp)) => {
                if matches!(resp.status, 400 | 403 | 404) {
                    return err(t(
                        "HTTP {code} für {url}",
                        &[
                            ("code", &resp.status.to_string()),
                            ("url", url.split('?').next().unwrap_or(url)),
                        ],
                    ));
                }
                if let Some(ra) = resp
                    .retry_after
                    .as_deref()
                    .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                {
                    delay = (ra.parse::<f64>().unwrap_or(0.0) + 1.0).min(30.0);
                }
                last = format!("HTTP Error {}", resp.status);
            }
            Some(Err(e)) => last = e,
            None => last = "too many redirects".into(),
        }
        if attempt + 1 < retries {
            std::thread::sleep(Duration::from_secs_f64(delay));
            delay *= 2.0;
        }
    }
    err(t("Audible nicht erreichbar: {error}", &[("error", &last)]))
}

fn join_url(base: &str, loc: &str) -> String {
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    let host = rest.split('/').next().unwrap_or("");
    if loc.starts_with("//") {
        format!("{scheme}:{loc}")
    } else if loc.starts_with('/') {
        format!("{scheme}://{host}{loc}")
    } else {
        let dir = base.rsplit_once('/').map_or(base, |x| x.0);
        format!("{dir}/{loc}")
    }
}

/// Cover download: public hosts only, size-capped, and it must really be a JPEG or PNG.
pub fn fetch_image(url: &str) -> Result<Option<Vec<u8>>> {
    if url.is_empty() || !is_public_url(url) {
        return Ok(None);
    }
    let data = http_get(url, 30, 2, false, MAX_IMAGE, Some(&is_public_url))?;
    Ok((data.starts_with(b"\xff\xd8\xff") || data.starts_with(b"\x89PNG\r\n\x1a\n")).then_some(data))
}

pub fn get_json(url: &str) -> Result<Value> {
    let data = http_get(url, 20, 3, true, MAX_RESPONSE, None)?;
    serde_json::from_slice(&data).map_err(|_| {
        AudibleError(t(
            "Ungültige Antwort von {url}",
            &[("url", url.split('?').next().unwrap_or(url))],
        ))
    })
}

// -- normalisation ------------------------------------------------------------------------------------

fn entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return Some(
            char::from_u32(code)
                .filter(|c| *c != '\0')
                .unwrap_or('\u{fffd}')
                .to_string(),
        );
    }
    Some(
        match name {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => "\u{a0}",
            "auml" => "ä",
            "ouml" => "ö",
            "uuml" => "ü",
            "Auml" => "Ä",
            "Ouml" => "Ö",
            "Uuml" => "Ü",
            "szlig" => "ß",
            "eacute" => "é",
            "egrave" => "è",
            "aacute" => "á",
            "agrave" => "à",
            "ndash" => "–",
            "mdash" => "—",
            "hellip" => "…",
            "laquo" => "«",
            "raquo" => "»",
            "bdquo" => "„",
            "ldquo" => "“",
            "rdquo" => "”",
            "lsquo" => "‘",
            "rsquo" => "’",
            "sbquo" => "‚",
            "copy" => "©",
            "reg" => "®",
            "trade" => "™",
            "deg" => "°",
            "euro" => "€",
            "middot" => "·",
            "shy" => "\u{ad}",
            _ => return None,
        }
        .to_string(),
    )
}

/// html.unescape for the entities shop texts use.
pub fn unescape(text: &str) -> String {
    rx!(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);?")
        .replace_all(text, |c: &fancy_regex::Captures| {
            entity(&c[1]).unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

/// Shop HTML (descriptions) -> plain text with paragraphs.
pub fn html_to_text(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }
    let mut parts = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        parts.push_str(&unescape(&rest[..i]));
        let after = &rest[i + 1..];
        let Some(end) = after.find('>') else {
            parts.push_str(&unescape(&rest[i..]));
            rest = "";
            break;
        };
        let tag = &after[..end];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        match (name.as_str(), closing) {
            ("p" | "div", _) => parts.push_str("\n\n"),
            ("br" | "li", false) => parts.push('\n'),
            _ => {}
        }
        rest = &after[end + 1..];
    }
    parts.push_str(&unescape(rest));
    let text = unescape(&parts);
    let mut out: Vec<String> = Vec::new();
    for ln in text.split(['\n', '\r']) {
        let ln = sub(rx!(r"[ \t]+"), ln, " ").trim().to_string();
        if !ln.is_empty() || out.last().is_some_and(|l| !l.is_empty()) {
            out.push(ln);
        }
    }
    out.join("\n").trim().to_string()
}

fn split_role(name: &str) -> (String, String) {
    for sep in [" — ", " - ", " – "] {
        if let Some((n, role)) = name.split_once(sep) {
            return (n.trim().to_string(), role.trim().to_string());
        }
    }
    (name.trim().to_string(), String::new())
}

pub fn series_position(seq: &Value) -> Value {
    let text = match seq {
        Value::Null => return Value::Null,
        Value::String(x) if x.is_empty() => return Value::Null,
        Value::String(x) => x.clone(),
        other => other.to_string(),
    };
    search(rx!(r"\d+(?:[.,]\d+)?(?:\s*-\s*\d+(?:[.,]\d+)?)?"), &text)
        .map(|m| json!(m[0].replace(',', ".").replace(' ', "")))
        .unwrap_or(Value::Null)
}

/// Python's str.capitalize()
pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn opt_str(v: &str) -> Value {
    if v.is_empty() { Value::Null } else { json!(v) }
}

/// Audible catalog product -> flat book used everywhere else.
pub fn normalize_product(p: &Value, region: &str) -> Value {
    let (mut authors, mut translators) = (Vec::<String>::new(), Vec::<String>::new());
    for a in p["authors"].as_array().into_iter().flatten() {
        let (name, role) = split_role(s(a, "name"));
        if name.is_empty() {
            continue;
        }
        if !role.is_empty()
            && is_match(
                rx!(
                    r"(?i)(translator|übersetz|editor|herausgeb|illustrat|foreword|vorwort|introduction|einleitung|adapt|bearbeit|contributor|afterword|nachwort)"
                ),
                &role,
            )
        {
            if is_match(rx!(r"(?i)translator|übersetz"), &role) {
                translators.push(name);
            }
            continue;
        }
        if !authors.contains(&name) {
            authors.push(name);
        }
    }
    let mut narrators: Vec<String> = Vec::new();
    for n in p["narrators"].as_array().into_iter().flatten() {
        let name = split_role(s(n, "name")).0;
        if !name.is_empty() && !narrators.contains(&name) {
            narrators.push(name);
        }
    }
    let series_list = p["series"].as_array().cloned().unwrap_or_default();
    let series = series_list.first().cloned().unwrap_or(Value::Null);
    let (mut genres, mut tags) = (Vec::<String>::new(), Vec::<String>::new());
    for ladder in p["category_ladders"].as_array().into_iter().flatten() {
        let steps: Vec<String> = ladder["ladder"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string))
            .collect();
        if let Some(first) = steps.first()
            && !genres.contains(first)
        {
            genres.push(first.clone());
        }
        for x in steps.iter().skip(1) {
            if !tags.contains(x) {
                tags.push(x.clone());
            }
        }
    }
    let images = p["product_images"].as_object().cloned().unwrap_or_default();
    let cover = images
        .get("1024")
        .or_else(|| images.get("500"))
        .or_else(|| images.values().next())
        .and_then(Value::as_str)
        .map(|c| {
            // Amazon image URLs: dropping the size suffix gives the original resolution
            sub(rx!(r"\._[A-Z0-9_,]+_(?=\.(jpg|png))"), c, "")
        });
    let summary = [s(p, "publisher_summary"), s(p, "merchandising_summary")]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or("")
        .to_string();
    let rating = &p["rating"]["overall_distribution"];
    let num = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|x| x.parse().ok()))
            .unwrap_or(0.0)
    };
    let release: String = [s(p, "release_date"), s(p, "issue_date")]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or("")
        .chars()
        .take(10)
        .collect();
    let asin = s(p, "asin");
    json!({
        "id": p["asin"], "asin": p["asin"], "title": s(p, "title").trim(),
        "subtitle": opt_str(s(p, "subtitle").trim()), "authors": authors, "translators": translators, "narrators": narrators,
        "series": series["title"], "series_asin": series["asin"], "series_position": series_position(&series["sequence"]),
        "all_series": series_list.iter().map(|x| json!({"name": x["title"], "position": series_position(&x["sequence"])})).collect::<Vec<_>>(),
        "publisher": p["publisher_name"], "release_date": opt_str(&release), "language": capitalize(s(p, "language")),
        "runtime_min": num(&p["runtime_length_min"]) as i64, "format_type": p["format_type"], "genres": genres, "tags": tags,
        "description": html_to_text(&summary), "summary_html": summary, "cover_url": cover, "region": region,
        "url": store_url(asin, region), "is_adult": p["is_adult_product"].as_bool().unwrap_or(false),
        "rating": num(&rating["display_average_rating"]), "ratings": num(&rating["num_ratings"]) as i64, "source": "audible",
    })
}

// -- client -----------------------------------------------------------------------------------------------

pub type JsonGetter = dyn Fn(&str) -> Result<Value> + Send + Sync;

pub struct Client {
    get: Box<JsonGetter>,
    cache: Mutex<HashMap<String, (f64, Value)>>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(Box::new(get_json))
    }
}

impl Client {
    pub fn new(get: Box<JsonGetter>) -> Self {
        Client {
            get,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn cached(&self, url: &str) -> Result<Value> {
        if let Some((at, v)) = self.cache.lock().unwrap_or_else(|p| p.into_inner()).get(url)
            && now() - at < 3600.0
        {
            return Ok(v.clone());
        }
        let data = (self.get)(url)?;
        let mut c = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if c.len() > 500 {
            c.clear();
        }
        c.insert(url.to_string(), (now(), data.clone()));
        Ok(data)
    }

    /// Released audiobooks only (pre-orders too with `upcoming`) – no podcasts or magazines.
    pub fn products(products: &Value, region: &str, upcoming: bool) -> Vec<Value> {
        let tm = text::local(now());
        let today = format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday);
        products
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| !s(p, "asin").is_empty() && !s(p, "title").is_empty())
            .filter(|p| {
                upcoming || {
                    let r = s(p, "release_date");
                    (if r.is_empty() { "0" } else { r }) <= today.as_str()
                }
            })
            .filter(|p| !matches!(s(p, "content_delivery_type"), "PodcastParent" | "Periodical"))
            .map(|p| normalize_product(p, region))
            .collect()
    }

    pub fn search(
        &self,
        region: &str,
        keywords: &str,
        title: &str,
        author: &str,
        limit: usize,
        upcoming: bool,
    ) -> Result<Vec<Value>> {
        let mut params = vec![
            ("response_groups", RESPONSE_GROUPS.to_string()),
            ("image_sizes", "500,1024".into()),
            ("num_results", limit.to_string()),
            ("products_sort_by", "Relevance".into()),
        ];
        for (k, v) in [("keywords", keywords), ("title", title), ("author", author)] {
            if !v.is_empty() {
                params.push((k, v.to_string()));
            }
        }
        let data = self.cached(&format!("{}/1.0/catalog/products?{}", api_host(region), form(&params)))?;
        Ok(Self::products(&data["products"], region, upcoming))
    }

    pub fn product(&self, asin: &str, region: &str) -> Result<Value> {
        let params = [
            ("response_groups", RESPONSE_GROUPS.to_string()),
            ("image_sizes", "500,1024".into()),
        ];
        let data = self.cached(&format!(
            "{}/1.0/catalog/products/{asin}?{}",
            api_host(region),
            form(&params)
        ))?;
        let p = &data["product"];
        if s(p, "title").is_empty() {
            return err(t(
                "ASIN {asin} im Audible-Shop ({region}) nicht gefunden",
                &[("asin", asin), ("region", region)],
            ));
        }
        Ok(normalize_product(p, region))
    }

    /// The released volumes of a series in order: {asin, title, books}. Box sets and other editions of the same
    /// volume are left out – one book per position, unabridged and in the wanted language first.
    pub fn series(&self, series_asin: &str, region: &str, language: &str, limit: usize) -> Result<Value> {
        let params = [
            ("response_groups", "relationships,product_attrs".to_string()),
            ("image_sizes", "500".into()),
        ];
        let data = self.cached(&format!(
            "{}/1.0/catalog/products/{series_asin}?{}",
            api_host(region),
            form(&params)
        ))?;
        let p = data["product"].clone();
        let mut rel: Vec<Value> = p["relationships"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| {
                !s(r, "asin").is_empty()
                    && r["relationship_type"] == "series"
                    && r["relationship_to_product"] == "child"
            })
            .cloned()
            .collect();
        let sortkey = |r: &Value| {
            r["sort"]
                .as_f64()
                .or_else(|| r["sort"].as_str().and_then(|x| x.parse().ok()))
                .unwrap_or(0.0)
        };
        rel.sort_by(|a, b| sortkey(a).partial_cmp(&sortkey(b)).unwrap_or(std::cmp::Ordering::Equal));
        let mut asins: Vec<String> = Vec::new();
        for r in &rel {
            let a = s(r, "asin").to_string();
            if !asins.contains(&a) {
                asins.push(a);
            }
        }
        asins.truncate(limit * 2);
        let mut products: Vec<Value> = Vec::new();
        for chunk in asins.chunks(50) {
            let q = [
                ("response_groups", RESPONSE_GROUPS.to_string()),
                ("image_sizes", "500,1024".into()),
                ("asins", chunk.join(",")),
            ];
            let got = self.cached(&format!("{}/1.0/catalog/products?{}", api_host(region), form(&q)))?;
            products.extend(got["products"].as_array().cloned().unwrap_or_default());
        }
        let by_asin: HashMap<String, Value> = products.into_iter().map(|x| (s(&x, "asin").to_string(), x)).collect();
        let mut best: Vec<(String, (bool, bool), Value)> = Vec::new();
        for r in &rel {
            let Some(raw) = by_asin.get(s(r, "asin")) else { continue };
            let Some(mut book) = Self::products(&json!([raw]), region, false).into_iter().next() else {
                continue;
            };
            let own = raw["series"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|x| s(x, "asin") == series_asin)
                .cloned()
                .unwrap_or(Value::Null);
            let seq = if s(&own, "sequence").is_empty() {
                r["sequence"].clone()
            } else {
                own["sequence"].clone()
            };
            let pos = series_position(&seq);
            let Some(pos) = pos.as_str().filter(|x| !x.contains('-')).map(str::to_string) else {
                continue;
            };
            let title = [s(&own, "title"), s(&p, "title")]
                .into_iter()
                .find(|x| !x.is_empty())
                .map(|x| json!(x))
                .unwrap_or(book["series"].clone());
            book["series"] = title;
            book["series_asin"] = json!(series_asin);
            book["series_position"] = json!(pos);
            let rank = (
                !language.is_empty() && s(&book, "language").to_lowercase() != language,
                book["format_type"] == "abridged",
            );
            match best.iter_mut().find(|b| b.0 == pos) {
                Some(b) if rank < b.1 => *b = (pos, rank, book),
                Some(_) => {}
                None => best.push((pos, rank, book)),
            }
        }
        let num = |b: &Value| s(b, "series_position").parse::<f64>().unwrap_or(0.0);
        let mut books: Vec<Value> = best.into_iter().map(|b| b.2).collect();
        books.sort_by(|a, b| num(a).partial_cmp(&num(b)).unwrap_or(std::cmp::Ordering::Equal));
        if !language.is_empty() {
            books.retain(|b| matches!(s(b, "language").to_lowercase().as_str(), l if l == language || l.is_empty()));
        }
        let title = match s(&p, "title").trim() {
            "" => books.first().map(|b| s(b, "series").to_string()).unwrap_or_default(),
            x => x.to_string(),
        };
        books.truncate(limit);
        Ok(json!({"asin": series_asin, "title": title, "books": books}))
    }

    /// Audnexus chapter data: {accurate, runtime_ms, chapters: [{start, title}]} – None when there is none.
    pub fn chapters(&self, asin: &str, region: &str) -> Option<Value> {
        let data = self
            .cached(&format!("{AUDNEXUS}/books/{asin}/chapters?region={region}"))
            .ok()?;
        let chs = data["chapters"].as_array().filter(|c| !c.is_empty())?;
        let ms = |v: &Value| v.as_f64().unwrap_or(0.0);
        Some(json!({
            "accurate": data["isAccurate"].as_bool().unwrap_or(false), "runtime_ms": ms(&data["runtimeLengthMs"]) as i64,
            "intro_ms": ms(&data["brandIntroDurationMs"]) as i64, "outro_ms": ms(&data["brandOutroDurationMs"]) as i64,
            "chapters": chs.iter().map(|c| json!({"start": ms(&c["startOffsetMs"]) / 1000.0, "title": s(c, "title").trim()})).collect::<Vec<_>>(),
        }))
    }
}

// -- queries and matching ------------------------------------------------------------------------------------

const NOISE: &str = r"(?i)\b(?:ungek(?:ü|ue)rzt|gek(?:ü|ue)rzt|unabridged|abridged|h(?:ö|oe)rbuch|h(?:ö|oe)rspiel|audiobook|audio\s*book|mp3|m4b|m4a|aac|flac|\d{2,3}\s*kbps|\d{2,3}k|retail|web|rip|german|deutsch|english|vbr|cbr)\b";

fn asin_rx() -> &'static fancy_regex::Regex {
    rx!(r"(?<![A-Z0-9])(B0[0-9A-Z]{8}|\d{9}[\dX])(?![A-Z0-9])")
}

/// Folder/tag text -> search keywords: drop rip cruft, years, CD numbers, brackets.
pub fn clean_query(text: &str) -> String {
    let noise = rx!(NOISE);
    let s = sub(asin_rx(), text, " ");
    let s = sub(rx!(r"(?<=\w)(\w*)\((\w{1,3})\)(?=[\s\-:,.!?]|$)"), &s, "$1$2");
    let s = s.replace('_', " ");
    let s = sub(rx!(r"(?<=\w)\.(?=\w{2,})"), &s, " ");
    let s = rx!(r"[\[({]([^\])}]*)[\])}]")
        .replace_all(&s, |c: &fancy_regex::Captures| {
            let inner = &c[1];
            if is_match(noise, inner) || is_match(rx!(r"^[\s\d.,:/-]*$"), inner) {
                " ".to_string()
            } else {
                format!(" {inner} ")
            }
        })
        .into_owned();
    let s = sub(rx!(r"(?i)\b(?:cd|disc|disk|dvd)\s*\d+\b"), &s, " ");
    let s = sub(noise, &s, " ");
    let s = sub(rx!(r"\b(?:19|20)\d{2}\b"), &s, " ");
    let s = sub(rx!(r"\s*[-–—:|]+\s*"), &s, " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn find_asin(texts: &[&str]) -> Option<String> {
    for text in texts {
        let up = text.to_uppercase();
        if let Some(m) = search(asin_rx(), &up) {
            let a = m[1].to_string();
            if a.starts_with("B0") || up.contains("ASIN") || text.chars().count() <= 12 {
                return Some(a);
            }
        }
    }
    None
}

fn fold(text: &str) -> String {
    let s = text
        .to_lowercase()
        .replace('ß', "ss")
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue");
    let s: String = s.nfkd().filter(|c| canonical_combining_class(*c) == 0).collect();
    sub(rx!(r"[^\w\s]"), &s, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

const STOP: [&str; 19] = [
    "der", "die", "das", "the", "a", "an", "und", "and", "of", "von", "ein", "eine", "teil", "band", "book", "buch",
    "folge", "vol", "volume",
];

fn tokens(text: &str) -> BTreeSet<String> {
    fold(text)
        .split_whitespace()
        .filter(|w| {
            !STOP.contains(w) && *w != "part" && (w.chars().count() > 1 || w.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
        .collect()
}

fn ratio(a: &str, b: &str) -> f64 {
    let (a, b) = (fold(a), fold(b));
    if a.is_empty() || b.is_empty() {
        0.0
    } else {
        difflib::ratio(&a, &b)
    }
}

fn coverage(needle: &str, hay: &BTreeSet<String>) -> f64 {
    let toks = tokens(needle);
    if toks.is_empty() {
        0.0
    } else {
        toks.intersection(hay).count() as f64 / toks.len() as f64
    }
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round_ties_even() / 1000.0
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        other => other.to_string(),
    }
}

/// How well a candidate fits what we know about the files: 0..1 plus parts.
pub fn score(book: &Value, hints: &Value) -> Value {
    let query = s(hints, "query");
    let qtok = tokens(&format!("{query} {} {}", s(hints, "title"), s(hints, "author")));
    let title = s(book, "title");
    let variants = [
        title.to_string(),
        format!("{title} {}", text_of(&book["subtitle"])),
        format!(
            "{} {} {title}",
            text_of(&book["series"]),
            text_of(&book["series_position"])
        ),
    ];
    let t_cov = variants.iter().map(|v| coverage(v, &qtok)).fold(0.0, f64::max);
    let t_ratio = if s(hints, "title").is_empty() {
        0.0
    } else {
        ratio(title, s(hints, "title"))
    };
    let extra = tokens(title).difference(&qtok).count() as f64;
    let t_score = (t_cov - 0.08 * (extra - 1.0).max(0.0)).max(t_ratio);
    let authors = strs(&book["authors"]);
    let a_score = if !s(hints, "author").is_empty() {
        let ha = s(hints, "author");
        let r = authors.iter().map(|a| ratio(a, ha)).fold(0.0, f64::max);
        let atok = tokens(ha);
        r.max(authors.iter().map(|a| coverage(a, &atok)).fold(0.0, f64::max))
    } else {
        authors
            .iter()
            .map(|a| coverage(a.split_whitespace().last().unwrap_or(a), &qtok))
            .fold(0.0, f64::max)
    };
    let n_score = if s(hints, "narrator").is_empty() {
        0.0
    } else {
        strs(&book["narrators"])
            .iter()
            .map(|n| ratio(n, s(hints, "narrator")))
            .fold(0.0, f64::max)
    };
    let dur = hints["duration"].as_f64().unwrap_or(0.0);
    let rt = book["runtime_min"].as_f64().unwrap_or(0.0) * 60.0;
    let d_score = (dur != 0.0 && rt != 0.0).then(|| {
        let diff = (dur - rt).abs() / rt;
        if diff <= 0.02 {
            1.0
        } else {
            (1.0 - (diff - 0.02) / 0.18).max(0.0)
        }
    });
    let mut total = match d_score {
        Some(d) => t_score * 0.55 + a_score * 0.25 + d * 0.20,
        None => t_score * 0.65 + a_score * 0.35,
    };
    if n_score > 0.8 {
        total = (total + 0.03).min(1.0);
    }
    json!({"score": r3(total), "title": r3(t_score), "author": r3(a_score), "duration": d_score.map(r3)})
}

fn sc(b: &Value) -> f64 {
    b["match"]["score"].as_f64().unwrap_or(0.0)
}

pub fn rank(books: &[Value], hints: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = books
        .iter()
        .map(|b| {
            let mut b = b.clone();
            b["match"] = score(&b, hints);
            b
        })
        .collect();
    // ties: unabridged before abridged/dramatised editions
    out.sort_by(|a, b| {
        sc(b)
            .partial_cmp(&sc(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then((a["format_type"] != "unabridged").cmp(&(b["format_type"] != "unabridged")))
    });
    out
}

/// The top candidate if it is confident and clearly ahead of the runner-up.
pub fn best_match(ranked: &[Value], threshold: f64) -> Option<Value> {
    let top = ranked.first()?;
    if sc(top) < threshold {
        return None;
    }
    if top["match"]["duration"].as_f64().is_some_and(|d| d < 0.5) {
        return None;
    }
    if let Some(second) = ranked.get(1) {
        let same_book = fold(s(second, "title")) == fold(s(top, "title")) && second["authors"] == top["authors"];
        if sc(top) - sc(second) < 0.03 && !same_book {
            return None;
        }
    }
    Some(top.clone())
}

pub fn author_words(author: &str) -> Vec<String> {
    let author = match author.split_once(',') {
        Some((last, first)) => format!("{first} {last}"),
        None => author.to_string(),
    };
    rx::findall(rx!(r"[\wäöüß]+"), &author.to_lowercase())
}

pub fn surname_of(author: &str) -> String {
    author_words(author)
        .into_iter()
        .rfind(|w| w.chars().count() > 1)
        .map(|w| capitalize(&w))
        .unwrap_or_default()
}

/// ["Twilight", "Saga", "5", "Biss", …] -> (series words, Some(5), title words).
pub fn split_volume(words: &[String]) -> (Vec<String>, Option<i64>, Vec<String>) {
    for (i, w) in words.iter().enumerate() {
        if let Some(m) = search(rx!(r"^(?:#)?0*(\d{1,3})$"), w.trim_matches(['.', ',', ':', ';', '-']))
            && let Ok(n) = m[1].parse::<i64>()
            && i > 0
            && n > 0
            && n < 500
        {
            return (words[..i].to_vec(), Some(n), words[i + 1..].to_vec());
        }
    }
    (words.to_vec(), None, Vec::new())
}

const TAG_WORDS: [&str; 26] = [
    "de",
    "en",
    "ger",
    "eng",
    "dl",
    "mp3",
    "m4b",
    "flac",
    "hoerbuch",
    "hörbuch",
    "audiobook",
    "ungekuerzt",
    "ungekürzt",
    "unabridged",
    "gekuerzt",
    "gekürzt",
    "abridged",
    "retail",
    "repack",
    "proper",
    "cd",
    "cds",
    "german",
    "deutsch",
    "english",
    "englisch",
];

/// Searches built from what a folder is called – for when the full search finds nothing or only other books.
pub fn content_queries(query: &str, author: &str, limit: usize) -> Vec<String> {
    let names: BTreeSet<String> = author_words(author).into_iter().collect();
    let surname = surname_of(author);
    let words: Vec<String> = rx::split(rx!(r"[\s._]+"), query)
        .into_iter()
        .filter(|w| {
            let bare = w.trim_matches([',', ';', ':']);
            !w.is_empty()
                && !is_match(rx!(r"^\w*[a-zäöüß][A-ZÄÖÜ]{2,}[a-zäöüß]\w*$"), bare)
                && !TAG_WORDS.contains(&bare.to_lowercase().as_str())
                && !is_match(rx!(r"(?i)^(?:\d+\s*cds?|\d{4})$"), w)
        })
        .collect();
    let core: Vec<String> = words
        .iter()
        .filter(|w| !names.contains(w.to_lowercase().trim_matches([',', '.', ';', ':'])))
        .cloned()
        .collect();
    let with_name = |ws: &[String]| {
        if surname.is_empty() {
            ws.join(" ")
        } else {
            [ws.to_vec(), vec![surname.clone()]].concat().join(" ")
        }
    };
    let mut out = vec![with_name(&core)];
    let (series, volume, title) = split_volume(&core);
    if volume.is_some() {
        if title.len() >= 2 {
            out.push(with_name(&title[..title.len().min(6)]));
        }
        if title.len() >= 3 {
            out.push(with_name(&title[1..title.len().min(6)]));
        }
        if !series.is_empty() {
            out.push(with_name(&series[..series.len().min(4)]));
        }
    }
    for q in out.clone() {
        // "Maerchen" -> "Märchen" (not after a vowel: "grauen", "Feuer", "neue" stay)
        let mut back = q.clone();
        for (a, b) in [
            ("ae", "ä"),
            ("oe", "ö"),
            ("ue", "ü"),
            ("Ae", "Ä"),
            ("Oe", "Ö"),
            ("Ue", "Ü"),
        ] {
            if let Ok(re) = fancy_regex::Regex::new(&format!("(?<![aeiouäöüAEIOU]){a}")) {
                back = sub(&re, &back, b);
            }
        }
        if back != q {
            out.push(back);
        }
    }
    if core.len() > 4 {
        out.push(with_name(&core[..4]));
    }
    if words.len() > 3 {
        out.push(words[words.len() - 3..].join(" "));
    }
    let mut seen: Vec<String> = Vec::new();
    for q in out.into_iter().map(|q| q.trim().to_string()) {
        if !q.is_empty() && q != query.trim() && !seen.contains(&q) {
            seen.push(q);
        }
    }
    seen.truncate(limit);
    seen
}

fn with_query(hints: &Value, q: &str) -> Value {
    let mut h = hints.clone();
    h["query"] = json!(q);
    h
}

/// Search strategy: ASIN -> title+author -> keywords -> shorter searches; results ranked.
pub fn lookup(client: &Client, hints: &Value, region: &str, limit: usize) -> Result<Vec<Value>> {
    if let Some(asin) = hints["asin"].as_str().filter(|a| !a.is_empty())
        && let Ok(p) = client.product(asin, region)
    {
        return Ok(rank(&[p], &with_query(hints, "")));
    }
    let mut order: Vec<String> = Vec::new();
    let mut results: HashMap<String, Value> = HashMap::new();
    let add = |found: Vec<Value>, order: &mut Vec<String>, results: &mut HashMap<String, Value>| {
        for b in found {
            let k = text_of(&b["asin"]);
            if let std::collections::hash_map::Entry::Vacant(e) = results.entry(k.clone()) {
                order.push(k);
                e.insert(b);
            }
        }
    };
    let (title, author) = (s(hints, "title"), s(hints, "author"));
    if !title.is_empty() && !author.is_empty() {
        add(
            client.search(region, "", title, author, limit, false)?,
            &mut order,
            &mut results,
        );
    }
    let query = if s(hints, "query").is_empty() {
        [title, author]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        s(hints, "query").to_string()
    };
    if !query.is_empty() && results.len() < 3 {
        add(
            client.search(region, &query, "", "", limit, false)?,
            &mut order,
            &mut results,
        );
    }
    if results.is_empty() {
        for q in content_queries(&query, author, 8) {
            add(
                client.search(region, &q, "", "", limit, false)?,
                &mut order,
                &mut results,
            );
            if !results.is_empty() {
                break;
            }
        }
    }
    let mut hints = hints.clone();
    if results.is_empty() && !s(&hints, "alt_query").is_empty() {
        let alt = s(&hints, "alt_query").to_string();
        add(
            client.search(region, &alt, "", "", limit, false)?,
            &mut order,
            &mut results,
        );
        hints["query"] = json!(alt);
    }
    let books: Vec<Value> = order.into_iter().filter_map(|k| results.remove(&k)).collect();
    Ok(rank(&books, &hints))
}

// -- the other shops (providers.py) -------------------------------------------------------------------------

fn host_ok(url: &str) -> bool {
    url.to_lowercase().starts_with("https://")
        && matches!(
            hostname(url).to_lowercase().as_str(),
            "api.audnex.us" | "itunes.apple.com" | "openlibrary.org"
        )
}

fn provider_json(url: &str) -> Result<Value> {
    let data = http_get(url, 15, 2, true, MAX_RESPONSE, Some(&host_ok))?;
    serde_json::from_slice(&data)
        .map_err(|_| AudibleError(t("Ungültige Antwort von {url}", &[("url", &hostname(url))])))
}

fn book(source: &str, fields: Value) -> Value {
    let mut b = json!({"asin": null, "title": "", "subtitle": null, "authors": [], "narrators": [], "translators": [],
                       "series": null, "series_position": null, "publisher": null, "release_date": null,
                       "language": null, "runtime_min": 0, "format_type": null, "genres": [], "tags": [],
                       "description": "", "cover_url": null, "region": null, "url": null, "source": source, "isbn": null});
    for (k, v) in fields.as_object().cloned().unwrap_or_default() {
        if !v.is_null() {
            b[k] = v;
        }
    }
    b
}

fn names(text: &str) -> Vec<String> {
    rx::split(rx!(r"\s*(?:,|&|\bund\b|\band\b)\s*"), text)
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

pub fn from_audnexus(d: &Value, region: &str) -> Value {
    let series = &d["seriesPrimary"];
    let list = |k: &str, kind: Option<&str>| -> Vec<String> {
        d[k].as_array()
            .into_iter()
            .flatten()
            .filter(|g| kind.is_none_or(|k| g["type"] == k))
            .filter_map(|g| g["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string))
            .collect()
    };
    let release: String = s(d, "releaseDate").chars().take(10).collect();
    book(
        "audnexus",
        json!({
            "id": d["asin"], "asin": d["asin"], "title": s(d, "title").trim(), "subtitle": opt_str(s(d, "subtitle").trim()),
            "authors": list("authors", None), "narrators": list("narrators", None), "series": series["name"],
            "series_position": series_position(&series["position"]), "publisher": d["publisherName"], "release_date": opt_str(&release),
            "language": opt_str(&capitalize(s(d, "language"))), "runtime_min": d["runtimeLengthMin"].as_f64().unwrap_or(0.0) as i64,
            "format_type": d["formatType"], "genres": list("genres", Some("genre")), "tags": list("genres", Some("tag")),
            "description": html_to_text([s(d, "summary"), s(d, "description")].into_iter().find(|x| !x.is_empty()).unwrap_or("")),
            "cover_url": d["image"], "region": region, "url": store_url(s(d, "asin"), region),
        }),
    )
}

pub fn from_itunes(d: &Value) -> Value {
    let cover = [s(d, "artworkUrl100"), s(d, "artworkUrl60")]
        .into_iter()
        .find(|x| !x.is_empty())
        .map(|c| sub(rx!(r"/\d+x\d+bb\.(jpg|png)$"), c, "/1400x1400bb.$1"));
    let title = sub(
        rx!(r"(?i)\s*\((?:ungekürzt|gekürzt|unabridged|abridged)\)\s*$"),
        s(d, "collectionName"),
        "",
    );
    let release: String = s(d, "releaseDate").chars().take(10).collect();
    let url = s(d, "collectionViewUrl").split('?').next().unwrap_or("").to_string();
    book(
        "apple",
        json!({
            "id": format!("apple:{}", if d["collectionId"].is_null() { "None".to_string() } else { text_of(&d["collectionId"]) }), "title": title.trim(), "authors": names(s(d, "artistName")),
            "release_date": opt_str(&release), "genres": if s(d, "primaryGenreName").is_empty() { json!([]) } else { json!([s(d, "primaryGenreName")]) },
            "description": html_to_text(s(d, "description")), "cover_url": cover, "url": opt_str(&url),
        }),
    )
}

pub fn from_openlibrary(d: &Value) -> Value {
    let key = text_of(&d["key"]);
    book(
        "openlibrary",
        json!({
            "id": format!("openlibrary:{key}"), "title": s(d, "title").trim(), "subtitle": d["subtitle"],
            "authors": strs(&d["author_name"]).into_iter().take(4).collect::<Vec<_>>(),
            "release_date": if d["first_publish_year"].is_null() { Value::Null } else { json!(text_of(&d["first_publish_year"])) },
            "cover_url": if d["cover_i"].is_null() { Value::Null } else { json!(format!("https://covers.openlibrary.org/b/id/{}-L.jpg", text_of(&d["cover_i"]))) },
            "url": if key.is_empty() { Value::Null } else { json!(format!("https://openlibrary.org{key}")) },
            "isbn": d["isbn"].as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null),
        }),
    )
}

fn itunes_country(region: &str) -> &'static str {
    match region {
        "de" => "de",
        "uk" => "gb",
        "au" => "au",
        "ca" => "ca",
        "fr" => "fr",
        "it" => "it",
        "es" => "es",
        "in" => "in",
        "jp" => "jp",
        _ => "us",
    }
}

/// Audible first; Audnexus, Apple Books and Open Library when Audible has nothing (setting "ab_fallback").
pub struct Providers {
    pub client: Client,
    get: Box<JsonGetter>,
    pub fallbacks: Vec<&'static str>,
}

impl Default for Providers {
    fn default() -> Self {
        Providers {
            client: Client::default(),
            get: Box::new(provider_json),
            fallbacks: vec!["audnexus", "apple", "openlibrary"],
        }
    }
}

impl Providers {
    pub fn with(client: Client, get: Box<JsonGetter>, fallbacks: Vec<&'static str>) -> Self {
        Providers { client, get, fallbacks }
    }

    fn apple(&self, hints: &Value, region: &str) -> Result<Vec<Value>> {
        let ta = [s(hints, "title"), s(hints, "author")]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let term = if ta.is_empty() {
            s(hints, "query").to_string()
        } else {
            ta
        };
        if term.is_empty() {
            return Ok(Vec::new());
        }
        let params = [
            ("term", term),
            ("media", "audiobook".into()),
            ("country", itunes_country(region).into()),
            ("limit", "8".into()),
        ];
        let data = (self.get)(&format!("https://itunes.apple.com/search?{}", form(&params)))?;
        Ok(data["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| !s(d, "collectionName").is_empty())
            .map(from_itunes)
            .collect())
    }

    fn openlibrary(&self, hints: &Value) -> Result<Vec<Value>> {
        let mut params = vec![
            ("limit", "5".to_string()),
            (
                "fields",
                "key,title,subtitle,author_name,first_publish_year,cover_i,isbn".into(),
            ),
        ];
        if !s(hints, "title").is_empty() {
            params.push(("title", s(hints, "title").into()));
            if !s(hints, "author").is_empty() {
                params.push(("author", s(hints, "author").into()));
            }
        } else if !s(hints, "query").is_empty() {
            params.push(("q", s(hints, "query").into()));
        } else {
            return Ok(Vec::new());
        }
        let data = (self.get)(&format!("https://openlibrary.org/search.json?{}", form(&params)))?;
        Ok(data["docs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| !s(d, "title").is_empty())
            .map(from_openlibrary)
            .collect())
    }

    /// (ranked candidates, notes for the job log, Audible failed)
    pub fn lookup_full(&self, hints: &Value, region: &str, enabled: bool) -> (Vec<Value>, Vec<String>, bool) {
        let mut notes = Vec::new();
        let (ranked, failed) = match lookup(&self.client, hints, region, 10) {
            Ok(r) => (r, false),
            Err(e) => {
                notes.push(if e.0.starts_with("Audible") {
                    e.0
                } else {
                    t("Audible nicht erreichbar: {error}", &[("error", &e.0)])
                });
                (Vec::new(), true)
            }
        };
        if !ranked.is_empty() || !enabled {
            return (ranked, notes, failed);
        }
        let mut found: Vec<Value> = Vec::new();
        if failed && !s(hints, "asin").is_empty() && self.fallbacks.contains(&"audnexus") {
            match (self.get)(&format!(
                "https://api.audnex.us/books/{}?region={region}",
                s(hints, "asin")
            )) {
                Ok(d) => {
                    found.push(from_audnexus(&d, region));
                    notes.push(t("Daten von Audnexus (Audible-Spiegel)", &[]));
                }
                Err(e) => notes.push(format!("Audnexus: {e}")),
            }
        }
        if found.is_empty() {
            for (name, key) in [("Apple Books", "apple"), ("Open Library", "openlibrary")] {
                if !self.fallbacks.contains(&key) {
                    continue;
                }
                let hits = if key == "apple" {
                    self.apple(hints, region)
                } else {
                    self.openlibrary(hints)
                };
                match hits {
                    Err(e) => notes.push(format!("{name}: {e}")),
                    Ok(h) => {
                        if !h.is_empty() {
                            notes.push(t(
                                "{n} Treffer bei {source}",
                                &[("n", &h.len().to_string()), ("source", name)],
                            ));
                        }
                        found.extend(h);
                    }
                }
            }
        }
        (rank(&found, hints), notes, failed)
    }

    pub fn lookup(&self, hints: &Value, region: &str, enabled: bool) -> (Vec<Value>, Vec<String>) {
        let (r, n, _) = self.lookup_full(hints, region, enabled);
        (r, n)
    }

    /// Manual search in the review: Audible errors are reported, the other shops fill gaps.
    pub fn search(&self, hints: &Value, region: &str, enabled: bool) -> Result<Vec<Value>> {
        let (ranked, notes, failed) = self.lookup_full(hints, region, enabled);
        if ranked.is_empty() && failed {
            return err(notes.first().cloned().unwrap_or_default());
        }
        Ok(ranked)
    }
}

/// Automatic acceptance only for Audible data (runtime and narrators known).
pub fn auto_match(ranked: &[Value], threshold: f64) -> Option<Value> {
    let only: Vec<Value> = ranked
        .iter()
        .filter(|b| AUTO_SOURCES.contains(&s(b, "source")))
        .cloned()
        .collect();
    best_match(&only, threshold)
}

pub fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn like_python() {
        crate::text::set_language("de");
        let g: Value =
            serde_json::from_str(include_str!("../../../contract/golden/audible.json")).expect("audible.json");
        let books: Vec<Value> = g["products"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| normalize_product(p, "de"))
            .collect();
        for (a, b) in books.iter().zip(g["normalized"].as_array().unwrap()) {
            assert_eq!(a, b);
        }
        for (i, h) in g["hints"].as_array().unwrap().iter().enumerate() {
            let r = rank(&books, h);
            assert_eq!(json!(r), g["ranked"][i], "rank {i}");
            let best: Vec<Value> = [0.5, 0.8, 0.95]
                .iter()
                .map(|th| best_match(&r, *th).map_or(Value::Null, |b| b["asin"].clone()))
                .collect();
            assert_eq!(json!(best), g["best"][i], "best {i}");
            assert_eq!(
                auto_match(&r, 0.5).map_or(Value::Null, |b| b["asin"].clone()),
                g["auto"][i]
            );
        }
        for c in g["clean_query"].as_array().unwrap() {
            assert_eq!(clean_query(c[0].as_str().unwrap()), c[1].as_str().unwrap(), "{}", c[0]);
        }
        for c in g["find_asin"].as_array().unwrap() {
            let texts: Vec<&str> = c[0].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
            assert_eq!(json!(find_asin(&texts)), c[1], "{}", c[0]);
        }
        for c in g["content_queries"].as_array().unwrap() {
            assert_eq!(
                json!(content_queries(c[0].as_str().unwrap(), c[1].as_str().unwrap(), 8)),
                c[2],
                "{}",
                c[0]
            );
        }
        for c in g["ratio"].as_array().unwrap() {
            assert_eq!(
                difflib::ratio(c[0].as_str().unwrap(), c[1].as_str().unwrap()),
                c[2].as_f64().unwrap()
            );
        }
        for c in g["series_position"].as_array().unwrap() {
            assert_eq!(series_position(&c[0]), c[1], "{}", c[0]);
        }
        for c in g["lookup"].as_array().unwrap() {
            let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let log = calls.clone();
            let products = g["products"].clone();
            let client = Client::new(Box::new(move |url: &str| {
                log.lock().unwrap().push(url.to_string());
                Ok(if url.contains("/catalog/products/B0STORM001") {
                    json!({"product": products[0]})
                } else if url.contains("keywords=Kurz") || url.contains("keywords=Schimmel") {
                    json!({"products": [products[0], products[1], products[2]]})
                } else {
                    json!({"products": []})
                })
            }));
            let got: Vec<Value> = lookup(&client, &c[0], "de", 10)
                .unwrap()
                .iter()
                .map(|b| b["asin"].clone())
                .collect();
            assert_eq!(json!(got), c[1], "lookup {}", c[0]);
            assert_eq!(json!(*calls.lock().unwrap()), c[2], "calls {}", c[0]);
        }
        let n: Value = serde_json::from_value(json!({"asin": "B0N", "title": "T ", "authors": [{"name": "A"}],
            "genres": [{"type": "genre", "name": "G"}, {"type": "tag", "name": "X"}], "seriesPrimary": {"name": "S", "position": "2"},
            "language": "english", "releaseDate": "2020-01-02T00:00:00Z", "summary": "<p>Hi</p>"})).unwrap();
        assert_eq!(from_audnexus(&n, "us"), g["audnexus"]);
        assert_eq!(
            from_itunes(
                &json!({"collectionId": 7, "collectionName": "Titel (Ungekürzt)", "artistName": "A & B und C",
            "artworkUrl100": "https://x/100x100bb.jpg", "collectionViewUrl": "https://a/b?c=d"})
            ),
            g["itunes"]
        );
        assert_eq!(
            from_openlibrary(&json!({"key": "/works/OL1W", "title": "T", "author_name": ["A", "B"],
            "first_publish_year": 1888, "cover_i": 5, "isbn": ["123"]})),
            g["openlibrary"]
        );
    }

    #[test]
    fn links_and_series_like_python() {
        crate::text::set_language("de");
        let g: Value =
            serde_json::from_str(include_str!("../../../contract/golden/audible.json")).expect("audible.json");
        let src = include_str!("../../../contract/make_audible.py");
        let grab = |name: &str| -> String {
            let start = src.find(&format!("{name} = \"\"\"")).unwrap() + name.len() + 6;
            let end = start + src[start..].find("\"\"\"").unwrap();
            src[start..end].to_string()
        };
        assert_eq!(
            crate::links::parse_product_page(&grab("PAGE"), "https://www.thalia.de/x", "thalia").unwrap(),
            g["page"]
        );
        assert_eq!(
            crate::links::parse_product_page(&grab("PAGE_OG"), "https://www.thalia.de/y", "thalia").unwrap(),
            g["page_og"]
        );
        let bb: Value = serde_json::from_value(json!({"id": 4711, "title": "Titel ", "author": "A & B", "narrator": ["C", {"name": "D"}],
            "series": {"name": "S", "partnumber": 2}, "language": "german", "audiobooklength": 3725,
            "editions": [{"format": "Ebook", "isbn": "1"}, {"format": "Audiobook", "isbn": "2", "publisher": "P", "published": "2021-02-03T00:00:00Z"}],
            "genres": [{"name": "G"}, {}], "summary": "<b>Hi</b>", "cover": "https://c.example/x.jpg?w=200&h=1"})).unwrap();
        assert_eq!(crate::links::normalize_bookbeat(&bb, ""), g["bookbeat"]);
        for c in g["shops"].as_array().unwrap() {
            assert_eq!(json!(crate::links::shop_of(c[0].as_str().unwrap())), c[1], "{}", c[0]);
        }
        let series = json!({"relationships": [
            {"asin": "B0VOL00002", "relationship_type": "series", "relationship_to_product": "child", "sort": "2", "sequence": "2"},
            {"asin": "B0VOL00001", "relationship_type": "series", "relationship_to_product": "child", "sort": "1", "sequence": "1"},
            {"asin": "B0BOX00013", "relationship_type": "series", "relationship_to_product": "child", "sort": "3", "sequence": "1-3"},
            {"asin": "B0VOL0001E", "relationship_type": "series", "relationship_to_product": "child", "sort": "1.5", "sequence": "1"}],
            "title": "Die Reihe"});
        let products = json!([
            {"asin": "B0VOL00001", "title": "Eins", "language": "german", "format_type": "unabridged", "authors": [{"name": "A"}]},
            {"asin": "B0VOL0001E", "title": "One", "language": "english", "format_type": "unabridged", "authors": [{"name": "A"}]},
            {"asin": "B0VOL00002", "title": "Zwei", "language": "german", "format_type": "abridged", "authors": [{"name": "A"}],
             "series": [{"asin": "B0SERIES01", "title": "Die Reihe (Eigentitel)", "sequence": "2"}]},
            {"asin": "B0BOX00013", "title": "Box", "language": "german", "authors": [{"name": "A"}]}]);
        let client = Client::new(Box::new(move |url: &str| {
            Ok(if url.contains("/catalog/products/B0SERIES01") {
                json!({"product": series})
            } else {
                json!({"products": products})
            })
        }));
        assert_eq!(
            client.series("B0SERIES01", "de", "", SERIES_MAX).unwrap(),
            g["series"]["all"]
        );
        assert_eq!(
            client.series("B0SERIES01", "de", "german", SERIES_MAX).unwrap(),
            g["series"]["german"]
        );
    }

    #[test]
    fn only_public_addresses() {
        for bad in [
            "http://127.0.0.1/x.jpg",
            "http://10.0.0.1/",
            "http://[::1]/",
            "http://169.254.169.254/latest",
            "file:///etc/passwd",
            "http://localhost/",
        ] {
            assert!(!is_public_url(bad), "{bad}");
        }
        assert!(global("8.8.8.8".parse().unwrap()) && global("2a00:1450::1".parse().unwrap()));
    }
}
