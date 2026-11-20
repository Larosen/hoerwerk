//! Fill the gaps in the details of library books: when Hörwerk is sure which edition a book is (its ASIN, or a
//! close match of title, author and length), the year, narrators, publisher, description, series … that the book
//! lacks are taken from there – and a missing cover. Nothing that is there already is changed. The book keeps its
//! place: same folder, same file name, only its tags (and `metadata.json`) are written.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::app::App;
use crate::text::t;
use crate::{audible, encoder, media, tagger};

/// The fields that are only filled, never overwritten: (key, label).
fn fields() -> Vec<(&'static str, String)> {
    vec![
        ("release_date", t("Jahr", &[])),
        ("narrators", t("Sprecher", &[])),
        ("publisher", t("Verlag", &[])),
        ("series", t("Serie", &[])),
        ("series_position", t("Folge", &[])),
        ("subtitle", t("Untertitel", &[])),
        ("genres", t("Genres", &[])),
        ("language", t("Sprache", &[])),
        ("description", t("Beschreibung", &[])),
        ("asin", "ASIN".to_string()),
    ]
}

fn empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn show(v: &Value) -> String {
    let text = match v {
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let short: String = text.chars().take(140).collect();
    if text.chars().count() > 140 {
        format!("{short} …")
    } else {
        short
    }
}

/// The year of a date as it is written ("2016-03-24" -> "2016").
fn year(date: &str) -> String {
    date.chars().take(4).collect()
}

/// What the matching edition `m` has that the book lacks: (key, label, value to write). Only gaps – what the book
/// has is never in the list. The date is cut to its year (the shop's day may differ from the audiobook's).
fn gaps(book: &Value, m: &Value) -> Vec<(&'static str, String, Value)> {
    fields()
        .into_iter()
        .filter_map(|(key, label)| {
            let new = if key == "release_date" {
                Some(year(m[key].as_str().unwrap_or("")))
                    .filter(|y| !y.is_empty())
                    .map_or(Value::Null, |y| json!(y))
            } else {
                m[key].clone()
            };
            (empty(&book[key]) && !empty(&new)).then_some((key, label, new))
        })
        .collect()
}

/// The edition Hörwerk is sure about, or why not: Err(message) when the lookup failed, Ok(None) when nothing is
/// sure enough.
fn candidate(app: &App, entry: &Value, file: &Path, book: &Value, auto: bool) -> Result<Option<Value>, String> {
    let settings = app.settings();
    let region = settings
        .get("ab_region")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
        .unwrap_or("de")
        .to_string();
    let dur = media::probe_cached(file)["duration"].as_f64().unwrap_or(0.0);
    let asin = entry["asin"].as_str().unwrap_or("");
    if !asin.is_empty() {
        // the edition is known: the book's own ASIN – unless the length says the file is another recording
        return match app.providers.client.product(asin, &region) {
            Ok(p) if crate::health::runtime_off(dur, p["runtime_min"].as_i64().unwrap_or(0)) => Ok(None),
            Ok(p) => Ok(Some(p)),
            Err(e) => Err(e.0),
        };
    }
    let hints = json!({
        "title": entry["title"], "author": entry["authors"].as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null),
        "duration": dur, "query": format!("{} {}", entry["title"].as_str().unwrap_or(""), book["authors"].as_array().and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("")),
    });
    let threshold = settings
        .get("ab_threshold")
        .and_then(Value::as_f64)
        .filter(|x| *x != 0.0)
        .unwrap_or(0.8);
    let (ranked, _) = app.providers.lookup(&hints, &region, true);
    // by itself (nobody looks at the result) only what is beyond doubt: a higher bar than for a book that waits for a look
    let threshold = if auto { threshold.max(AUTO_MIN_SCORE) } else { threshold };
    Ok(audible::auto_match(&ranked, threshold).filter(|m| !auto || sure(m)))
}

/// Score a match needs to be taken over without anybody looking (the setting may only raise it).
const AUTO_MIN_SCORE: f64 = 0.85;

/// Beyond doubt: the runtime agrees within about 4 % – or, when the runtime of one side is unknown, title and
/// author are near identical.
pub fn sure(m: &Value) -> bool {
    let x = &m["match"];
    let (title, author) = (x["title"].as_f64().unwrap_or(0.0), x["author"].as_f64().unwrap_or(0.0));
    match x["duration"].as_f64() {
        Some(d) => d >= 0.9 && title >= 0.8,
        None => title >= 0.95 && author >= 0.9,
    }
}

