//! Book data from a shop link: Audible, BookBeat, Thalia. Only these shops are fetched – no arbitrary
//! URLs; Audible links resolve to the ASIN, BookBeat to its public book API, Thalia through the schema.org data
//! embedded in the product page.

use serde_json::{Value, json};

use crate::audible::{self, AudibleError, Client, REGIONS, form, html_to_text, series_position, unescape};
use crate::rx;
use crate::text::t;

const BOOKBEAT_API: &str = "https://api.bookbeat.com/api/books/";
const BOOKBEAT_SEARCH: &str = "https://api.bookbeat.com/api/search/books";

#[derive(Debug)]
pub struct LinkError(pub String);

type Res<T> = Result<T, LinkError>;

pub fn is_link(text: &str) -> bool {
    rx::is_match(rx!(r"(?i)^\s*(https?://|www\.)"), text)
}

fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |x| x.1);
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    let netloc = netloc.rsplit_once('@').map_or(netloc, |x| x.1);
    netloc.split(':').next().unwrap_or("").to_lowercase()
}

pub fn shop_of(url: &str) -> Option<&'static str> {
    let h = host(url);
    if rx::is_match(
        rx!(r"(?:^|\.)audible\.(de|com|co\.uk|fr|it|es|ca|com\.au|in|co\.jp)$"),
        &h,
    ) {
        Some("audible")
    } else if rx::is_match(rx!(r"(?:^|\.)bookbeat\.[a-z]{2,3}$"), &h) {
        Some("bookbeat")
    } else if rx::is_match(rx!(r"(?:^|\.)thalia\.(de|at|ch)$"), &h) {
        Some("thalia")
    } else {
        None
    }
}

fn split_names(value: &Value) -> Vec<String> {
    match value {
        Value::Null | Value::Bool(false) => Vec::new(),
        Value::String(s) if s.is_empty() => Vec::new(),
        Value::Array(a) => a.iter().flat_map(split_names).collect(),
        Value::Object(o) => split_names(o.get("name").unwrap_or(&Value::Null)),
        other => {
            let text = match other {
                Value::String(s) => s.clone(),
                x => x.to_string(),
            };
            // entities first: "Max &amp; Moritz" are two names, not "Max", "amp" and "Moritz"
            rx::split(rx!(r"\s*(?:,|;|&|\bund\b|\band\b)\s*"), &unescape(&text))
                .iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        }
    }
}

fn empty_book(source: &str, url: &str) -> Value {
    json!({"asin": null, "title": "", "subtitle": null, "authors": [], "narrators": [], "translators": [],
           "series": null, "series_position": null, "publisher": null, "release_date": null,
           "language": null, "runtime_min": 0, "format_type": null, "genres": [], "tags": [],
           "description": "", "cover_url": null, "region": null, "url": if url.is_empty() { Value::Null } else { json!(url) },
           "source": source, "isbn": null})
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn opt(v: &str) -> Value {
    if v.is_empty() { Value::Null } else { json!(v) }
}

fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn audible_link(url: &str, client: &Client) -> Res<Value> {
    let h = host(url);
    let tld = rx::search(
        rx!(r"(?:^|\.)audible\.(de|com|co\.uk|fr|it|es|ca|com\.au|in|co\.jp)$"),
        &h,
    )
    .map(|m| m[1].to_string())
    .unwrap_or_default();
    let rest = url.split_once("://").map_or(url, |x| x.1);
    let path_query = rest
        .find('/')
        .map_or("", |i| &rest[i..])
        .replace('?', "/")
        .to_uppercase();
    let asin =
        rx::search(rx!(r"(?<![A-Z0-9])(B0[0-9A-Z]{8}|\d{9}[\dX])(?![A-Z0-9])"), &path_query).map(|m| m[1].to_string());
    let Some(asin) = asin else {
        return Err(LinkError(t("Keine ASIN im Audible-Link gefunden", &[])));
    };
    let region = REGIONS.iter().find(|r| r.1 == tld).map_or("us", |r| r.0);
    client.product(&asin, region).map_err(|e| LinkError(e.0))
}

fn bookbeat_ok(url: &str) -> bool {
    url.to_lowercase().starts_with("https://") && host(url) == "api.bookbeat.com"
}

fn bookbeat_id(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |x| x.1);
    let path = rest
        .find('/')
        .map_or("", |i| &rest[i..])
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    rx::search(rx!(r"/book/(?:[^/?#]*-)?(\d+)(?:[/?#]|$)"), &format!("{path}/")).map(|m| m[1].to_string())
}

