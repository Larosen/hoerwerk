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
            // an "ASIN" the shop does not know (an ISBN in the tag, a book of its own): by itself that is no trouble
            // with the net – there is just nothing to add, and several of these in a row must not end the whole run
            Err(e)
                if auto
                    && e.0
                        == t(
                            "ASIN {asin} im Audible-Shop ({region}) nicht gefunden",
                            &[("asin", asin), ("region", &region)],
                        ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.0),
        };
    }
    let hints = lookup_hints(entry, book, dur);
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

/// What the lookup of an edition is given: title, author and the length of the file.
fn lookup_hints(entry: &Value, book: &Value, dur: f64) -> Value {
    json!({
        "title": entry["title"], "author": entry["authors"].as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null),
        "duration": dur, "query": format!("{} {}", entry["title"].as_str().unwrap_or(""), book["authors"].as_array().and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("")),
    })
}

/// The link of a book that was corrected: the ASIN it had, the one it has now, and the edition's title.
pub struct Retargeted {
    pub old: String,
    pub new: String,
    pub title: String,
}

/// A book whose Audible link contradicts the file (Audible's length is far off): it is probably another work. When
/// another edition is beyond doubt – the title fits and the length agrees with the file – the link is corrected
/// (only the ASIN and what the book lacks; what the book says stays). Anything less sure: the book is left alone.
pub fn retarget(app: &Arc<App>, rel: &str) -> Result<Option<Retargeted>, String> {
    let entry = app.library.entry(rel).ok_or_else(|| t("Nicht gefunden", &[]))?;
    let old = s(&entry, "asin").to_string();
    if old.is_empty() {
        return Ok(None);
    }
    let folder = app.cfg.library.join(rel);
    let audio = media::collect_audio(&folder);
    let [name] = audio.as_slice() else { return Ok(None) };
    if !name.to_lowercase().ends_with(".m4b") {
        return Ok(None);
    }
    let file = folder.join(name);
    let mut book = media::read_book(&file);
    let dur = media::probe_cached(&file)["duration"].as_f64().unwrap_or(0.0);
    let settings = app.settings();
    let region = settings
        .get("ab_region")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
        .unwrap_or("de")
        .to_string();
    let threshold = settings
        .get("ab_threshold")
        .and_then(Value::as_f64)
        .filter(|x| *x != 0.0)
        .unwrap_or(0.8)
        .max(AUTO_MIN_SCORE);
    let (ranked, _) = app.providers.lookup(&lookup_hints(&entry, &book, dur), &region, true);
    let Some(m) = audible::auto_match(&ranked, threshold).filter(|m| retarget_ok(m, &old)) else {
        return Ok(None);
    };
    let new = s(&m, "asin").to_string();
    book["asin"] = json!(new);
    for (key, _, value) in gaps(&book, &m) {
        book[key] = value;
    }
    let settings = Value::Object(app.settings());
    tagger::write_tags(&file, &book, &settings, None)?;
    if folder.join(tagger::METADATA_FILE).is_file() {
        let _ = tagger::write_abs_metadata(&folder, &book, &encoder::file_chapters(&file), dur);
    }
    app.library.refresh(rel);
    app.health.recheck(app, rel);
    Ok(Some(Retargeted {
        old,
        new,
        title: s(&m, "title").to_string(),
    }))
}

/// Another edition than the linked one, and one the file really is: its length agrees (within about 4 %, not just
/// "unknown") and it is sure by the usual rules. Without a length to compare nothing is corrected by itself.
fn retarget_ok(m: &Value, old_asin: &str) -> bool {
    let new = s(m, "asin");
    !new.is_empty()
        && !new.eq_ignore_ascii_case(old_asin)
        && m["match"]["duration"].as_f64().is_some_and(|d| d >= 0.9)
        && sure(m)
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
        // not sure: the best candidate is shown, the admin decides whether to take it over
        let proposal = proposal_of(&ranked_for(app, &entry, &book, &file));
        return Ok(json!({"status": "unsure", "changes": [], "proposal": proposal}));
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
    let book = media::read_book(&file);
    let Some(m) = candidate(app, &entry, &file, &book, auto)? else {
        return Ok(Outcome::Unsure);
    };
    let (added, book) = fill_from(app, rel, &entry, &folder, &file, book, &m, false)?;
    if added == 0 {
        return Ok(Outcome::Complete {
            source: s(&m, "source").to_string(),
        });
    }
    Ok(Outcome::Added {
        n: added,
        source: s(&m, "source").to_string(),
        asin: !s(&book, "asin").is_empty(),
        title: s(&m, "title").to_string(),
    })
}

