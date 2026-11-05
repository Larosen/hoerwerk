//! Entdecken (discover.py): audiobook suggestions – for your library ("Für dich"), by genre, and from book reviews.
//! Reviews come from Deutschlandfunk (Kultur) feeds, "Die Bücher des Monats" with its best-of list PDFs and
//! Perlentaucher; only author and title are taken and matched against Audible. Reviews are remembered for 180
//! days; reviewed books without an audiobook wait two years and are looked up again once a month.
//! Lists are cached in the database (kv `discover:<key>`) and refreshed in the background, at most once a day.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::audible::{self, AudibleError};
use crate::db::Db;
use crate::htmlent::unescape;
use crate::rx;
use crate::rx::{search, split};
use crate::text::{self, now, t};

pub const MAX_AGE: f64 = 24.0 * 3600.0;
pub const SECTIONS: [&str; 3] = ["foryou", "genre", "critics"];
pub const FEEDS: [(&str, &str, &str); 7] = [
    (
        "Deutschlandfunk Kultur",
        "https://www.deutschlandfunkkultur.de/buchkritik-100.rss",
        "rss",
    ),
    (
        "Deutschlandfunk Kultur",
        "https://www.deutschlandfunkkultur.de/lesart-100.rss",
        "rss",
    ),
    (
        "Deutschlandfunk Kultur",
        "https://www.deutschlandfunkkultur.de/rezensionen-des-monats-114.html",
        "month",
    ),
    (
        "Deutschlandfunk",
        "https://www.deutschlandfunk.de/buechermarkt-100.rss",
        "rss",
    ),
    (
        "Perlentaucher",
        "https://www.perlentaucher.de/buchKSL/hoerbuecher-literatur-aktuell.html",
        "list",
    ),
    (
        "Perlentaucher",
        "https://www.perlentaucher.de/buchKSL/hoerbuecher-sachbuch-aktuell.html",
        "list",
    ),
    ("Perlentaucher", "https://www.perlentaucher.de/index.rss", "rss"),
];
const PT_BASE: &str = "https://www.perlentaucher.de";
const MONTH_LINK: &str = "https://www.deutschlandfunkkultur.de/rezensionen-des-monats-114.html";
pub const PDF_HOST: &str = "bilder.deutschlandfunk.de";
const PDF_PER_RUN: usize = 6;
const REVIEW_DAYS: f64 = 180.0;
const WAIT_DAYS: f64 = 730.0;
const RECHECK_DAYS: f64 = 30.0;
const RECHECK_MAX: usize = 60;
const MAX_REVIEWS: usize = 3000;
const CRITICS_MAX: usize = 40;
const PT_CATEGORIES: [&str; 3] = ["BÜCHERSCHAU", "BÜCHERBRIEF", "TASCHENBÜCHERBRIEF"];
pub const FEED_HOSTS: [&str; 4] = [
    "www.deutschlandfunkkultur.de",
    "www.deutschlandfunk.de",
    "www.perlentaucher.de",
    PDF_HOST,
];
const NONFICTION: [&str; 28] = [
    "Beziehungen, Elternschaft & persönliche Entwicklung",
    "Bildung & Lernen",
    "Biografien & Erinnerungen",
    "Computer & Technologie",
    "Geld & Finanzen",
    "Geschichte",
    "Gesundheit & Wellness",
    "Haushalt & Garten",
    "Politik & Sozialwissenschaften",
    "Reisen & Tourismus",
    "Religion & Spiritualität",
    "Sport & Freizeit",
    "Wirtschaft & Karriere",
    "Wissenschaft & Technik",
    "Relationships, Parenting & Personal Development",
    "Education & Learning",
    "Biographies & Memoirs",
    "Computers & Technology",
    "Money & Finance",
    "History",
    "Health & Wellness",
    "Home & Garden",
    "Politics & Social Sciences",
    "Travel & Tourism",
    "Religion & Spirituality",
    "Sports & Outdoors",
    "Business & Careers",
    "Science & Engineering",
];
const HIDDEN_GENRES: [&str; 2] = ["Erotik", "Erotica"];
const PT_AUTHOR_DAYS: f64 = 60.0;
const PT_LOOKUPS: i64 = 25;
const GENRE_MIN: usize = 30;
const GENRE_MAX: usize = 48;
const LOOSE_RATING: f64 = 4.0;
const LOOSE_COUNT: i64 = 5;
const WARM_PAUSE: Duration = Duration::from_secs(1);
const ITEM_KEYS: [&str; 17] = [
    "asin",
    "title",
    "subtitle",
    "authors",
    "narrators",
    "series",
    "series_position",
    "runtime_min",
    "release_date",
    "cover_url",
    "url",
    "rating",
    "ratings",
    "genres",
    "format_type",
    "language",
    "region",
];

/// Established non-fiction authors (trusted_authors.py, kept current by contract/make_discover.py).
pub static TRUSTED_AUTHORS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("trusted_authors.txt")
        .lines()
        .filter(|l| !l.is_empty())
        .collect()
});

/// Patterns with long counted repeats of Unicode classes (no lookarounds): the plain regex crate, larger limit.
macro_rules! big {
    ($p:expr) => {{
        static R: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::RegexBuilder::new($p)
                .size_limit(1 << 27)
                .dfa_size_limit(1 << 24)
                .build()
                .expect("valid regex")
        });
        &*R
    }};
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(x) => !x.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

// -- parsing ------------------------------------------------------------------------------------------