fn wide_cover(cover: &str, w: &str) -> String {
    rx::sub(rx!(r"([?&])w=\d+"), cover, &format!("${{1}}w={w}"))
}

pub fn normalize_bookbeat(d: &Value, url: &str) -> Value {
    let mut book = empty_book("bookbeat", url);
    let editions = d["editions"].as_array().cloned().unwrap_or_default();
    let audio = editions
        .iter()
        .find(|e| text(&e["format"]).to_lowercase().starts_with("audio"))
        .cloned();
    let main = audio.clone().or_else(|| editions.first().cloned()).unwrap_or(json!({}));
    let series = &d["series"];
    let cover = s(d, "cover");
    let part = [&series["displaypartnumber"], &series["partnumber"]]
        .into_iter()
        .map(text)
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    let published: String = [s(&main, "published"), s(d, "published")]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or("")
        .chars()
        .take(10)
        .collect();
    let fields = json!({
        "title": s(d, "title").trim(), "subtitle": opt(s(d, "subtitle").trim()),
        "authors": split_names(&d["author"]), "narrators": split_names(&d["narrator"]), "translators": split_names(&d["translator"]),
        "series": series["name"], "series_position": series_position(&json!(part)), "publisher": main["publisher"],
        "release_date": opt(&published), "language": d["language"],
        "runtime_min": crate::releases::py_round(d["audiobooklength"].as_f64().unwrap_or(0.0) / 60.0),
        "genres": d["genres"].as_array().into_iter().flatten().filter_map(|g| g["name"].as_str().filter(|n| !n.is_empty())).collect::<Vec<_>>(),
        "description": html_to_text(s(d, "summary")),
        "cover_url": if cover.is_empty() { Value::Null } else if cover.contains("w=") { json!(wide_cover(cover, "1400")) } else { json!(cover) },
        "isbn": main["isbn"], "format_type": if audio.is_some() { json!("unabridged") } else { Value::Null },
        "id": format!("bookbeat:{}", if d["id"].is_null() { "None".to_string() } else { text(&d["id"]) }),
    });
    for (k, v) in fields.as_object().cloned().unwrap_or_default() {
        book[k] = v;
    }
    if !d["id"].is_null() {
        book["url"] = json!(if url.is_empty() {
            format!("https://www.bookbeat.com/de/book/{}", text(&d["id"]))
        } else {
            url.to_string()
        });
    }
    book
}

fn bookbeat_hit(d: &Value) -> Value {
    let mut book = empty_book("bookbeat", "");
    let image = s(d, "image");
    let share = s(d, "shareurl");
    let published: String = s(d, "published").chars().take(10).collect();
    let fields = json!({
        "title": s(d, "title").trim(), "authors": split_names(&d["author"]),
        "series": if d["series"].is_object() { d["series"]["name"].clone() } else { Value::Null },
        "language": opt(&s(d, "language").to_lowercase()), "release_date": opt(&published),
        "cover_url": if image.starts_with("https://") { json!(wide_cover(image, "500")) } else { Value::Null },
        "isbn": d["audiobookisbn"], "id": format!("bookbeat:{}", if d["id"].is_null() { "None".to_string() } else { text(&d["id"]) }),
        "url": if share.starts_with("https://") { json!(share) } else { Value::Null },
    });
    for (k, v) in fields.as_object().cloned().unwrap_or_default() {
        book[k] = v;
    }
    book
}