/// What would be added to a book – and the matching edition. `apply` writes it.
pub fn plan(app: &App, rel: &str) -> Result<Value, String> {
    let entry = app.library.entry(rel).ok_or_else(|| t("Nicht gefunden", &[]))?;
    let folder = app.cfg.library.join(rel);
    let audio = media::collect_audio(&folder);
    let [file] = audio.as_slice() else {
        return Ok(json!({"status": "no_m4b", "changes": []}));
    };
    let file = folder.join(file);
    if !file.to_string_lossy().to_lowercase().ends_with(".m4b") {
        return Ok(json!({"status": "no_m4b", "changes": []}));
    }
    let book = media::read_book(&file);
    let Some(m) = candidate(app, &entry, &file, &book, false)? else {
        return Ok(json!({"status": "unsure", "changes": []}));
    };
    let mut changes: Vec<Value> = gaps(&book, &m)
        .into_iter()
        .map(|(key, label, new)| json!({"field": key, "label": label, "to": show(&new)}))
        .collect();
    let cover_missing = entry["cover"] != true && !s(&m, "cover_url").is_empty();
    if cover_missing {
        changes.push(json!({"field": "cover", "label": t("Cover", &[]), "to": s(&m, "cover_url"), "image": true}));
    }
    Ok(json!({
        "status": if changes.is_empty() { "complete" } else { "ready" },
        "match": {"title": m["title"], "source": m["source"], "asin": m["asin"]},
        "changes": changes,
    }))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// Write what `plan` found into the book's M4B (tags, cover) and `metadata.json`; returns how many things were added.
pub fn apply(app: &Arc<App>, rel: &str) -> Result<usize, String> {
    match apply_inner(app, rel, false)? {
        Outcome::Added { n, .. } => {
            crate::mediaservers::request_scans(app);
            Ok(n)
        }
        _ => Ok(0),
    }
}

/// What came of looking at a book.
pub enum Outcome {
    /// not one M4B, or nothing to look at
    Skipped,
    /// no edition that is sure enough
    Unsure,
    /// the edition is known (`source`) and the book lacks nothing
    Complete { source: String },
    /// details were added; `source` is where the edition is from, `asin` whether the book has an Audible link now
    Added {
        n: usize,
        source: String,
        asin: bool,
        title: String,
    },
}

/// The same by itself, in the background: only editions beyond doubt (a higher bar for a book without an ASIN), and no
/// media server scan per book – the caller asks once at the end.
pub fn auto(app: &Arc<App>, rel: &str) -> Result<Outcome, String> {
    apply_inner(app, rel, true)
}

fn apply_inner(app: &Arc<App>, rel: &str, auto: bool) -> Result<Outcome, String> {
    let entry = app.library.entry(rel).ok_or_else(|| t("Nicht gefunden", &[]))?;
    let folder = app.cfg.library.join(rel);
    let audio = media::collect_audio(&folder);
    let [name] = audio.as_slice() else {
        if auto {
            return Ok(Outcome::Skipped);
        }
        return Err(t("Daten ergänzen geht nur bei einer einzelnen M4B", &[]));
    };
    let file = folder.join(name);
    if !name.to_lowercase().ends_with(".m4b") {
        return Ok(Outcome::Skipped);
    }
    let mut book = media::read_book(&file);
    let Some(m) = candidate(app, &entry, &file, &book, auto)? else {
        return Ok(Outcome::Unsure);
    };
    let mut added = 0;
    for (key, _, new) in gaps(&book, &m) {
        book[key] = new;
        added += 1;
    }
    let mut cover: Option<Vec<u8>> = None;
    if entry["cover"] != true
        && !s(&m, "cover_url").is_empty()
        && let Ok(Some(c)) = audible::fetch_image(s(&m, "cover_url"))
    {
        cover = Some(c);
        added += 1;
    }
    if added == 0 {
        return Ok(Outcome::Complete {
            source: s(&m, "source").to_string(),
        });
    }
    let settings = Value::Object(app.settings());
    // the tags are rewritten in place: same file, same place in the library; the picture stays when there is no new one
    tagger::write_tags(&file, &book, &settings, cover.clone())?;
    if let Some(c) = &cover {
        let settings = app.settings();
        let _ = tagger::write_sidecars(&folder, &book, &settings, Some(c));
    }
    if folder.join(tagger::METADATA_FILE).is_file() {
        let dur = media::probe_cached(&file)["duration"].as_f64().unwrap_or(0.0);
        let _ = tagger::write_abs_metadata(&folder, &book, &encoder::file_chapters(&file), dur);
    }
    app.library.refresh(rel);
    app.health.recheck(app, rel);
    Ok(Outcome::Added {
        n: added,
        source: s(&m, "source").to_string(),
        asin: !s(&book, "asin").is_empty(),
        title: s(&m, "title").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_is_beyond_doubt_is_taken_over_by_itself() {
        let m = |title: f64, author: f64, duration: Value| json!({"match": {"title": title, "author": author, "duration": duration}});
        // runtime within about 4 %, title fits: sure
        assert!(sure(&m(0.9, 0.5, json!(1.0))));
        assert!(sure(&m(0.85, 0.5, json!(0.92))));
        // the runtime is off (another recording) or the title does not fit: not sure
        assert!(!sure(&m(0.95, 0.95, json!(0.6))));
        assert!(!sure(&m(0.6, 0.95, json!(1.0))));
        // no runtime to compare: only a near identical title and author
        assert!(sure(&m(0.97, 0.95, Value::Null)));
        assert!(!sure(&m(0.9, 0.95, Value::Null)));
    }

    #[test]
    fn only_gaps_are_filled() {
        let book = json!({"title": "Der Federmann", "narrators": ["Simon Jäger"], "release_date": null, "publisher": "", "genres": []});
        let m = json!({"title": "Der Federmann", "narrators": ["Jemand Anderes"], "release_date": "2016-03-24", "publisher": "Audible Studios",
                       "genres": ["Thriller"], "series": null, "description": "Ein Thriller.", "language": "German", "asin": "B01"});
        let got: Vec<(&str, Value)> = gaps(&book, &m).into_iter().map(|(k, _, v)| (k, v)).collect();
        // the narrators the book has stay; the year (not the day), publisher, genres, language, description and ASIN are added
        assert_eq!(
            got,
            vec![
                ("release_date", json!("2016")),
                ("publisher", json!("Audible Studios")),
                ("genres", json!(["Thriller"])),
                ("language", json!("German")),
                ("description", json!("Ein Thriller.")),
                ("asin", json!("B01")),
            ]
        );
        // nothing to add to a complete book
        assert!(gaps(&m, &m).is_empty());
    }
}