/// Write what the edition `m` has and the book lacks (and a missing cover) into the book's M4B and metadata.json:
/// (how many things were added, the book as written). `take_asin`: the admin chose this edition – its ASIN replaces
/// the one the book had (a wrong link); otherwise an ASIN is only filled in where there is none.
#[allow(clippy::too_many_arguments)]
fn fill_from(
    app: &Arc<App>,
    rel: &str,
    entry: &Value,
    folder: &Path,
    file: &Path,
    mut book: Value,
    m: &Value,
    take_asin: bool,
) -> Result<(usize, Value), String> {
    let mut added = 0;
    if take_asin && !s(m, "asin").is_empty() && !s(&book, "asin").eq_ignore_ascii_case(s(m, "asin")) {
        book["asin"] = json!(s(m, "asin"));
        added += 1;
    }
    for (key, _, new) in gaps(&book, m) {
        book[key] = new;
        added += 1;
    }
    let mut cover: Option<Vec<u8>> = None;
    if entry["cover"] != true
        && !s(m, "cover_url").is_empty()
        && let Ok(Some(c)) = audible::fetch_image(s(m, "cover_url"))
    {
        cover = Some(c);
        added += 1;
    }
    if added == 0 {
        return Ok((0, book));
    }
    let settings = Value::Object(app.settings());
    // the tags are rewritten in place: same file, same place in the library; the picture stays when there is no new one
    tagger::write_tags(file, &book, &settings, cover.clone())?;
    if let Some(c) = &cover {
        let settings = app.settings();
        let _ = tagger::write_sidecars(folder, &book, &settings, Some(c));
    }
    if folder.join(tagger::METADATA_FILE).is_file() {
        let dur = media::probe_cached(file)["duration"].as_f64().unwrap_or(0.0);
        let _ = tagger::write_abs_metadata(folder, &book, &encoder::file_chapters(file), dur);
    }
    app.library.refresh(rel);
    app.health.recheck(app, rel);
    Ok((added, book))
}

/// All editions the lookup finds for a book, best first (also the ones below any threshold).
fn ranked_for(app: &App, entry: &Value, book: &Value, file: &Path) -> Vec<Value> {
    let region = app
        .settings()
        .get("ab_region")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
        .unwrap_or("de")
        .to_string();
    let dur = media::probe_cached(file)["duration"].as_f64().unwrap_or(0.0);
    app.providers.lookup(&lookup_hints(entry, book, dur), &region, true).0
}

/// The id a candidate is picked by: its ASIN, else the source's own id ("dnb:…", "apple:…").
fn pick_id(m: &Value) -> String {
    let asin = s(m, "asin");
    if asin.is_empty() {
        s(m, "id").to_string()
    } else {
        asin.to_string()
    }
}

/// The best candidate for a book Hörwerk is not sure about – shown to the admin to take over or not (only when it
/// is at least a plausible match).
fn proposal_of(ranked: &[Value]) -> Option<Value> {
    let m = ranked.first()?;
    let score = m["match"]["score"].as_f64().unwrap_or(0.0);
    (score >= PROPOSAL_MIN && !pick_id(m).is_empty()).then(|| {
        json!({"id": pick_id(m), "title": m["title"], "authors": m["authors"], "narrators": m["narrators"],
               "source": m["source"], "asin": m["asin"], "score": (score * 100.0).round(), "runtime_min": m["runtime_min"],
               "release_date": m["release_date"], "cover_url": m["cover_url"]})
    })
}

/// Below this a candidate is not worth showing at all.
const PROPOSAL_MIN: f64 = 0.5;

/// The admin took over a proposed edition (`pick` = its ASIN or id): it is written into the book like a sure match,
/// its ASIN replacing a wrong one. Returns how many things were added.
pub fn apply_pick(app: &Arc<App>, rel: &str, pick: &str) -> Result<usize, String> {
    let entry = app.library.entry(rel).ok_or_else(|| t("Nicht gefunden", &[]))?;
    let folder = app.cfg.library.join(rel);
    let audio = media::collect_audio(&folder);
    let [name] = audio.as_slice() else {
        return Err(t("Daten ergänzen geht nur bei einer einzelnen M4B", &[]));
    };
    if !name.to_lowercase().ends_with(".m4b") {
        return Err(t("Daten ergänzen geht nur bei einer einzelnen M4B", &[]));
    }
    let file = folder.join(name);
    let book = media::read_book(&file);
    let ranked = ranked_for(app, &entry, &book, &file);
    let m = ranked
        .iter()
        .find(|m| pick_id(m).eq_ignore_ascii_case(pick))
        .cloned()
        .ok_or_else(|| t("Der Vorschlag ist nicht mehr zu finden – bitte neu prüfen", &[]))?;
    let (added, _) = fill_from(app, rel, &entry, &folder, &file, book, &m, true)?;
    if added > 0 {
        crate::mediaservers::request_scans(app);
    }
    Ok(added)
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
    fn a_link_is_only_corrected_when_the_length_agrees() {
        let m = |asin: &str, title: f64, duration: Value| json!({"asin": asin, "match": {"title": title, "author": 0.95, "duration": duration}});
        // another edition whose length fits the file: corrected
        assert!(retarget_ok(&m("B0NEW00001", 0.9, json!(0.97)), "B0OLD00001"));
        // the same edition, a length that does not fit, or no length at all: left alone
        assert!(!retarget_ok(&m("B0OLD00001", 0.9, json!(0.97)), "b0old00001"));
        assert!(!retarget_ok(&m("B0NEW00001", 0.9, json!(0.6)), "B0OLD00001"));
        assert!(!retarget_ok(&m("B0NEW00001", 0.99, Value::Null), "B0OLD00001"));
        assert!(!retarget_ok(&m("", 0.9, json!(0.97)), "B0OLD00001"));
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