/// Audiobooks at BookBeat for a search term (its public search; shorter versions too). Blocking.
pub fn bookbeat_search(query: &str, region: &str, limit: usize) -> Vec<Value> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if region != "de" || words.is_empty() {
        return Vec::new();
    }
    let mut tries = vec![words.join(" ")];
    if words.len() > 2 {
        tries.push(words[1..].join(" "));
        tries.push(words[..words.len() - 1].join(" "));
    }
    for q in tries {
        let url = format!(
            "{BOOKBEAT_SEARCH}?{}",
            form(&[("query", q), ("market", "Germany".into())])
        );
        let Ok(raw) = audible::http_get(&url, 20, 3, true, 8 << 20, Some(&bookbeat_ok)) else {
            return Vec::new();
        };
        let Ok(data) = serde_json::from_slice::<Value>(&raw) else {
            return Vec::new();
        };
        let hits: Vec<Value> = data["_embedded"]["books"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| !text(&b["audiobookisbn"]).is_empty())
            .cloned()
            .collect();
        if !hits.is_empty() {
            return hits.iter().take(limit).map(bookbeat_hit).collect();
        }
    }
    Vec::new()
}

fn audio_edition(d: &Value) -> bool {
    d["audiobooklength"].as_f64().is_some_and(|x| x != 0.0)
        || d["editions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| text(&e["format"]).to_lowercase().starts_with("audio"))
}

fn bookbeat_link(url: &str) -> Res<Value> {
    let id = bookbeat_id(url).ok_or_else(|| {
        LinkError(t(
            "Keine Hörbuch-ID im BookBeat-Link gefunden (…/book/titel-123456)",
            &[],
        ))
    })?;
    let n: u64 = id.parse().map_err(|_| {
        LinkError(t(
            "Keine Hörbuch-ID im BookBeat-Link gefunden (…/book/titel-123456)",
            &[],
        ))
    })?;
    let data = audible::http_get(&format!("{BOOKBEAT_API}{n}"), 20, 3, true, 8 << 20, Some(&bookbeat_ok))
        .map_err(|e| e.0)
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).map_err(|e| e.to_string()))
        .map_err(|e| LinkError(t("BookBeat-Daten nicht abrufbar: {error}", &[("error", &e)])))?;
    if !audio_edition(&data) {
        return Err(LinkError(t(
            "Bei BookBeat gibt es dieses Hörbuch nur als E-Book, nicht als Hörbuch",
            &[],
        )));
    }
    Ok(normalize_bookbeat(&data, url))
}

fn iso_minutes(value: &str) -> i64 {
    let Some(m) = rx::search(rx!(r"^P(?:\d+D)?T?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$"), value.trim()) else {
        return 0;
    };
    let n = |i: usize| m.get(i).and_then(|x| x.as_str().parse::<f64>().ok()).unwrap_or(0.0);
    crate::releases::py_round(n(1) * 60.0 + n(2) + n(3) / 60.0)
}

fn nodes(data: &Value) -> Vec<Value> {
    match data {
        Value::Array(a) => a.iter().flat_map(nodes).collect(),
        Value::Object(o) => std::iter::once(data.clone())
            .chain(nodes(o.get("@graph").unwrap_or(&json!([]))))
            .collect(),
        _ => Vec::new(),
    }
}

fn types(node: &Value) -> Vec<String> {
    match &node["@type"] {
        Value::Array(a) => a.iter().map(|x| text(x).to_lowercase()).collect(),
        Value::Null => Vec::new(),
        other => vec![text(other).to_lowercase()],
    }
}

fn image(v: &Value) -> Value {
    match v {
        Value::Array(a) => a.first().map(image).unwrap_or(Value::Null),
        Value::Object(o) => o
            .get("url")
            .or_else(|| o.get("contentUrl"))
            .cloned()
            .unwrap_or(Value::Null),
        Value::String(s) if !s.is_empty() => v.clone(),
        _ => Value::Null,
    }
}

