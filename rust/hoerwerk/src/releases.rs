//! Is this release the requested book – and how good is it? (like Radarr's decision engine.)
//! `evaluate` gives every indexer result a verdict with a reason a person can read.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::packs;
use crate::quality;
use crate::rx;
use crate::rx::{findall, is_match, sub};
use crate::text::t;
use crate::usenet::release_info;
use crate::words::match_words;

const STOP: [&str; 14] = [
    "der", "die", "das", "und", "ein", "eine", "the", "and", "von", "mit", "roman", "band", "teil", "folge",
];
const MIN_KBPS: i64 = 20;
const MAX_KBPS: i64 = 1600;
const LOW_KBPS: i64 = 48;
pub const SEARCH_BUDGET: usize = 9;

/// Python's round(): halves to even.
pub fn py_round(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Python's round(x, 1), printed like Python prints a float ("8.0", "12.5").
fn py_round1(x: f64) -> String {
    format!("{:?}", (x * 10.0).round_ties_even() / 10.0)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn first_author(book: &Value) -> String {
    book["authors"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub fn surname(name: &str) -> String {
    match_words(name).pop().unwrap_or_default()
}

pub fn title_words(title: &str) -> Vec<String> {
    match_words(&clean_title(title))
        .into_iter()
        .filter(|w| !STOP.contains(&w.as_str()) && w.chars().count() > 1)
        .collect()
}

/// The title as release names carry it: without the series remark the shops append.
pub fn clean_title(title: &str) -> String {
    let mut s = title.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let a = sub(rx!(r"\s*[\(\[][^\(\)\[\]]*[\)\]]\s*$"), &s, "");
        let shorter = sub(
            rx!(r"(?i)\s*[,:\-–—]\s*(?:band|bd|teil|folge|buch|book|vol|volume|nr)\.?\s*\d{1,3}\s*$"),
            &a,
            "",
        )
        .trim()
        .to_string();
        if shorter == s || !is_match(rx!(r"\w"), &shorter) {
            return s;
        }
        s = shorter;
    }
}

/// The surname as written ("Nesbø", not "nesbo") – for searching.
pub fn name_surname(name: &str) -> String {
    findall(rx!(r"[^\W\d_][\w'’-]*"), name).pop().unwrap_or_default()
}

/// "Ein Bodenstein-Kirchhoff-Krimi" -> "Bodenstein Kirchhoff" – what release names call the series.
pub fn series_core(series: &str) -> String {
    let mut s = clean_title(series);
    for _ in 0..3 {
        s = sub(
            rx!(r"(?i)^(?:ein|eine|der|die|das|a|an|the)\s+|[\s-]+(?:krimi|thriller|roman|reihe|serie|saga|series|fall|f(?:ä|ae)lle|mystery|novel)s?$"),
            &s,
            "",
        )
        .trim()
        .to_string();
    }
    sub(rx!(r"[-–&+/]"), &s, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What to ask the indexers for a book, in rounds – the next round only when the one before found nothing that fits.
pub fn search_stages(book: &Value) -> Vec<Vec<String>> {
    let author = first_author(book).split_whitespace().collect::<Vec<_>>().join(" ");
    let sur = name_surname(&author);
    let title = clean_title(s(book, "title"));
    let words: Vec<&str> = title.split_whitespace().collect();
    let short = if words.len() > 4 {
        words[..3].join(" ")
    } else {
        String::new()
    };
    let series = series_core(s(book, "series"));
    let mut stages: Vec<Vec<String>> = vec![vec![format!("{sur} {title}").trim().to_string()]];
    let mut second = Vec::new();
    if words.len() >= 2 || title.chars().count() >= 8 {
        second.push(title.clone());
    }
    if !short.is_empty() && !sur.is_empty() {
        second.push(format!("{sur} {short}"));
    }
    if !series.is_empty()
        && !sur.is_empty()
        && series.chars().count() >= 4
        && match_words(&series) != match_words(&title)
    {
        second.push(format!("{sur} {series}"));
    }
    stages.push(second);
    let plain = match_words(&sur);
    stages.push(if author.is_empty() {
        Vec::new()
    } else if plain.last().is_some_and(|w| w.chars().count() >= 4) {
        vec![sur.clone()]
    } else {
        vec![author.clone()]
    });
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for stage in stages {
        let stage: Vec<String> = stage
            .into_iter()
            .filter(|q| !q.is_empty() && !seen.contains(&q.to_lowercase()))
            .collect();
        for q in &stage {
            seen.insert(q.to_lowercase());
        }
        if !stage.is_empty() {
            out.push(stage);
        }
    }
    out
}

/// "de", "en" or "" (unknown) – the language a request is for.
pub fn book_language(book: &Value) -> String {
    match s(book, "language").trim().to_lowercase().as_str() {
        "german" | "deutsch" | "de" => "de",
        "english" | "englisch" | "en" => "en",
        _ => "",
    }
    .into()
}

const DE_WORDS: [&str; 27] = [
    "der",
    "die",
    "das",
    "und",
    "zum",
    "zur",
    "vom",
    "bis",
    "biss",
    "ein",
    "eine",
    "des",
    "den",
    "dem",
    "im",
    "auf",
    "aus",
    "mit",
    "nach",
    "hoerbuch",
    "horbuch",
    "ungekurzt",
    "gekurzt",
    "gelesen",
    "teil",
    "folge",
    "band",
];
const EN_WORDS: [&str; 12] = [
    "the",
    "of",
    "and",
    "in",
    "to",
    "a",
    "an",
    "unabridged",
    "narrated",
    "book",
    "audiobook",
    "edition",
];

/// "de"/"en" from the words of a release name without a language tag.
pub fn guess_language(name: &str) -> String {
    if is_match(rx!(r"[äöüÄÖÜß]"), name) {
        return "de".into();
    }
    let words = match_words(name);
    let de = words.iter().filter(|w| DE_WORDS.contains(&w.as_str())).count();
    let en = words.iter().filter(|w| EN_WORDS.contains(&w.as_str())).count();
    if de >= 2 && de > en {
        "de".into()
    } else if en >= 2 && en > de {
        "en".into()
    } else {
        String::new()
    }
}

/// Author's surname and (nearly) all words of the title in the release name.
pub fn fits(release: &str, book: &Value) -> bool {
    evaluate(&json!({"title": release}), book, "de", true, &[], false)["ok"]
        .as_bool()
        .unwrap_or(false)
}

/// The volume number of a book (int(float(series_position))) – None when there is none.
pub fn volumes(book: &Value) -> Option<i64> {
    let raw = match &book["series_position"] {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        Value::Bool(false) => String::new(),
        other => other.to_string(),
    };
    let f: f64 = raw.replace(',', ".").trim().parse().ok()?;
    f.is_finite().then_some(f.trunc() as i64)
}

/// Volume numbers a release name states – after a volume word, or right after the series name.
pub fn release_volumes(name: &str, book: &Value) -> BTreeSet<i64> {
    let text = sub(rx!(r"[-_.+]"), name, " ");
    let mut found: BTreeSet<i64> = findall(
        rx!(r"(?i)(?:\b(?:band|bd|teil|folge|buch|book|vol|volume|nr|saga|reihe|serie|series|episode|ep)\b\.?|#)\s*0*(\d{1,3})(?![\da-z])"),
        &text,
    )
    .iter()
    .filter_map(|n| n.parse().ok())
    .collect();
    let mut anchors: BTreeSet<String> = BTreeSet::new();
    for w in match_words(&series_core(s(book, "series")))
        .into_iter()
        .chain(title_words(s(book, "title")))
    {
        if w.chars().count() >= 4 && !w.chars().all(|c| c.is_ascii_digit()) && !STOP.contains(&w.as_str()) {
            anchors.insert(w);
        }
    }
    let plain = match_words(&text).join(" ");
    for w in anchors {
        if let Ok(re) = fancy_regex::Regex::new(&format!(r"\b{}\s+0*(\d{{1,3}})(?![\da-z])", rx::escape(&w))) {
            found.extend(findall(&re, &plain).iter().filter_map(|n| n.parse::<i64>().ok()));
        }
    }
    found.into_iter().filter(|n| *n > 0 && *n < 500).collect()
}

fn list(set: &BTreeSet<i64>) -> String {
    set.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
}

/// {ok, score, reason (why rejected), notes (why it scores), kbps}.
pub fn evaluate(
    release: &Value,
    book: &Value,
    prefer_language: &str,
    unabridged: bool,
    blocked: &[String],
    check_size: bool,
) -> Value {
    let name = s(release, "title");
    let words: BTreeSet<String> = match_words(name).into_iter().collect();
    let mut notes: Vec<String> = Vec::new();
    let runtime = book["runtime_min"]
        .as_f64()
        .map(|f| f as i64)
        .or_else(|| s(book, "runtime_min").parse().ok())
        .unwrap_or(0);
    let size = release["size"].as_f64().map(|f| f as i64).unwrap_or(0);
    let pack = packs::pack_info(name);
    let kbps: Option<i64> = (runtime != 0 && size != 0 && pack.is_none())
        .then(|| py_round(size as f64 * 8.0 / 1000.0 / (runtime as f64 * 60.0)));
    let no = |reason: String, notes: &Vec<String>| json!({"ok": false, "score": 0.0, "reason": reason, "notes": notes, "kbps": kbps});
    if blocked.iter().any(|b| b == name) {
        return no(t("gesperrt (Download oder Import ging schief)", &[]), &notes);
    }
    let author = surname(&first_author(book));
    if !author.is_empty() && !words.contains(&author) {
        return no(
            t("Autor „{name}“ fehlt im Namen", &[("name", &first_author(book))]),
            &notes,
        );
    }
    let title = title_words(s(book, "title"));
    if title.is_empty() {
        return no(t("Buch ohne Titel", &[]), &notes);
    }
    let share = title.iter().filter(|w| words.contains(*w)).count() as f64 / title.len() as f64;
    if share < 0.8 {
        return no(
            t(
                "Titel passt nicht ({n} % der Wörter)",
                &[("n", &py_round(share * 100.0).to_string())],
            ),
            &notes,
        );
    }
    let vol = volumes(book);
    if let Some(p) = &pack {
        let vols: Vec<i64> = p["volumes"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        match vol {
            Some(v) if vols.contains(&v) => {
                notes.push(t("Paket – nur Band {n} wird übernommen", &[("n", &v.to_string())]))
            }
            _ => {
                let (first, last) = (vols.first().copied().unwrap_or(0), vols.last().copied().unwrap_or(0));
                return no(
                    t(
                        "Paket ohne diesen Band ({first}–{last})",
                        &[("first", &first.to_string()), ("last", &last.to_string())],
                    ),
                    &notes,
                );
            }
        }
    } else {
        // a series book: a release that names another volume is another book ("Twilight-Saga 5" for Band 1)
        let stated = release_volumes(name, book);
        if let Some(v) = vol {
            if !stated.is_empty() && !stated.contains(&v) {
                return no(
                    t(
                        "anderer Band ({found} statt {vol})",
                        &[("found", &list(&stated)), ("vol", &v.to_string())],
                    ),
                    &notes,
                );
            }
            if stated.contains(&v) {
                notes.push(t("Band {n} passt", &[("n", &v.to_string())]));
            }
        }
    }
    if unabridged && release["abridged"].as_bool().unwrap_or(false) {
        return no(t("gekürzte Fassung", &[]), &notes);
    }
    let lang = s(release, "language");
    if !prefer_language.is_empty() && !lang.is_empty() && lang != prefer_language {
        return no(t("andere Sprache ({lang})", &[("lang", lang)]), &notes);
    }
    if !prefer_language.is_empty() && lang.is_empty() {
        let guessed = guess_language(name);
        if !guessed.is_empty() && guessed != prefer_language {
            return no(t("vermutlich andere Sprache ({lang})", &[("lang", &guessed)]), &notes);
        }
    }
    let format = s(release, "format").to_string();
    let lossless = matches!(format.to_uppercase().as_str(), "FLAC" | "LOSSLESS" | "WAV");
    let hours = py_round1(runtime as f64 / 60.0);
    if check_size && let Some(k) = kbps {
        if lossless && k < 200 {
            return no(
                t(
                    "verlustfrei, aber viel zu klein für {h} h Laufzeit ({kbps} kbit/s)",
                    &[("h", &hours), ("kbps", &k.to_string())],
                ),
                &notes,
            );
        }
        if k < MIN_KBPS {
            return no(
                t(
                    "zu klein für {h} h Laufzeit ({kbps} kbit/s)",
                    &[("h", &hours), ("kbps", &k.to_string())],
                ),
                &notes,
            );
        }
        if k > MAX_KBPS {
            return no(
                t(
                    "zu groß für {h} h Laufzeit ({kbps} kbit/s)",
                    &[("h", &hours), ("kbps", &k.to_string())],
                ),
                &notes,
            );
        }
    }
    let mut score = match format.as_str() {
        "M4B" => 30.0,
        "M4A" => 20.0,
        "MP3" => 15.0,
        "FLAC" => 10.0,
        _ => 5.0,
    };
    if !format.is_empty() {
        notes.push(format.clone());
    }
    if !lang.is_empty() && lang == prefer_language {
        score += 10.0;
        notes.push(t("Sprache passt", &[]));
    }
    let drama = quality::is_drama(book, "");
    match kbps {
        Some(k) if !lossless => {
            if k < LOW_KBPS {
                score -= 40.0;
                notes.push(t("geringe Qualität ({kbps} kbit/s)", &[("kbps", &k.to_string())]));
            } else {
                score += k.min(if drama { 320 } else { 160 }) as f64 / 10.0;
            }
        }
        Some(_) if drama => {
            score += 40.0;
            notes.push(t("verlustfrei", &[]));
        }
        _ => {}
    }
    if is_match(rx!(r"(?i)\batmos\b|\be-?ac-?3\b|\bddp\b|\bdd\+|\bdolby\b"), name) {
        score += 12.0;
        notes.push(t("Dolby Atmos", &[]));
    }
    let year = findall(rx!(r"(?<!\d)((?:19|20)\d\d)(?!\d)"), name)
        .iter()
        .filter_map(|y| y.parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    if year != 0 {
        score += ((year - 2005) as f64 / 2.0).clamp(0.0, 8.0).max(0.0);
    }
    let grabs = release["grabs"].as_f64().map(|f| f as i64).unwrap_or(0);
    score += grabs.min(200) as f64 / 20.0;
    score += share * 5.0 - if pack.is_some() { 8.0 } else { 0.0 };
    if pack.is_none()
        && let Some(v) = vol
        && release_volumes(name, book).contains(&v)
    {
        score += 20.0;
    }
    json!({"ok": true, "score": (score * 100.0).round_ties_even() / 100.0, "reason": null, "notes": notes, "kbps": kbps})
}

/// Why a downloaded folder can't be this book – another volume, another language, a far other runtime – or None.
pub fn mismatch(name: &str, book: &Value, duration: f64) -> Option<String> {
    if packs::pack_info(name).is_none() {
        let stated = release_volumes(name, book);
        if let Some(v) = volumes(book)
            && !stated.is_empty()
            && !stated.contains(&v)
        {
            return Some(t(
                "anderer Band ({found} statt {vol})",
                &[("found", &list(&stated)), ("vol", &v.to_string())],
            ));
        }
    }
    let want = book_language(book);
    let info = release_info(name);
    let got = match info["language"].as_str() {
        Some(l) => l.to_string(),
        None => guess_language(name),
    };
    if !want.is_empty() && !got.is_empty() && got != want {
        return Some(t("andere Sprache ({lang})", &[("lang", &got)]));
    }
    let runtime = book["runtime_min"].as_f64().map(|f| f as i64).unwrap_or(0) * 60;
    if duration > 0.0 && runtime > 0 && !(0.6..=1.67).contains(&(duration / runtime as f64)) {
        return Some(t(
            "Laufzeit passt nicht ({have} statt {want} min)",
            &[
                ("have", &py_round(duration / 60.0).to_string()),
                ("want", &(runtime / 60).to_string()),
            ],
        ));
    }
    None
}

/// All results with their verdict, the accepted ones first (best first).
pub fn rank(
    results: &[Value],
    book: &Value,
    prefer_language: &str,
    unabridged: bool,
    blocked: &[String],
) -> Vec<Value> {
    let mut out: Vec<Value> = results
        .iter()
        .map(|r| {
            let mut r = r.clone();
            r["verdict"] = evaluate(&r, book, prefer_language, unabridged, blocked, true);
            r
        })
        .collect();
    out.sort_by(|a, b| {
        let key = |r: &Value| {
            (
                !r["verdict"]["ok"].as_bool().unwrap_or(false),
                -r["verdict"]["score"].as_f64().unwrap_or(0.0),
            )
        };
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0)
            .then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn like_python() {
        crate::text::set_language("de");
        let g: Value =
            serde_json::from_str(include_str!("../../../contract/golden/releases.json")).expect("releases.json");
        let books = g["books"].as_array().unwrap();
        let mut wrong = Vec::new();
        for case in g["evaluate"].as_array().unwrap() {
            let book = &books[case[0].as_u64().unwrap() as usize];
            let mut rel = case[1].clone();
            for (k, v) in crate::usenet::release_info(rel["title"].as_str().unwrap())
                .as_object()
                .unwrap()
            {
                rel[k] = v.clone();
            }
            let a = evaluate(&rel, book, "de", true, &[], true);
            let b = evaluate(&rel, book, "en", false, &[], false);
            if a != case[2] || b != case[3] {
                wrong.push(format!(
                    "{} / {}:\n  rust   {a}\n  python {}\n  rust   {b}\n  python {}",
                    rel["title"], book["title"], case[2], case[3]
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
        let first = &g["evaluate"][0][1];
        assert_eq!(
            evaluate(
                &json!({"title": first["title"]}),
                &books[0],
                "de",
                true,
                &[first["title"].as_str().unwrap().to_string()],
                true
            ),
            g["blocked"]
        );
        let all: Vec<Value> = g["evaluate"]
            .as_array()
            .unwrap()
            .iter()
            .take(19)
            .map(|c| {
                let mut r = c[1].clone();
                for (k, v) in crate::usenet::release_info(r["title"].as_str().unwrap())
                    .as_object()
                    .unwrap()
                {
                    r[k] = v.clone();
                }
                r
            })
            .collect();
        let ranked: Vec<String> = rank(&all, &books[0], "de", true, &[])
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ranked, strs(&g["rank"]));
        for (i, b) in books.iter().enumerate() {
            let want: Vec<Vec<String>> = g["search_stages"][i].as_array().unwrap().iter().map(strs).collect();
            assert_eq!(search_stages(b), want, "stages {}", b["title"]);
        }
        for c in g["release_volumes"].as_array().unwrap() {
            let want: Vec<i64> = c[2].as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect();
            let got: Vec<i64> = release_volumes(c[0].as_str().unwrap(), &books[c[1].as_u64().unwrap() as usize])
                .into_iter()
                .collect();
            assert_eq!(got, want, "volumes {}", c[0]);
        }
        for c in g["pack_info"].as_array().unwrap() {
            assert_eq!(
                packs::pack_info(c[0].as_str().unwrap()).unwrap_or(Value::Null),
                c[1],
                "pack {}",
                c[0]
            );
        }
        for c in g["volume_of"].as_array().unwrap() {
            let pack = c[1].as_str().and_then(packs::pack_info);
            assert_eq!(
                json!(packs::volume_of(c[0].as_str().unwrap(), pack.as_ref())),
                c[2],
                "volume_of {}",
                c[0]
            );
        }
        for c in g["title_of"].as_array().unwrap() {
            let pack = c[1].as_str().and_then(packs::pack_info);
            assert_eq!(
                packs::title_of(c[0].as_str().unwrap(), pack.as_ref()),
                c[2].as_str().unwrap(),
                "title_of {}",
                c[0]
            );
        }
        for c in g["release_info"].as_array().unwrap() {
            assert_eq!(
                crate::usenet::release_info(c[0].as_str().unwrap()),
                c[1],
                "release_info {}",
                c[0]
            );
        }
        for c in g["query_variants"].as_array().unwrap() {
            assert_eq!(
                crate::usenet::query_variants(c[0].as_str().unwrap()),
                strs(&c[1]),
                "variants {}",
                c[0]
            );
        }
        for c in g["clean_title"].as_array().unwrap() {
            assert_eq!(
                clean_title(c[0].as_str().unwrap()),
                c[1].as_str().unwrap(),
                "clean_title {}",
                c[0]
            );
        }
        for c in g["series_core"].as_array().unwrap() {
            assert_eq!(
                series_core(c[0].as_str().unwrap()),
                c[1].as_str().unwrap(),
                "series_core {}",
                c[0]
            );
        }
        for c in g["guess_language"].as_array().unwrap() {
            assert_eq!(
                guess_language(c[0].as_str().unwrap()),
                c[1].as_str().unwrap(),
                "guess {}",
                c[0]
            );
        }
        for c in g["mismatch"].as_array().unwrap() {
            let got = mismatch(
                c[0].as_str().unwrap(),
                &books[c[1].as_u64().unwrap() as usize],
                c[2].as_f64().unwrap(),
            );
            assert_eq!(json!(got), c[3], "mismatch {}", c[0]);
        }
        let parsed = crate::usenet::parse_results(g["feed"].as_str().unwrap().as_bytes(), "Test").unwrap();
        assert_eq!(json!(parsed), g["parsed"]);
        let cats = crate::usenet::audiobook_categories(g["caps"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(json!(cats), g["caps_categories"]);
        let dtd = br#"<?xml version="1.0"?><!-- x --><!DOCTYPE r [<!ENTITY a "b">]><r>&a;</r>"#;
        assert_eq!(
            crate::usenet::parse_results(dtd, "Test").unwrap_err().to_string(),
            g["dtd_refused"].as_str().unwrap()
        );
    }
}