fn first_author(author: &str) -> String {
    split(rx!(r",|\bund\b|\s&\s|\("), author)
        .into_iter()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Title and first author of a review headline, or None for interviews, essays & co.
pub fn parse_review_title(text: &str) -> Option<Value> {
    let m = search(
        rx!(
            r#"(?i)^\s*(?:buchkritik\s*[-:–]\s*)?(?:(?P<a1>[^"„»“]+?)\s*:\s*)?["„»“](?P<title>[^"“”«]+)["“”«](?:\s*(?:von|v\.)\s*(?P<a2>.+?))?(?:\s+[-–—]\s+.*)?\s*$"#
        ),
        text,
    )?;
    let author = m.name("a2").or_else(|| m.name("a1")).map_or("", |x| x.as_str()).trim();
    let author = first_author(author);
    let title = m.name("title").map_or("", |x| x.as_str()).trim();
    (!title.is_empty() && !author.is_empty()).then(|| json!({"title": title, "author": author}))
}

/// A bold text that looks like a name ("Shida Bazyar") – not "Schuld und Sühne".
fn person(text: &str) -> Option<String> {
    let text = unescape(text);
    let words: Vec<&str> = text.split_whitespace().collect();
    if !(2..=5).contains(&words.len())
        || !words.iter().all(|w| {
            rx::is_match(
                rx!(r"^(?:[A-ZÄÖÜÀ-Þ][\w'’.-]*|von|van|de|der|del|da|di|du|le|la|al|ibn|bin|zu|ten|ter)$"),
                w,
            )
        })
    {
        return None;
    }
    let upper = |w: &str| w.chars().next().is_some_and(char::is_uppercase);
    (upper(words[0]) && upper(words[words.len() - 1])).then(|| words.join(" "))
}

/// Author/title pairs from a Perlentaucher teaser: <b>Author</b> … "Title" in one sentence.
pub fn parse_book_mentions(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for sentence in split(rx!(r"(?<=[.!?])\s+(?=[A-ZÄÖÜ<])"), text) {
        let mut author: Option<String> = None;
        for m in rx!(r#"<b>\s*(?P<name>[^<]{3,60}?)\s*</b>|["„“»](?P<title>[^"„“”«»<]{2,150})["“”«]"#)
            .captures_iter(&sentence)
            .filter_map(Result::ok)
        {
            if let Some(name) = m.name("name") {
                author = person(name.as_str()).or(author);
            } else if let Some(a) = author.take() {
                let title = unescape(m.name("title").map_or("", |x| x.as_str()));
                out.push(json!({"title": title.trim(), "author": a}));
            }
        }
    }
    out
}

/// Books on a Perlentaucher list page: author, title (without subtitle), link and number of reviews.
pub fn parse_pt_list(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for m in big!(
        r#"(?s)<h2>\s*<a href="(?P<href>/buch/[^"]+\.html)">\s*(?P<author>[^<:]{2,80}):\s*<span>(?P<title>[^<]{1,200})</span>(?P<rest>.*?)</article>"#
    )
    .captures_iter(text)
    {
        let title = unescape(&m["title"]);
        let title = split(rx!(r"\.\s+(?=[A-ZÄÖÜ])"), title.trim())
            .into_iter()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let notes = search(rx!(r"<span>\s*(\d{1,3})\s+Notiz"), &m["rest"])
            .and_then(|n| n[1].parse::<i64>().ok())
            .unwrap_or(0);
        out.push(json!({"title": title, "author": unescape(&m["author"]).trim(),
                        "link": format!("{PT_BASE}{}", &m["href"]), "notes": notes}));
    }
    out
}

/// Without the subtitle: "Hi Honey. Nigerias …" / "Endzeitfaschismus - Und der Kampf …" -> the first part.
pub fn main_title(title: &str) -> String {
    let first = split(rx!(r"\.\s+(?=[A-ZÄÖÜ])|\s+[-–—]\s+"), title)
        .into_iter()
        .next()
        .unwrap_or_default();
    let first = first.trim();
    if first.is_empty() {
        title.to_string()
    } else {
        first.to_string()
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_days(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// "02", "11", "2026" -> "Mon, 02 Nov 2026" (a feed date); "" for a day that does not exist.
fn feed_day(d: &str, m: &str, y: &str) -> String {
    let (Ok(d), Ok(m), Ok(y)) = (d.parse::<u32>(), m.parse::<u32>(), y.parse::<i64>()) else {
        return String::new();
    };
    if !(1..=12).contains(&m) || d == 0 || d > month_days(y, m) || y < 1 {
        return String::new();
    }
    let days = text::days_from_civil(y, m, d);
    let wd = (days + 3).rem_euclid(7) as usize; // 1970-01-01 was a Thursday
    format!("{}, {d:02} {} {y:04}", WEEKDAYS[wd], MONTHS[(m - 1) as usize])
}

/// time.strptime(text, "%a, %d %b %Y") as seconds (UTC) – None when it does not parse.
fn parse_feed_day(text: &str) -> Option<f64> {
    let c = search(
        rx!(
            r"(?i)^(?:mon|tue|wed|thu|fri|sat|sun),\s+(3[01]|[12]\d|0[1-9]|[1-9]| [1-9])\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+(\d\d\d\d)$"
        ),
        text,
    )?;
    let d: u32 = c[1].trim().parse().ok()?;
    let m = MONTHS.iter().position(|x| x.eq_ignore_ascii_case(&c[2]))? as u32 + 1;
    let y: i64 = c[3].parse().ok()?;
    if y < 1 || d > month_days(y, m) {
        return None;
    }
    Some(text::days_from_civil(y, m, d) as f64 * 86400.0)
}

/// Books on Deutschlandfunk Kultur's "Die Bücher des Monats": "Author: „Title“ (PDF) (Show, 02.11.2026)".
pub fn parse_dlf_month(text: &str) -> Vec<Value> {
    let raw = unescape(text);
    let page = raw.replace("\\\"", "\"");
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut add = |one: Option<Value>, day: String, out: &mut Vec<Value>| {
        let Some(mut one) = one else { return };
        one["title"] = json!(main_title(s(&one, "title")));
        let key = (s(&one, "author").to_string(), s(&one, "title").to_string());
        if seen.insert(key) {
            one["date"] = json!(day);
            out.push(one);
        }
    };
    for m in big!(
        r">(?P<head>[^<>]{3,300}?) \(PDF\)</a>\s*\((?P<show>[^()]{0,80}?),?\s*(?P<d>\d\d)\.(?P<m>\d\d)\.(?P<y>20\d\d)\)"
    )
    .captures_iter(&page)
    {
        add(
            parse_review_title(&m["head"]),
            feed_day(&m["d"], &m["m"], &m["y"]),
            &mut out,
        );
    }
    for m in big!(r#""teaserHeadline":"((?:[^"\\]|\\.){3,300})""#).captures_iter(&raw) {
        add(parse_review_title(&m[1].replace("\\\"", "\"")), String::new(), &mut out);
    }
    out
}

/// The best-of lists on "Die Bücher des Monats": [{name, url, date}].
pub fn dlf_best_lists(text: &str) -> Vec<Value> {
    let page = unescape(text).replace("\\\"", "\"");
    let mut out: Vec<Value> = Vec::new();
    for m in big!(
        r#"(?i)href="(?P<url>https://bilder\.deutschlandfunk\.de/[\w./-]{1,300}\.pdf)"[^<>]*>(?P<name>[^<>]{3,200}?bestenliste[^<>]{0,200}?) \(PDF\)</a>\s*\([^()]{0,80}?(?P<d>\d\d)\.(?P<m>\d\d)\.(?P<y>20\d\d)\)"#
    )
    .captures_iter(&page)
    {
        let name = m["name"].trim();
        let name = match rx!(r"\s+[-–:]\s+|:\s+").find(name).ok().flatten() {
            Some(sep) => &name[sep.end()..],
            None => name,
        };
        if !out.iter().any(|o| s(o, "url") == &m["url"]) {
            out.push(json!({"name": name.trim(), "url": &m["url"], "date": feed_day(&m["d"], &m["m"], &m["y"])}));
        }
    }
    out
}

/// The books of one best-of list PDF as reviews (first author, title without subtitle, place on the list).
pub fn best_list_reviews(raw: &[u8], best: &Value) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for e in crate::pdftext::best_list(raw)? {
        let author = first_author(s(&e, "author"));
        if !author.is_empty() && !s(&e, "title").is_empty() {
            out.push(
                json!({"title": main_title(s(&e, "title")), "author": author, "date": best["date"],
                            "link": best["url"], "list": best["name"], "rank": e["rank"], "month": true}),
            );
        }
    }
    Ok(out)
}

/// When a review appeared: the feed date, else when it was first seen; a late audiobook counts from then.
fn review_time(r: &Value) -> f64 {
    let date: String = s(r, "date").chars().take(16).collect();
    let when = parse_feed_day(&date).unwrap_or_else(|| r.get("seen").map_or(0.0, num));
    when.max(num(&r["found"]))
}

/// Newest first within each source, the sources taking turns.
pub fn mix_sources(reviews: Vec<Value>) -> Vec<Value> {
    let mut sorted: Vec<(f64, Value)> = reviews.into_iter().map(|r| (-review_time(&r), r)).collect();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut queues: Vec<(String, std::collections::VecDeque<Value>)> = Vec::new();
    for (_, r) in sorted {
        let src = s(&r, "source").to_string();
        match queues.iter_mut().find(|q| q.0 == src) {
            Some(q) => q.1.push_back(r),
            None => queues.push((src, [r].into())),
        }
    }
    let mut out = Vec::new();
    while !queues.is_empty() {
        for q in &mut queues {
            if let Some(r) = q.1.pop_front() {
                out.push(r);
            }
        }
        queues.retain(|q| !q.1.is_empty());
    }
    out
}

/// Lower case without accents: "Faulerová" -> "faulerova".
pub fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|c| canonical_combining_class(*c) == 0)
        .collect::<String>()
        .to_lowercase()
}

/// Surname of a reviewed author, also without the German genitive s ("Kapralovas" -> "kapralova").
pub fn surnames(author: &str) -> Vec<String> {
    let last = fold(author.split_whitespace().last().unwrap_or(""));
    let last = last.trim_matches(['.', ',']).to_string();
    if last.chars().count() > 3 && last.ends_with('s') {
        let cut = last[..last.len() - 1].to_string();
        vec![last, cut]
    } else {
        vec![last]
    }
}

/// "Rüdiger Maas" -> "ruediger-maas" (Perlentaucher's author addresses).
pub fn pt_slug(name: &str) -> String {
    let mut x = name.to_lowercase();
    for (a, b) in [("ä", "ae"), ("ö", "oe"), ("ü", "ue"), ("ß", "ss")] {
        x = x.replace(a, b);
    }
    let x = rx::sub(rx!(r"['’`]"), &fold(&x), "");
    rx::sub(rx!(r"[^a-z0-9]+"), &x, "-").trim_matches('-').to_string()
}

/// An author page lists "3 Bücher" / "1 Buch"; an unknown author gets the general index instead.
pub fn pt_has_books(page: &str) -> bool {
    !page.contains("Buchautor-Übersicht") && rx::is_match(rx!(r"\b\d+\s+B(?:ü|ue)ch(?:er)?\b"), page)
}

fn today() -> String {
    let tm = text::local(now());
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/// The release date of an audiobook that is announced but not out yet ("2026-12-04") – else None.
fn announced(book: &Value) -> Value {
    let raw = match &book["release_date"] {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        other => other.to_string(),
    };
    let day: String = raw.chars().take(10).collect();
    if rx::is_match(rx!(r"^\d{4}-\d\d-\d\d$"), &day) && day > today() {
        json!(day)
    } else {
        Value::Null
    }
}

pub fn is_nonfiction(book: &Value) -> bool {
    strs(&book["genres"]).iter().any(|g| NONFICTION.contains(&g.as_str()))
}

fn item(book: &Value, extra: Value) -> Value {
    let mut out: Map<String, Value> = ITEM_KEYS
        .iter()
        .map(|k| (k.to_string(), book.get(*k).cloned().unwrap_or(Value::Null)))
        .collect();
    if let Value::Object(e) = extra {
        out.extend(e);
    }
    Value::Object(out)
}

fn xml_child_text(node: roxmltree::Node, name: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace().is_none())
        .map(|c| match c.first_child() {
            Some(t) if t.is_text() => t.text().unwrap_or("").to_string(),
            _ => String::new(),
        })
}

/// {title, author, link, date} of the books a feed reviews or recommends (rss and list kinds).
pub fn reviews_from_feed(source: &str, raw: &[u8], kind: &str) -> Result<Vec<Value>, String> {
    let with = |r: &Value, extra: Value| {
        let mut o = r.as_object().cloned().unwrap_or_default();
        if let Value::Object(e) = extra {
            o.extend(e);
        }
        Value::Object(o)
    };
    if kind == "list" {
        return Ok(parse_pt_list(&String::from_utf8_lossy(raw))
            .iter()
            .map(|r| with(r, json!({"source": source, "date": ""})))
            .collect());
    }
    if regex::bytes::Regex::new(r"(?i-u)<!DOCTYPE|<!ENTITY").is_ok_and(|re| re.is_match(raw)) {
        return Err(t("Antwort enthält eine DTD – abgelehnt", &[]));
    }
    let text = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
    let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for item in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "item" && n.tag_name().namespace().is_none())
    {
        let link = xml_child_text(item, "link").unwrap_or_default();
        let link = link.trim();
        let date: String = xml_child_text(item, "pubDate")
            .unwrap_or_default()
            .chars()
            .take(16)
            .collect();
        let meta =
            json!({"source": source, "link": if link.starts_with("https://") { link } else { "" }, "date": date});
        let found = if source == "Perlentaucher" {
            let cat = xml_child_text(item, "category")
                .unwrap_or_default()
                .trim()
                .to_uppercase();
            if !PT_CATEGORIES.iter().any(|c| cat.starts_with(c)) {
                continue;
            }
            parse_book_mentions(&xml_child_text(item, "description").unwrap_or_default())
        } else {
            parse_review_title(&xml_child_text(item, "title").unwrap_or_default())
                .into_iter()
                .collect()
        };
        out.extend(found.iter().map(|r| with(r, meta.clone())));
    }
    Ok(out)
}

// -- the service ----------------------------------------------------------------------------------------

/// What Entdecken needs from the rest of the app (the store, the feeds, the library) – a fake in the tests.
pub trait Env: Send + Sync + 'static {
    fn db(&self) -> &Db;
    fn discover(&self) -> &Discover;
    fn settings(&self) -> Map<String, Value>;
    fn similar(&self, asin: &str, region: &str, limit: usize) -> Result<Vec<Value>, AudibleError>;
    fn genre_products(
        &self,
        region: &str,
        genre: &str,
        sort: &str,
        limit: usize,
        page: usize,
    ) -> Result<Vec<Value>, AudibleError>;
    fn genres(&self, region: &str) -> Result<Vec<Value>, AudibleError>;
    fn search(&self, region: &str, title: &str, author: &str, limit: usize) -> Result<Vec<Value>, AudibleError>;
    /// Only the fixed feed addresses, HTTPS, max. 8 MB.
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
    /// ASINs being downloaded or processed right now.
    fn pending(&self) -> HashSet<String>;
    fn owned(&self, book: &Value) -> bool;
    /// The newest library books with an ASIN (the seeds of the general "Für dich").
    fn library_seeds(&self) -> Vec<Value>;
    /// What this account heard (Audiobookshelf / Plex).
    fn seeds(&self, uid: i64) -> Vec<Value>;
}