/// Only http(s) links leave the server (no javascript:/data: from foreign pages).
pub fn safe_url(v: &Value) -> Value {
    match v.as_str() {
        Some(u) if rx::is_match(rx!(r#"(?i)^https?://[^\s"'<>]+$"#), u) => v.clone(),
        _ => Value::Null,
    }
}

/// schema.org Book/Audiobook/Product (JSON-LD), falling back to OpenGraph tags.
pub fn parse_product_page(html: &str, url: &str, source: &str) -> Res<Value> {
    let mut all: Vec<Value> = Vec::new();
    for c in rx!(r"(?is)<script[^>]*application/ld\+json[^>]*>(.*?)</script>")
        .captures_iter(html)
        .filter_map(Result::ok)
    {
        if let Ok(v) = serde_json::from_str::<Value>(c[1].trim()) {
            all.extend(nodes(&v));
        }
    }
    let mut book = empty_book(source, url);
    let main = all
        .iter()
        .find(|n| types(n).contains(&"audiobook".to_string()))
        .or_else(|| {
            all.iter()
                .find(|n| types(n).iter().any(|t| t == "book" || t == "product"))
        })
        .cloned();
    if let Some(main) = main {
        let editions = match &main["workExample"] {
            Value::Array(a) => a.clone(),
            Value::Null => Vec::new(),
            other => vec![other.clone()],
        };
        let audio = editions
            .iter()
            .find(|e| {
                e.is_object()
                    && (types(e).contains(&"audiobook".to_string())
                        || text(&e["bookFormat"]).to_lowercase().contains("audiobook"))
            })
            .cloned()
            .unwrap_or(json!({}));
        let get = |k: &str| -> Value {
            let a = &audio[k];
            let empty =
                matches!(a, Value::Null | Value::Bool(false)) || a == &json!("") || a == &json!([]) || a == &json!({});
            if empty { main[k].clone() } else { a.clone() }
        };
        let part_of = match &main["isPartOf"] {
            Value::Array(a) => a.first().cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        let publisher = {
            let p = get("publisher");
            if p.is_null() || p == json!("") { get("brand") } else { p }
        };
        let in_lang = get("inLanguage");
        let duration = {
            let d = get("duration");
            if d.is_null() || d == json!("") {
                get("timeRequired")
            } else {
                d
            }
        };
        let isbn = ["isbn", "gtin13", "gtin"]
            .iter()
            .map(|k| get(k))
            .find(|v| !v.is_null() && v != &json!(""))
            .unwrap_or(Value::Null);
        let date: String = text(&get("datePublished")).chars().take(10).collect();
        book["title"] = json!(unescape(&text(&main["name"])).trim());
        book["authors"] = json!(split_names(&main["author"]));
        book["narrators"] = json!(split_names(&get("readBy")));
        book["translators"] = json!(split_names(&get("translator")));
        let pubname = if publisher.is_object() {
            publisher["name"].clone()
        } else {
            publisher
        };
        book["publisher"] = if pubname.is_null() || pubname == json!("") {
            Value::Null
        } else {
            pubname
        };
        book["release_date"] = opt(&date);
        book["language"] = if in_lang.is_string() { in_lang } else { Value::Null };
        book["runtime_min"] = json!(iso_minutes(&text(&duration)));
        book["description"] = json!(html_to_text(&unescape(&text(&main["description"]))));
        book["cover_url"] = image(&main["image"]);
        book["isbn"] = isbn;
        book["genres"] = if main["genre"].is_null() {
            json!([])
        } else {
            json!(split_names(&main["genre"]))
        };
        if part_of.is_object() && !text(&part_of["name"]).is_empty() {
            book["series"] = part_of["name"].clone();
            let pos = [&main["position"], &part_of["position"]]
                .into_iter()
                .map(text)
                .find(|x| !x.is_empty())
                .unwrap_or_default();
            book["series_position"] = series_position(&json!(pos));
        }
    }
    if s(&book, "title").is_empty() {
        let mut og = std::collections::HashMap::new();
        for c in rx!(r#"<meta[^>]+property="og:(title|image|description)"[^>]+content="([^"]*)""#)
            .captures_iter(html)
            .filter_map(Result::ok)
        {
            og.insert(c[1].to_string(), c[2].to_string());
        }
        let title = unescape(og.get("title").map_or("", String::as_str));
        book["title"] = json!(title.split(" - ").next().unwrap_or("").trim());
        if book["cover_url"].is_null() {
            book["cover_url"] = opt(&unescape(og.get("image").map_or("", String::as_str)));
        }
        if s(&book, "description").is_empty() {
            book["description"] = json!(unescape(og.get("description").map_or("", String::as_str)));
        }
    }
    if s(&book, "title").is_empty() {
        return Err(LinkError(t("Auf der Seite wurden keine Buchdaten gefunden", &[])));
    }
    // "Titel (Band 3)" / "Serie, Band 3: Titel" – take the number when no series data came along
    if book["series"].is_null()
        && let Some(m) = rx::search(
            rx!(r"(?i)^(.*?)[,:]?\s*(?:band|folge|teil|book)\s+(\d+(?:[.,]\d+)?)\s*[:\-–]\s*(.+)$"),
            s(&book, "title"),
        )
    {
        let (series, pos, title) = (
            m[1].trim_matches([' ', ',', ':']).to_string(),
            m[2].to_string(),
            m[3].trim().to_string(),
        );
        book["series"] = json!(series);
        book["series_position"] = json!(pos);
        book["title"] = json!(title);
    }
    let id = if book["isbn"].is_null() || book["isbn"] == json!("") {
        url.to_string()
    } else {
        text(&book["isbn"])
    };
    book["id"] = json!(format!("{source}:{id}"));
    book["cover_url"] = safe_url(&book["cover_url"]);
    Ok(book)
}

fn page_link(url: &str, source: &str) -> Res<Value> {
    let html = audible::http_get(url, 25, 3, false, 8 << 20, Some(&|u: &str| shop_of(u) == Some(source))).map_err(|e: AudibleError| {
        if e.0.contains("403") || e.0.contains("429") {
            let shop = crate::audible::capitalize(source);
            LinkError(t(
                "{shop} hat den automatischen Abruf blockiert. Daten bitte von Hand eintragen oder einen Audible-/BookBeat-Link verwenden.",
                &[("shop", &shop)],
            ))
        } else {
            LinkError(t("Seite nicht abrufbar: {error}", &[("error", &e.0)]))
        }
    })?;
    parse_product_page(&String::from_utf8_lossy(&html), url, source)
}

/// Book data for a link from one of the known shops. Blocking.
pub fn fetch(url: &str, client: &Client) -> Res<Value> {
    let mut url = url.trim().to_string();
    if url.to_lowercase().starts_with("www.") {
        url = format!("https://{url}");
    }
    let lower = url.to_lowercase();
    let netloc = url
        .split_once("://")
        .map_or("", |x| x.1)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let has_port = netloc.rsplit_once(':').is_some_and(|(_, p)| !p.is_empty());
    if !(lower.starts_with("http://") || lower.starts_with("https://")) || netloc.contains('@') || has_port {
        return Err(LinkError(t("Ungültiger Link", &[])));
    }
    match shop_of(&url) {
        Some("audible") => audible_link(&url, client),
        Some("bookbeat") => bookbeat_link(&url),
        Some("thalia") => page_link(&url, "thalia"),
        _ => Err(LinkError(t(
            "Nur Links von Audible, BookBeat und Thalia werden unterstützt",
            &[],
        ))),
    }
}