struct State {
    pt_budget: i64,
    running: HashSet<String>,
    known: HashSet<String>,
    warming: bool,
    warmed: f64,
}

pub struct Discover {
    state: Mutex<State>,
}

impl Discover {
    /// prepare=false (tests): genre lists are not prepared in the background.
    pub fn new(prepare: bool) -> Self {
        Discover {
            state: Mutex::new(State {
                pt_budget: PT_LOOKUPS,
                running: HashSet::new(),
                known: HashSet::new(),
                warming: !prepare,
                warmed: 0.0,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The single big cache row of 5.x–7.2 goes.
pub fn cleanup(db: &Db) {
    let _ = db.lock().execute("DELETE FROM kv WHERE key='discover'", []);
}

fn key_of(section: &str, genre: &str) -> String {
    if section == "genre" {
        format!("{section}:{genre}")
    } else {
        section.to_string()
    }
}

fn entry(env: &dyn Env, key: &str) -> Map<String, Value> {
    env.db()
        .get(&format!("discover:{key}"))
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn region(env: &dyn Env) -> String {
    env.settings()
        .get("ab_region")
        .and_then(Value::as_str)
        .unwrap_or("de")
        .to_string()
}

fn busy(env: &dyn Env) -> HashSet<String> {
    env.pending()
        .into_iter()
        .filter(|a| !a.is_empty())
        .map(|a| a.to_uppercase())
        .collect()
}

fn personal_seeds(env: &dyn Env, uid: Option<i64>) -> Vec<Value> {
    match uid {
        Some(u) if u != 0 => env.seeds(u),
        _ => Vec::new(),
    }
}

/// The cached list; refreshed in the background when it is missing or older than a day.
pub fn get(env: &Arc<dyn Env>, section: &str, genre: &str, uid: Option<i64>) -> Value {
    let seeds = if section == "foryou" {
        personal_seeds(&**env, uid)
    } else {
        Vec::new()
    };
    let key = if seeds.is_empty() {
        key_of(section, genre)
    } else {
        format!("foryou:u{}", uid.unwrap_or(0))
    };
    let e = entry(&**env, &key);
    let personal = !seeds.is_empty();
    if e.is_empty() || now() - e.get("time").map_or(0.0, num) > MAX_AGE {
        refresh_with(env, section, genre, false, Some(seeds), &key);
    }
    let running = env.discover().state().running.contains(&key);
    let busy = busy(&**env);
    let items: Vec<Value> = e
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| !busy.contains(s(i, "asin")) && !env.owned(i))
        .cloned()
        .collect();
    json!({"items": items, "updated": e.get("time").cloned().unwrap_or(Value::Null),
           "error": e.get("error").cloned().unwrap_or(Value::Null), "running": running, "personal": personal})
}

/// Refresh a list (personal "Für dich" when this account's listening history is known).
pub fn refresh(env: &Arc<dyn Env>, section: &str, genre: &str, wait: bool, uid: Option<i64>) {
    let (seeds, key) = if section == "foryou" && uid.is_some_and(|u| u != 0) {
        let seeds = personal_seeds(&**env, uid);
        let key = if seeds.is_empty() {
            String::new()
        } else {
            format!("foryou:u{}", uid.unwrap_or(0))
        };
        (Some(seeds), key)
    } else {
        (None, String::new())
    };
    refresh_with(env, section, genre, wait, seeds, &key);
}

fn refresh_with(env: &Arc<dyn Env>, section: &str, genre: &str, wait: bool, seeds: Option<Vec<Value>>, key: &str) {
    if !SECTIONS.contains(&section) || (section == "genre" && (genre.is_empty() || !known_genre(env, genre))) {
        return;
    }
    let key = if key.is_empty() {
        key_of(section, genre)
    } else {
        key.to_string()
    };
    {
        let mut st = env.discover().state();
        if !st.running.insert(key.clone()) {
            return;
        }
    }
    let (env2, section, genre, lang) = (env.clone(), section.to_string(), genre.to_string(), text::language());
    let run = move || {
        text::set_language(&lang);
        env2.discover().state().pt_budget = PT_LOOKUPS;
        let res = match section.as_str() {
            "foryou" => Ok(for_you(&*env2, seeds.unwrap_or_default())),
            "critics" => critics(&*env2),
            _ => genre_list(&*env2, &genre).map_err(|e| e.0),
        };
        let entry = match res {
            Ok(items) => json!({"time": now(), "items": items, "error": null}),
            Err(e) => {
                let source = if section == "critics" {
                    let mut names: Vec<&str> = Vec::new();
                    for (n, _, _) in FEEDS {
                        if !names.contains(&n) {
                            names.push(n);
                        }
                    }
                    names.join(" / ")
                } else {
                    "Audible".into()
                };
                text::warn(
                    "Entdecken",
                    &t(
                        "Entdecken: {source} nicht erreichbar: {error}",
                        &[("source", &source), ("error", &e)],
                    ),
                );
                let mut old = entry(&*env2, &key);
                old.insert("time".into(), json!(now()));
                old.insert("error".into(), json!(e));
                Value::Object(old)
            }
        };
        let _ = env2.db().put(&format!("discover:{key}"), &entry);
        env2.discover().state().running.remove(&key);
    };
    if wait {
        run();
    } else {
        let _ = std::thread::Builder::new().name("discover".into()).spawn(run);
    }
}

// -- filters ------------------------------------------------------------------------------------------

/// Quality rules; reviewed books (from_store=false) skip the rating rule – they are often new.
fn good(env: &dyn Env, book: &Value, from_store: bool, chosen_genre: bool, loose: bool) -> bool {
    let st = env.settings();
    let flag = |k: &str, default: bool| st.get(k).map_or(default, truthy);
    let genres = strs(&book["genres"]);
    if truthy(&book["is_adult"]) || genres.iter().any(|g| HIDDEN_GENRES.contains(&g.as_str())) {
        return false;
    }
    if flag("discover_unabridged", true) && s(book, "format_type").to_lowercase() == "abridged" {
        return false;
    }
    if flag("discover_german", true) && !matches!(s(book, "language").to_lowercase().as_str(), "german" | "deutsch") {
        return false;
    }
    if from_store {
        if is_nonfiction(book)
            && !chosen_genre
            && !flag("discover_nonfiction_store", false)
            && trusted_author(env, book).is_none()
        {
            return false;
        }
        let mut min_rating = st.get("discover_min_rating").map_or(0.0, num);
        let mut min_count = st.get("discover_min_count").and_then(Value::as_f64).unwrap_or(0.0) as i64;
        if loose {
            min_rating = min_rating.min(LOOSE_RATING);
            min_count = min_count.min(LOOSE_COUNT);
        }
        if book.get("rating").map_or(0.0, num) < min_rating {
            return false;
        }
        if book.get("ratings").map_or(0.0, num) < min_count as f64 {
            return false;
        }
    }
    !busy(env).contains(&s(book, "asin").to_uppercase()) && !env.owned(book)
}

fn named_trusted(env: &dyn Env) -> HashSet<String> {
    let st = env.settings();
    let own_raw = match st.get("discover_trusted_authors") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(x)) => x.clone(),
        Some(other) if !truthy(other) => String::new(),
        Some(other) => other.to_string(),
    };
    let own = split(rx!(r"[,;\n]+"), &own_raw);
    let reviewed: Vec<String> = env
        .db()
        .get("discover_reviews")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
        .values()
        .filter(|v| v.is_object())
        .map(|v| s(v, "author").to_string())
        .collect();
    TRUSTED_AUTHORS
        .iter()
        .map(|x| x.to_string())
        .chain(own)
        .chain(reviewed)
        .filter(|n| !n.trim().is_empty())
        .map(|n| fold(&n).trim().to_string())
        .collect()
}

/// Why this book's author is an established one ("list" or "perlentaucher") – or None.
pub fn trusted_author(env: &dyn Env, book: &Value) -> Option<&'static str> {
    let authors: Vec<String> = strs(&book["authors"]).into_iter().filter(|a| !a.is_empty()).collect();
    let named = named_trusted(env);
    if authors.iter().any(|a| named.contains(fold(a).trim())) {
        return Some("list");
    }
    authors
        .iter()
        .take(2)
        .any(|a| perlentaucher_author(env, a))
        .then_some("perlentaucher")
}

/// Has Perlentaucher an author page with reviewed books? Remembered for 60 days; few new lookups per run.
fn perlentaucher_author(env: &dyn Env, name: &str) -> bool {
    let slug = pt_slug(name);
    if slug.is_empty() {
        return false;
    }
    let mut known = env
        .db()
        .get("pt_authors")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    if let Some(hit) = known.get(&slug).filter(|h| truthy(h))
        && now() - num(&hit[1]) < PT_AUTHOR_DAYS * 86400.0
    {
        return truthy(&hit[0]);
    }
    {
        let mut st = env.discover().state();
        if st.pt_budget <= 0 {
            return false;
        }
        st.pt_budget -= 1;
    }
    let Ok(page) = env.fetch(&format!("https://www.perlentaucher.de/autor/{slug}.html")) else {
        return false;
    };
    let ok = pt_has_books(&String::from_utf8_lossy(&page));
    known.insert(slug, json!([ok, now()]));
    if known.len() > 5000 {
        let mut all: Vec<(String, Value)> = known.into_iter().collect();
        all.sort_by(|a, b| {
            num(&b.1[1])
                .partial_cmp(&num(&a.1[1]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all.truncate(4000);
        known = all.into_iter().collect();
    }
    let _ = env.db().put("pt_authors", &Value::Object(known));
    ok
}

// -- sources ------------------------------------------------------------------------------------------

/// Audible's similar titles, counted across the seeds: the books this account heard, or the library's newest.
fn for_you(env: &dyn Env, seeds: Vec<Value>) -> Vec<Value> {
    let seeds = if seeds.is_empty() { env.library_seeds() } else { seeds };
    if seeds.is_empty() {
        return Vec::new();
    }
    let region = region(env);
    let mut found: Vec<Value> = Vec::new();
    let mut because: HashMap<String, Vec<String>> = HashMap::new();
    for seed in &seeds {
        let Ok(sims) = env.similar(s(seed, "asin"), &region, 15) else {
            continue;
        };
        for b in sims {
            let asin = s(&b, "asin").to_string();
            if !because.contains_key(&asin) {
                because.insert(asin.clone(), Vec::new());
                found.push(b);
            }
            let list = because.get_mut(&asin).expect("just added");
            let title = s(seed, "title").to_string();
            if !list.contains(&title) {
                list.push(title);
            }
        }
    }
    let mut good_ones: Vec<Value> = found.into_iter().filter(|b| good(env, b, true, false, false)).collect();
    let n = |b: &Value| because[s(b, "asin")].len();
    good_ones.sort_by(|a, b| {
        n(b).cmp(&n(a))
            .then(
                num(&b["rating"])
                    .partial_cmp(&num(&a["rating"]))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(
                num(&b["ratings"])
                    .partial_cmp(&num(&a["ratings"]))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    good_ones
        .iter()
        .take(48)
        .map(|b| {
            let why: Vec<&String> = because[s(b, "asin")].iter().take(3).collect();
            item(b, json!({"because": why, "source": "audible"}))
        })
        .collect()
}

/// Best sellers, best rated and new releases of a genre; small genres are filled up with books rated just as
/// well by fewer listeners.
fn genre_list(env: &dyn Env, genre: &str) -> Result<Vec<Value>, AudibleError> {
    let region = region(env);
    let (mut strict, mut loose): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    let fetches = [
        ("BestSellers", 1),
        ("AvgRating", 1),
        ("-ReleaseDate", 1),
        ("BestSellers", 2),
        ("AvgRating", 2),
    ];
    for (n, (sort, page)) in fetches.iter().enumerate() {
        if n >= 3 && strict.len() >= GENRE_MIN {
            break;
        }
        for b in env.genre_products(&region, genre, sort, 50, *page)? {
            let asin = s(&b, "asin");
            if strict.iter().chain(loose.iter()).any(|x| s(x, "asin") == asin) {
                continue;
            }
            if good(env, &b, true, true, false) {
                strict.push(item(&b, json!({"source": "audible", "list": sort})));
            } else if good(env, &b, true, true, true) {
                loose.push(item(&b, json!({"source": "audible", "list": sort, "loose": true})));
            }
        }
    }
    let mut out: Vec<Value> = strict.into_iter().take(GENRE_MAX).collect();
    if out.len() < GENRE_MIN {
        loose.sort_by(|a, b| {
            num(&b["rating"])
                .partial_cmp(&num(&a["rating"]))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    num(&b["ratings"])
                        .partial_cmp(&num(&a["ratings"]))
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });
        let room = GENRE_MIN - out.len();
        out.extend(loose.into_iter().take(room));
    }
    Ok(out)
}

/// The real feed download: only the fixed hosts, HTTPS, max. 8 MB (the monthly page is ~3 MB).
pub fn fetch_feed(url: &str) -> Result<Vec<u8>, String> {
    let allow = |u: &str| {
        u.starts_with("https://") && {
            let host = u["https://".len()..].split(['/', '?', '#']).next().unwrap_or("");
            let host = host.rsplit_once('@').map_or(host, |x| x.1);
            let host = host.split(':').next().unwrap_or("").to_lowercase();
            FEED_HOSTS.contains(&host.as_str())
        }
    };
    audible::http_get(url, 20, 3, false, 8 << 20, Some(&allow)).map_err(|e| e.0)
}

fn reviews_from(env: &dyn Env, source: &str, raw: &[u8], kind: &str) -> Result<Vec<Value>, String> {
    if kind != "month" {
        return reviews_from_feed(source, raw, kind);
    }
    let text = String::from_utf8_lossy(raw);
    let mut out: Vec<Value> = parse_dlf_month(&text)
        .into_iter()
        .map(|mut r| {
            r["source"] = json!(source);
            r["link"] = json!(MONTH_LINK);
            r["month"] = json!(true);
            r
        })
        .collect();
    out.extend(best_lists(env, &text).into_iter().map(|mut r| {
        r["source"] = json!(source);
        r
    }));
    Ok(out)
}

/// Books of the best-of list PDFs not read yet (each PDF once; a failed one is tried again the next day).
fn best_lists(env: &dyn Env, page: &str) -> Vec<Value> {
    let mut done = env
        .db()
        .get("discover_lists")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut out = Vec::new();
    let todo: Vec<Value> = dlf_best_lists(page)
        .into_iter()
        .filter(|b| !done.contains_key(s(b, "url")))
        .take(PDF_PER_RUN)
        .collect();
    for best in todo {
        match env
            .fetch(s(&best, "url"))
            .and_then(|raw| best_list_reviews(&raw, &best))
        {
            Ok(found) => out.extend(found),
            Err(e) => {
                text::warn(
                    "Entdecken",
                    &t(
                        "Entdecken: {source} nicht erreichbar: {error}",
                        &[("source", s(&best, "name")), ("error", &e)],
                    ),
                );
                continue;
            }
        }
        done.insert(s(&best, "url").to_string(), json!(now()));
    }
    if done.len() > 500 {
        let mut all: Vec<(String, Value)> = done.into_iter().collect();
        all.sort_by(|a, b| num(&b.1).partial_cmp(&num(&a.1)).unwrap_or(std::cmp::Ordering::Equal));
        all.truncate(400);
        done = all.into_iter().collect();
    }
    let _ = env.db().put("discover_lists", &Value::Object(done));
    out
}

/// Reviewed books that exist as an audiobook (and are not in the library yet). Every review seen is remembered
/// with its Audible match, so the list grows beyond the few newest feed items.
fn critics(env: &dyn Env) -> Result<Vec<Value>, String> {
    let region = region(env);
    let mut memory = env
        .db()
        .get("discover_reviews")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let (now, mut errors, mut fetched) = (now(), Vec::<String>::new(), 0);
    for (source, url, kind) in FEEDS {
        let reviews = match env.fetch(url).and_then(|raw| reviews_from(env, source, &raw, kind)) {
            Ok(r) => {
                fetched += 1;
                r
            }
            Err(e) => {
                errors.push(format!("{source}: {e}"));
                text::warn(
                    "Entdecken",
                    &t(
                        "Entdecken: {source} nicht erreichbar: {error}",
                        &[("source", source), ("error", &e)],
                    ),
                );
                continue;
            }
        };
        for r in reviews {
            let key = format!("{region}|{}|{}", fold(s(&r, "author")), fold(s(&r, "title")));
            match memory.get_mut(&key).and_then(Value::as_object_mut) {
                None => {
                    let mut v = r.as_object().cloned().unwrap_or_default();
                    v.insert("seen".into(), json!(now));
                    v.insert("checked".into(), json!(now));
                    v.insert("book".into(), audiobook_for(env, &r, &region));
                    memory.insert(key, Value::Object(v));
                }
                Some(old) => {
                    for k in ["notes", "month", "list", "rank"] {
                        if r.get(k).is_some_and(truthy) {
                            old.insert(k.into(), r[k].clone());
                        }
                    }
                    let seen = old.get("seen").map_or(0.0, num).max(now - 1.0);
                    old.insert("seen".into(), json!(seen));
                }
            }
        }
    }
    if fetched == 0 && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    recheck_waiting(env, &mut memory, &region, now);
    let keep = |v: &Value| {
        let days = if truthy(&v["book"]) { REVIEW_DAYS } else { WAIT_DAYS };
        now - v.get("seen").map_or(0.0, num) < days * 86400.0 || now - num(&v["found"]) < REVIEW_DAYS * 86400.0
    };
    let mut fresh: Vec<(String, Value)> = memory.into_iter().filter(|(_, v)| keep(v)).collect();
    let newest = |v: &Value| v.get("seen").map_or(0.0, num).max(num(&v["found"]));
    fresh.sort_by(|a, b| {
        newest(&b.1)
            .partial_cmp(&newest(&a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fresh.truncate(MAX_REVIEWS);
    let values: Vec<Value> = fresh.iter().map(|x| x.1.clone()).collect();
    let _ = env
        .db()
        .put("discover_reviews", &Value::Object(fresh.into_iter().collect()));
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for r in mix_sources(values) {
        let book = &r["book"];
        if !truthy(book) || seen.contains(s(book, "asin")) || !good(env, book, false, false, false) {
            continue;
        }
        seen.insert(s(book, "asin").to_string());
        let review: Map<String, Value> = ["source", "link", "date", "notes", "month", "list", "rank"]
            .iter()
            .map(|k| (k.to_string(), r.get(*k).cloned().unwrap_or(Value::Null)))
            .collect();
        out.push(item(
            book,
            json!({"source": "review", "review": review, "later": truthy(&r["found"]),
                                   "announced": announced(book)}),
        ));
        if out.len() >= CRITICS_MAX {
            break;
        }
    }
    Ok(out)
}

/// Reviewed books without an audiobook: look again once a month whether one came out or is announced.
fn recheck_waiting(env: &dyn Env, memory: &mut Map<String, Value>, region: &str, now: f64) {
    let last = |v: &Value| match v.get("checked").filter(|c| truthy(c)) {
        Some(c) => num(c),
        None => v.get("seen").map_or(0.0, num),
    };
    let mut due: Vec<(String, f64)> = memory
        .iter()
        .filter(|(_, v)| {
            !truthy(&v["book"])
                && !s(v, "author").is_empty()
                && !s(v, "title").is_empty()
                && now - last(v) >= RECHECK_DAYS * 86400.0
        })
        .map(|(k, v)| (k.clone(), last(v)))
        .collect();
    due.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut found = 0;
    for (k, _) in due.into_iter().take(RECHECK_MAX) {
        let review = memory[&k].clone();
        let book = audiobook_for(env, &review, region);
        let v = memory.get_mut(&k).expect("listed");
        v["checked"] = json!(now);
        if truthy(&book) {
            v["book"] = book;
            v["found"] = json!(now);
            found += 1;
        }
    }
    if found > 0 {
        text::info(
            "Entdecken",
            &if found > 1 {
                t(
                    "Entdecken: {n} besprochene Bücher gibt es jetzt als Hörbuch",
                    &[("n", &found.to_string())],
                )
            } else {
                t("Entdecken: 1 besprochenes Buch gibt es jetzt als Hörbuch", &[])
            },
        );
    }
}

/// How many reviewed books are still waiting for their audiobook.
pub fn waiting(db: &Db) -> usize {
    db.get("discover_reviews")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
        .values()
        .filter(|v| v.is_object() && !truthy(&v["book"]))
        .count()
}

/// Called every few minutes: the review list (and the monthly look for late audiobooks) stays current.
pub fn tick(env: &Arc<dyn Env>) {
    if now() - entry(&**env, "critics").get("time").map_or(0.0, num) > MAX_AGE {
        refresh(env, "critics", "", false, None);
    }
}

/// The Audible audiobook of a reviewed book – same title (roughly) and same author.
fn audiobook_for(env: &dyn Env, review: &Value, region: &str) -> Value {
    let author = s(review, "author");
    let names = surnames(author);
    let first = author.split_whitespace().next().unwrap_or("");
    let mut hits = Vec::new();
    for a in [
        author.to_string(),
        format!("{first} {}", names[names.len() - 1]),
        String::new(),
    ] {
        match env.search(region, s(review, "title"), a.trim(), 5) {
            Ok(h) => hits = h,
            Err(_) => return Value::Null,
        }
        if !hits.is_empty() {
            break;
        }
    }
    for b in hits {
        let authors: Vec<String> = strs(&b["authors"]).iter().map(|a| fold(a)).collect();
        if !names
            .iter()
            .any(|n| !n.is_empty() && authors.iter().any(|a| a.contains(n.as_str())))
        {
            continue;
        }
        if audible::ratio(s(&b, "title"), s(review, "title")) >= 0.6 {
            let mut keys: Vec<&str> = ITEM_KEYS.to_vec();
            keys.extend(["is_adult", "format_type", "language", "genres"]);
            let out: Map<String, Value> = keys
                .iter()
                .map(|k| (k.to_string(), b.get(*k).cloned().unwrap_or(Value::Null)))
                .collect();
            return Value::Object(out);
        }
    }
    Value::Null
}

fn genre_entry_empty(env: &dyn Env, gid: &str) -> bool {
    let e = entry(env, &key_of("genre", gid));
    !e.is_empty() && !e.get("items").is_some_and(truthy) && !e.get("error").is_some_and(truthy)
}

/// All store genres with their subcategories, like in the Audible shop (no adult ones). Non-fiction genres are
/// marked; subcategories known to have no German audiobook are left out. Starts preparing all lists.
pub fn genres(env: &Arc<dyn Env>) -> Vec<Value> {
    let Ok(gs) = env.genres(&region(&**env)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for g in gs {
        let name = s(&g, "name");
        if HIDDEN_GENRES.contains(&name) {
            continue;
        }
        let kids: Vec<Value> = g["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| !HIDDEN_GENRES.contains(&s(c, "name")) && !genre_entry_empty(&**env, s(c, "id")))
            .map(|c| json!({"id": c["id"], "name": c["name"]}))
            .collect();
        out.push(json!({"id": g["id"], "name": g["name"], "nonfiction": NONFICTION.contains(&name), "children": kids}));
    }
    let known: HashSet<String> = out
        .iter()
        .flat_map(|g| {
            std::iter::once(s(g, "id").to_string()).chain(
                g["children"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| s(c, "id").to_string()),
            )
        })
        .collect();
    env.discover().state().known = known;
    warm(env, out.clone());
    out
}

/// Only store genres are looked up and cached (not any number someone sends).
pub fn known_genre(env: &Arc<dyn Env>, genre: &str) -> bool {
    if env.discover().state().known.is_empty() {
        genres(env);
    }
    let st = env.discover().state();
    if st.known.is_empty() {
        !genre.is_empty() && genre.chars().all(|c| c.is_ascii_digit())
    } else {
        st.known.contains(genre)
    }
}

/// Prepare every genre list in the background (one after the other, gently), at most once a day.
fn warm(env: &Arc<dyn Env>, tree: Vec<Value>) {
    {
        let mut st = env.discover().state();
        if st.warming || now() - st.warmed < MAX_AGE {
            return;
        }
        st.warming = true;
    }
    let env = env.clone();
    let lang = text::language();
    let _ = std::thread::Builder::new().name("discover-warm".into()).spawn(move || {
        text::set_language(&lang);
        for g in &tree {
            let ids = std::iter::once(s(g, "id").to_string()).chain(
                g["children"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| s(c, "id").to_string()),
            );
            for gid in ids {
                let e = entry(&*env, &key_of("genre", &gid));
                if !e.is_empty() && now() - e.get("time").map_or(0.0, num) < MAX_AGE {
                    continue;
                }
                refresh_with(&env, "genre", &gid, true, None, "");
                std::thread::sleep(WARM_PAUSE);
            }
        }
        let mut st = env.discover().state();
        st.warming = false;
        st.warmed = now();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn golden_parsers() {
        let g: Value =
            serde_json::from_str(include_str!("../../../contract/golden/discover.json")).expect("discover.json");
        for c in g["review_titles"].as_array().unwrap() {
            assert_eq!(
                parse_review_title(c[0].as_str().unwrap()).unwrap_or(Value::Null),
                c[1],
                "{}",
                c[0]
            );
        }
        for c in g["mentions"].as_array().unwrap() {
            assert_eq!(json!(parse_book_mentions(c[0].as_str().unwrap())), c[1], "{}", c[0]);
        }
        assert_eq!(json!(parse_pt_list(g["pt_list"][0].as_str().unwrap())), g["pt_list"][1]);
        assert_eq!(
            json!(parse_dlf_month(g["dlf_month"][0].as_str().unwrap())),
            g["dlf_month"][1]
        );
        assert_eq!(
            json!(dlf_best_lists(g["dlf_lists"][0].as_str().unwrap())),
            g["dlf_lists"][1]
        );
        for (name, c) in g["pdfs"].as_object().unwrap() {
            let raw = base64::engine::general_purpose::STANDARD
                .decode(c[0].as_str().unwrap())
                .unwrap();
            let got = match crate::pdftext::best_list(&raw) {
                Ok(v) => json!(v),
                Err(e) => json!({"error": e}),
            };
            assert_eq!(got, c[1], "{name}");
        }
        let list = base64::engine::general_purpose::STANDARD
            .decode(g["pdfs"]["list"][0].as_str().unwrap())
            .unwrap();
        assert_eq!(
            json!(best_list_reviews(&list, &g["best_entry"]).unwrap()),
            g["best_list_reviews"]
        );
        for c in g["rss"].as_array().unwrap() {
            let got = reviews_from_feed(
                c[0].as_str().unwrap(),
                c[2].as_str().unwrap().as_bytes(),
                c[1].as_str().unwrap(),
            );
            assert_eq!(json!(got.unwrap()), c[3], "{}", c[0]);
        }
        for c in g["slugs"].as_array().unwrap() {
            assert_eq!(pt_slug(c[0].as_str().unwrap()), c[1].as_str().unwrap());
        }
        for c in g["has_books"].as_array().unwrap() {
            assert_eq!(
                pt_has_books(c[0].as_str().unwrap()),
                c[1].as_bool().unwrap(),
                "{}",
                c[0]
            );
        }
        for c in g["surnames"].as_array().unwrap() {
            assert_eq!(json!(surnames(c[0].as_str().unwrap())), c[1]);
        }
        for c in g["main_titles"].as_array().unwrap() {
            assert_eq!(main_title(c[0].as_str().unwrap()), c[1].as_str().unwrap());
        }
        let mixed: Vec<Value> = mix_sources(g["mix"][0].as_array().unwrap().clone())
            .iter()
            .map(|r| r["n"].clone())
            .collect();
        assert_eq!(json!(mixed), g["mix"][1]);
        assert_eq!(json!(*TRUSTED_AUTHORS), g["trusted_authors"]);
    }

    // -- the service, with the shop, the feeds and the library faked (tests/test_discover.py) ------------

    #[derive(Default)]
    struct Fake {
        db: Option<Db>,
        discover: Option<Discover>,
        settings: Mutex<Map<String, Value>>,
        sims: Mutex<HashMap<String, Vec<Value>>>,
        lists: Mutex<HashMap<String, Vec<Value>>>,
        hits: Mutex<Vec<Value>>,
        asked: Mutex<Vec<(String, String, usize)>>,
        feeds: Mutex<HashMap<&'static str, Option<Vec<u8>>>>,
        pdfs: Mutex<HashMap<String, Vec<u8>>>,
        pdf_asked: Mutex<Vec<String>>,
        pt_authors: Mutex<HashMap<String, String>>,
        pt_asked: Mutex<Vec<String>>,
        busy: Mutex<HashSet<String>>,
        entries: Mutex<Vec<Value>>,
        owned_titles: Mutex<HashSet<String>>,
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap()
    }

    impl Env for Fake {
        fn db(&self) -> &Db {
            self.db.as_ref().unwrap()
        }
        fn discover(&self) -> &Discover {
            self.discover.as_ref().unwrap()
        }
        fn settings(&self) -> Map<String, Value> {
            lock(&self.settings).clone()
        }
        fn similar(&self, asin: &str, _: &str, _: usize) -> Result<Vec<Value>, AudibleError> {
            Ok(lock(&self.sims).get(asin).cloned().unwrap_or_default())
        }
        fn genre_products(
            &self,
            _: &str,
            genre: &str,
            sort: &str,
            _: usize,
            page: usize,
        ) -> Result<Vec<Value>, AudibleError> {
            lock(&self.asked).push((genre.into(), sort.into(), page));
            let key = if page == 1 {
                sort.to_string()
            } else {
                format!("{sort}:{page}")
            };
            Ok(lock(&self.lists).get(&key).cloned().unwrap_or_default())
        }
        fn genres(&self, _: &str) -> Result<Vec<Value>, AudibleError> {
            Ok(serde_json::from_value(json!([
                {"id": "1", "name": "Krimis & Thriller", "children": [{"id": "11", "name": "Krimis"}, {"id": "12", "name": "Cricket"}]},
                {"id": "2", "name": "Geschichte", "children": []},
                {"id": "3", "name": "Erotik", "children": [{"id": "31", "name": "Sexpraktiken"}]},
                {"id": "4", "name": "Literatur & Belletristik", "children": [{"id": "41", "name": "Erotik"}]}]))
            .unwrap())
        }
        fn search(&self, _: &str, title: &str, _: &str, _: usize) -> Result<Vec<Value>, AudibleError> {
            let want: String = title.to_lowercase().chars().take(5).collect();
            Ok(lock(&self.hits)
                .iter()
                .filter(|h| s(h, "title").to_lowercase().starts_with(&want))
                .cloned()
                .collect())
        }
        fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
            if url.contains("/autor/") {
                lock(&self.pt_asked).push(url.into());
                return Ok(lock(&self.pt_authors)
                    .get(url)
                    .cloned()
                    .unwrap_or("<h1>Buchautor-Übersicht</h1>".into())
                    .into_bytes());
            }
            if url.ends_with(".pdf") {
                lock(&self.pdf_asked).push(url.into());
                return lock(&self.pdfs).get(url).cloned().ok_or_else(|| "missing".to_string());
            }
            let kind = if url.contains("buchKSL") {
                "list"
            } else if url.contains("perlentaucher") {
                "pt"
            } else if url.contains("rezensionen-des-monats") {
                "month"
            } else {
                "feed"
            };
            lock(&self.feeds)
                .get(kind)
                .cloned()
                .flatten()
                .ok_or_else(|| "down".to_string())
        }
        fn pending(&self) -> HashSet<String> {
            lock(&self.busy).clone()
        }
        fn owned(&self, book: &Value) -> bool {
            lock(&self.owned_titles).contains(s(book, "title"))
        }
        fn library_seeds(&self) -> Vec<Value> {
            let mut all: Vec<Value> = lock(&self.entries)
                .iter()
                .filter(|e| !s(e, "asin").is_empty())
                .cloned()
                .collect();
            all.sort_by(|a, b| num(&b["mtime"]).partial_cmp(&num(&a["mtime"])).unwrap());
            all
        }
        fn seeds(&self, _: i64) -> Vec<Value> {
            Vec::new()
        }
    }

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel>
<item><title>Buchkritik: "Das stille Haus" von Anna Autorin</title><link>https://www.deutschlandfunkkultur.de/a-100.html</link>
<pubDate>Fri, 30 Oct 2026 10:00:00 +0100</pubDate></item>
<item><title>Bert Berger: „Die Geschichte der Wolken“</title><link>https://www.deutschlandfunkkultur.de/b-100.html</link></item>
<item><title>Ein Gespräch über Bücher</title><link>https://www.deutschlandfunkkultur.de/c-100.html</link></item>
</channel></rss>"#;
    const EMPTY: &str = "<rss><channel></channel></rss>";

    fn golden() -> Value {
        serde_json::from_str(include_str!("../../../contract/golden/discover.json")).unwrap()
    }

    fn setup() -> Arc<Fake> {
        let g = golden();
        let fake = Fake {
            db: Some(Db::memory()),
            discover: Some(Discover::new(false)),
            ..Default::default()
        };
        *lock(&fake.settings) = serde_json::from_value(json!({"ab_region": "de", "discover_min_rating": 4.3,
            "discover_min_count": 50, "discover_unabridged": true, "discover_german": true,
            "discover_nonfiction_store": false, "discover_trusted_authors": ""}))
        .unwrap();
        let pt = g["rss"][1][2].as_str().unwrap().as_bytes().to_vec();
        *lock(&fake.feeds) = HashMap::from([
            ("feed", Some(RSS.as_bytes().to_vec())),
            ("pt", Some(pt)),
            ("list", Some(b"<html></html>".to_vec())),
            ("month", Some(b"<html></html>".to_vec())),
        ]);
        Arc::new(fake)
    }

    fn book(asin: &str, title: &str, extra: Value) -> Value {
        let mut b = json!({"asin": asin, "title": title, "authors": ["Anna Autorin"], "narrators": ["Sam Sprecher"],
            "rating": 4.6, "ratings": 300, "genres": ["Krimis & Thriller"], "format_type": "unabridged",
            "language": "german", "is_adult": false, "url": format!("https://www.audible.de/pd/{asin}")});
        for (k, v) in extra.as_object().unwrap() {
            b[k] = v.clone();
        }
        b
    }

    fn items(f: &Arc<Fake>, section: &str, genre: &str) -> Vec<Value> {
        let e: Arc<dyn Env> = f.clone();
        refresh(&e, section, genre, true, None);
        get(&e, section, genre, None)["items"].as_array().unwrap().clone()
    }

    fn asins(v: &[Value]) -> Vec<String> {
        v.iter().map(|i| s(i, "asin").to_string()).collect()
    }

    fn set_feed(f: &Fake, kind: &'static str, v: Option<&str>) {
        lock(&f.feeds).insert(kind, v.map(|x| x.as_bytes().to_vec()));
    }

    #[test]
    fn established_nonfiction_authors_in_for_you() {
        let f = setup();
        *lock(&f.entries) = vec![json!({"asin": "S1", "title": "Seed", "mtime": 1})];
        let nf = json!(["Politik & Sozialwissenschaften"]);
        let b = |a, t, who| book(a, t, json!({"authors": [who], "genres": nf}));
        lock(&f.sims).insert(
            "S1".into(),
            vec![
                b("N1", "Sapiens", "Yuval Noah Harari"),
                b("N2", "Die Kunst, Spuren zu lesen", "Rüdiger Maas"),
                b("N3", "Klima verstehen", "Mai Thi Nguyen-Kim"),
                b("N4", "Die geheime Wahrheit", "Unbekannter Schwurbler"),
                b("N5", "Gute Nachrichten", "Eva Eigene"),
            ],
        );
        lock(&f.pt_authors).insert(
            "https://www.perlentaucher.de/autor/mai-thi-nguyen-kim.html".into(),
            "<h1>Mai</h1><p>4 Bücher</p>".into(),
        );
        lock(&f.settings).insert("discover_trusted_authors".into(), json!("Eva Eigene\nNoch Jemand"));
        let titles: HashSet<String> = items(&f, "foryou", "")
            .iter()
            .map(|i| s(i, "title").to_string())
            .collect();
        let want: HashSet<String> = [
            "Sapiens",
            "Die Kunst, Spuren zu lesen",
            "Klima verstehen",
            "Gute Nachrichten",
        ]
        .map(String::from)
        .into();
        assert_eq!(titles, want);
        assert!(
            lock(&f.pt_asked).contains(&"https://www.perlentaucher.de/autor/unbekannter-schwurbler.html".to_string())
        );
        let asked = lock(&f.pt_asked).len();
        items(&f, "foryou", "");
        assert_eq!(lock(&f.pt_asked).len(), asked); // remembered, not asked again
        assert!(TRUSTED_AUTHORS.len() > 150);
    }

    #[test]
    fn for_you_counts_and_filters() {
        let f = setup();
        *lock(&f.entries) = vec![
            json!({"asin": "S1", "title": "Seed Eins", "mtime": 2}),
            json!({"asin": "S2", "title": "Seed Zwei", "mtime": 1}),
        ];
        lock(&f.owned_titles).insert("Schon da".into());
        let shared = book("B1", "Beide mögen das", json!({}));
        *lock(&f.sims) = HashMap::from([
            (
                "S1".to_string(),
                vec![
                    shared.clone(),
                    book("B2", "Nur eins", json!({})),
                    book("B3", "Schlecht", json!({"rating": 3.2})),
                    book("B4", "Kaum bewertet", json!({"ratings": 4})),
                    book("B5", "Gekürzt", json!({"format_type": "abridged"})),
                    book("B6", "Englisch", json!({"language": "english"})),
                    book("B7", "Schon da", json!({})),
                ],
            ),
            (
                "S2".to_string(),
                vec![
                    shared,
                    book("B8", "Sachbuch", json!({"genres": ["Geschichte"]})),
                    book("B9", "Erotisch", json!({"genres": ["Erotik"]})),
                ],
            ),
        ]);
        let got = items(&f, "foryou", "");
        assert_eq!(asins(&got), ["B1", "B2"]);
        assert_eq!(got[0]["because"], json!(["Seed Eins", "Seed Zwei"]));
        lock(&f.settings).insert("discover_nonfiction_store".into(), json!(true));
        assert!(asins(&items(&f, "foryou", "")).contains(&"B8".to_string()));
    }

    #[test]
    fn genre_lists() {
        let f = setup();
        *lock(&f.lists) = HashMap::from([
            (
                "BestSellers".to_string(),
                vec![book("G1", "Eins", json!({})), book("G2", "Zwei", json!({}))],
            ),
            (
                "-ReleaseDate".to_string(),
                vec![
                    book("G2", "Zwei", json!({})),
                    book("G3", "Drei", json!({"rating": 2.0})),
                ],
            ),
            (
                "AvgRating".to_string(),
                vec![
                    book("G4", "Vier", json!({"rating": 4.9, "ratings": 12})),
                    book("G5", "Fünf", json!({"rating": 4.5, "ratings": 2})),
                ],
            ),
            ("BestSellers:2".to_string(), vec![book("G6", "Sechs", json!({}))]),
        ]);
        assert_eq!(asins(&items(&f, "genre", "1")), ["G1", "G2", "G6", "G4"]);
        let e: Arc<dyn Env> = f.clone();
        assert_eq!(get(&e, "genre", "1", None)["items"][3]["loose"], json!(true));
        assert_eq!(get(&e, "genre", "", None)["items"], json!([]));
        lock(&f.owned_titles).insert("Eins".into()); // owned later: gone from the cached list
        assert_eq!(
            asins(get(&e, "genre", "1", None)["items"].as_array().unwrap()),
            ["G2", "G6", "G4"]
        );
        // non-fiction shows when the genre is picked – not in "Für dich"
        *lock(&f.lists) = HashMap::from([(
            "BestSellers".to_string(),
            vec![book("H1", "Rom", json!({"genres": ["Geschichte"]}))],
        )]);
        assert_eq!(asins(&items(&f, "genre", "2")), ["H1"]);
        *lock(&f.sims) = HashMap::from([(
            "A1".to_string(),
            vec![book("H1", "Rom", json!({"genres": ["Geschichte"]}))],
        )]);
        *lock(&f.entries) = vec![json!({"asin": "A1", "title": "Seed"})];
        assert_eq!(items(&f, "foryou", ""), Vec::<Value>::new());
        // a big genre needs no second page; an unknown one is not looked up at all
        lock(&f.asked).clear();
        *lock(&f.lists) = HashMap::from([(
            "BestSellers".to_string(),
            (0..40)
                .map(|n| book(&format!("B{n}"), &format!("Titel {n}"), json!({})))
                .collect(),
        )]);
        items(&f, "genre", "11");
        assert!(!lock(&f.asked).contains(&("11".to_string(), "BestSellers".to_string(), 2)));
        lock(&f.asked).clear();
        refresh(&e, "genre", "999", true, None);
        assert!(lock(&f.asked).is_empty());
        assert!(!known_genre(&e, "999") && known_genre(&e, "11"));
    }

    #[test]
    fn genres_like_the_shop() {
        let f = setup();
        let e: Arc<dyn Env> = f.clone();
        let gs = genres(&e);
        let names: Vec<&str> = gs.iter().map(|g| s(g, "name")).collect();
        assert_eq!(names, ["Krimis & Thriller", "Geschichte", "Literatur & Belletristik"]);
        assert_eq!(gs[1]["nonfiction"], json!(true));
        assert_eq!(
            gs[0]["children"],
            json!([{"id": "11", "name": "Krimis"}, {"id": "12", "name": "Cricket"}])
        );
        assert_eq!(gs[2]["children"], json!([]));
        items(&f, "genre", "12"); // nothing German there at all …
        assert_eq!(genres(&e)[0]["children"], json!([{"id": "11", "name": "Krimis"}])); // … so it is left out
    }

    #[test]
    fn critics_from_feeds_and_memory() {
        let f = setup();
        set_feed(&f, "pt", None); // one feed down: the other still counts
        *lock(&f.hits) = vec![
            book("R1", "Das stille Haus", json!({"rating": 0, "ratings": 0})),
            book(
                "R2",
                "Die Geschichte der Wolken",
                json!({"authors": ["Bert Berger"], "genres": ["Geschichte"]}),
            ),
            book("R3", "Das stille Haus", json!({"authors": ["Jemand Anderes"]})),
        ];
        let mut got = items(&f, "critics", "");
        got.sort_by(|a, b| s(a, "asin").cmp(s(b, "asin")));
        assert_eq!(asins(&got), ["R1", "R2"]);
        assert_eq!(
            got[0]["review"]["link"],
            json!("https://www.deutschlandfunkkultur.de/a-100.html")
        );
        assert_eq!(got[0]["review"]["source"], json!("Deutschlandfunk Kultur"));

        let f = setup();
        set_feed(&f, "feed", Some(EMPTY));
        *lock(&f.hits) = vec![
            book(
                "P1",
                "Die Mariborhypnose",
                json!({"authors": ["Dora Kapralová"], "rating": 0, "ratings": 0}),
            ),
            book("P2", "Ein Film", json!({"authors": ["Anna Regie"]})),
        ];
        let got = items(&f, "critics", "");
        assert_eq!(asins(&got), ["P1"]); // genitive s and accent tolerated, no film
        assert_eq!(got[0]["review"]["source"], json!("Perlentaucher"));
        set_feed(&f, "pt", Some(EMPTY)); // feed moved on: still remembered
        lock(&f.hits).clear();
        assert_eq!(asins(&items(&f, "critics", "")), ["P1"]);
        lock(&f.busy).insert("P1".into()); // being downloaded: hidden
        let e: Arc<dyn Env> = f.clone();
        assert_eq!(get(&e, "critics", "", None)["items"], json!([]));

        let f = setup();
        set_feed(&f, "feed", Some(EMPTY));
        set_feed(&f, "pt", Some(EMPTY));
        set_feed(&f, "list", Some(golden()["pt_list"][0].as_str().unwrap()));
        *lock(&f.hits) = vec![book(
            "L1",
            "Radio Nacht",
            json!({"authors": ["Juri Andruchowytsch"], "rating": 0, "ratings": 0}),
        )];
        let got = items(&f, "critics", "");
        assert_eq!(asins(&got), ["L1"]);
        assert_eq!(got[0]["review"]["notes"], json!(7));
    }

    #[test]
    fn feed_with_dtd_is_rejected() {
        let f = setup();
        let dtd = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><rss/>"#;
        set_feed(&f, "feed", Some(dtd));
        set_feed(&f, "pt", Some(dtd));
        set_feed(&f, "list", None);
        set_feed(&f, "month", None);
        let e: Arc<dyn Env> = f.clone();
        refresh(&e, "critics", "", true, None);
        let res = get(&e, "critics", "", None);
        assert_eq!(res["items"], json!([]));
        assert!(truthy(&res["error"]));
    }

    #[test]
    fn books_of_the_month_and_late_audiobooks() {
        let f = setup();
        set_feed(&f, "feed", Some(EMPTY));
        set_feed(&f, "pt", Some(EMPTY));
        let month = golden()["dlf_month"][0].as_str().unwrap().to_string();
        set_feed(&f, "month", Some(&month));
        *lock(&f.hits) = vec![book(
            "M1",
            "Wir alle",
            json!({"authors": ["Kerstin Preiwuß"], "rating": 0, "ratings": 0}),
        )];
        let got = items(&f, "critics", "");
        assert_eq!(asins(&got), ["M1"]);
        assert_eq!(got[0]["review"]["month"], json!(true));
        assert_eq!(waiting(f.db()), 3); // the others: no audiobook yet ("Falsch" too)

        let year: i64 = today()[..4].parse().unwrap();
        let future = format!("{}-03-01", year + 1);
        *lock(&f.hits) = vec![
            book(
                "L1",
                "Hi Honey",
                json!({"authors": ["Carlos Barragán"], "rating": 0, "ratings": 0}),
            ),
            book(
                "L2",
                "Das liberale Versprechen",
                json!({"authors": ["Daron Acemoglu"], "rating": 0, "ratings": 0, "release_date": future}),
            ),
        ];
        assert_eq!(asins(&items(&f, "critics", "")), ["M1"]); // looked again only after a month
        let mut memory = f.db().get("discover_reviews").unwrap();
        for v in memory.as_object_mut().unwrap().values_mut() {
            v["checked"] = json!(num(&v["checked"]) - 31.0 * 86400.0);
        }
        f.db().put("discover_reviews", &memory).unwrap();
        let got: HashMap<String, Value> = items(&f, "critics", "")
            .into_iter()
            .map(|i| (s(&i, "asin").to_string(), i))
            .collect();
        assert_eq!(
            got.keys().cloned().collect::<HashSet<_>>(),
            ["M1", "L1", "L2"].map(String::from).into()
        );
        assert_eq!(got["L1"]["later"], json!(true));
        assert_eq!(got["L1"]["announced"], Value::Null);
        assert_eq!(got["L2"]["announced"], json!(future));
        assert_eq!(waiting(f.db()), 1);
    }

    #[test]
    fn waiting_books_are_kept_longer() {
        let f = setup();
        let old = now() - 400.0 * 86400.0;
        f.db()
            .put(
                "discover_reviews",
                &json!({
            "de|a|waiting": {"author": "A", "title": "Waiting", "seen": old, "checked": now(), "book": null},
            "de|b|had": {"author": "B", "title": "Had", "seen": old, "book": book("H1", "Had", json!({}))}}),
            )
            .unwrap();
        set_feed(&f, "feed", Some(EMPTY));
        set_feed(&f, "pt", Some(EMPTY));
        items(&f, "critics", "");
        let keys: Vec<String> = f
            .db()
            .get("discover_reviews")
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, ["de|a|waiting"]);
    }

    #[test]
    fn best_lists_read_once() {
        let f = setup();
        set_feed(&f, "feed", Some(EMPTY));
        set_feed(&f, "pt", Some(EMPTY));
        let g = golden();
        let url = "https://bilder.deutschlandfunk.de/87/07/krimibestenliste-oktober2026-100.pdf";
        set_feed(&f, "month", Some(g["dlf_lists"][0].as_str().unwrap()));
        let pdf = base64::engine::general_purpose::STANDARD
            .decode(g["pdfs"]["list"][0].as_str().unwrap())
            .unwrap();
        lock(&f.pdfs).insert(url.into(), pdf);
        *lock(&f.hits) = vec![book(
            "K1",
            "Die Schakalin",
            json!({"authors": ["Mike Nicol"], "rating": 0, "ratings": 0}),
        )];
        let got = items(&f, "critics", "");
        assert_eq!(asins(&got), ["K1"]);
        let r = &got[0]["review"];
        assert_eq!(
            json!([r["list"], r["rank"], r["link"], r["date"]]),
            json!(["Die besten Krimis im Oktober 2026", 5, url, "Fri, 02 Oct 2026"])
        );
        assert!(
            f.db()
                .get("discover_reviews")
                .unwrap()
                .get("de|naomi klein|endzeitfaschismus")
                .is_some()
        );
        items(&f, "critics", "");
        // each list PDF only once (the second, broken one is tried again the next run)
        assert_eq!(lock(&f.pdf_asked).iter().filter(|u| u.as_str() == url).count(), 1);
    }
}
