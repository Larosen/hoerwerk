//! Which recording of a book is this? The same title often exists several times: a reading by one narrator,
//! a radio play, a new production with a full cast. A release or a downloaded folder that clearly is another
//! recording than the requested one must not be taken – while missing information never blocks anything
//! (better to ask once too often than to load twice).
//!
//! Signals, each a veto only when it clearly contradicts:
//! - production: radio play / full cast on one side, a reading on the other
//! - narrator: the release names who reads it ("gesprochen von Rufus Beck"), and that person is not among
//!   the book's narrators (surnames compared, also with small spelling differences – Jaro-Winkler)
//! - time: a release posted (or naming a year) before a new production came out can't be that production

use serde_json::Value;

use crate::quality;
use crate::rx;
use crate::rx::{findall, is_match, sub};
use crate::text::t;
use crate::words::match_words;

/// Grace for the posting date: pre-releases and time zones (seconds).
const EARLY_GRACE: f64 = 30.0 * 86400.0;
/// From this similarity on two surnames count as the same person ("Bruehl" / "Brühl", "Stadtlober").
const SAME_NAME: f64 = 0.92;

fn plain(text: &str) -> String {
    sub(rx!(r"[._]+"), text, " ")
}

/// "mit voller Besetzung", "Full Cast", "Hörspiel", "dramatized" … in a title, series or release name.
pub fn production_marked(text: &str) -> bool {
    is_match(
        rx!(
            r"(?i)volle[rn]?\s+besetzung|full[\s-]?cast|h(?:ö|oe)rspiel|dramati[sz]|radio\s?(?:play|drama)|audio\s?drama|(?:szenische|inszenierte)\s+lesung"
        ),
        &plain(text),
    )
}

/// "Lesung", "gelesen von", "read by" … – a reading by one (or a few) narrators.
fn reading_marked(text: &str) -> bool {
    is_match(
        rx!(r"(?i)\blesung\b|\b(?:gelesen|gesprochen|vorgelesen)\s+von\b|\bliest\b|\b(?:read|narrated)\s+by\b"),
        &plain(text),
    )
}

/// Words after "gesprochen von", "read by" … that are not release jargon: the people who read it.
pub fn named_readers(text: &str) -> Vec<String> {
    const JARGON: [&str; 26] = [
        "german",
        "deutsch",
        "english",
        "englisch",
        "audiobook",
        "audiobooks",
        "hoerbuch",
        "horbuch",
        "hörbuch",
        "ungekuerzt",
        "ungekurzt",
        "unabridged",
        "abridged",
        "gekuerzt",
        "retail",
        "web",
        "m4b",
        "mp3",
        "flac",
        "aac",
        "und",
        "and",
        "u",
        "a",
        "dl",
        "cd",
    ];
    let text = plain(text);
    let mut out = Vec::new();
    for m in findall(
        rx!(r"(?i)\b(?:(?:gelesen|gesprochen|vorgelesen)\s+von|(?:read|narrated)\s+by)\s+([^\-–()\[\]]{2,80})"),
        &text,
    ) {
        for w in m.split(|c: char| c.is_whitespace() || c == ',' || c == '&' || c == '/') {
            let w = w.trim();
            if w.is_empty() || is_match(rx!(r"^\d"), w) {
                continue;
            }
            let lw = w.to_lowercase();
            if JARGON.contains(&lw.as_str()) || lw.chars().count() < 2 {
                break; // the names end where the release jargon starts
            }
            if !w.chars().next().is_some_and(char::is_uppercase) {
                break;
            }
            out.push(w.to_string());
        }
    }
    out
}

/// Jaro-Winkler similarity of two words (0 … 1).
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut used = vec![false; b.len()];
    let mut ma = Vec::new();
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        if let Some(j) = (lo..hi).find(|&j| !used[j] && b[j] == *ca) {
            used[j] = true;
            ma.push(*ca);
        }
    }
    if ma.is_empty() {
        return 0.0;
    }
    let mb: Vec<char> = b.iter().zip(&used).filter(|(_, u)| **u).map(|(c, _)| *c).collect();
    let m = ma.len() as f64;
    let transpositions = ma.iter().zip(&mb).filter(|(x, y)| x != y).count() as f64 / 2.0;
    let jaro = (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions) / m) / 3.0;
    let prefix = a.iter().zip(&b).take(4).take_while(|(x, y)| x == y).count() as f64;
    jaro + prefix * 0.1 * (1.0 - jaro)
}

/// The surnames of a list of names ("R. Beck", "Daniel Brühl") – folded like match_words.
fn surnames(names: &[String]) -> Vec<String> {
    const NOT_A_NAME: [&str; 8] = ["cast", "besetzung", "diverse", "various", "sprecher", "div", "u", "a"];
    names
        .iter()
        .filter_map(|n| match_words(n).into_iter().rev().find(|w| w.chars().count() >= 2))
        .filter(|w| !NOT_A_NAME.contains(&w.as_str()))
        .collect()
}

/// Do two lists of people share someone? None when one of them is unknown.
pub fn same_people(a: &[String], b: &[String]) -> Option<bool> {
    let (sa, sb) = (surnames(a), surnames(b));
    if sa.is_empty() || sb.is_empty() {
        return None;
    }
    Some(
        sa.iter()
            .any(|x| sb.iter().any(|y| x == y || jaro_winkler(x, y) >= SAME_NAME)),
    )
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|x| !x.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn text_of(book: &Value, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| book[*k].as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A new production: a radio play / full cast, or announced as a new recording. Only for such a book a
/// release from before its release date is surely another recording – a plain reading may well have been
/// on CD years before the shop lists it.
fn new_production(book: &Value) -> bool {
    quality::is_drama(book, "")
        || production_marked(&text_of(book, &["title", "subtitle", "series"]))
        || is_match(
            rx!(
                r"(?i)neu\s*(?:eingelesen|aufnahme|produktion|ausgabe)|neue\s+fassung|new\s+(?:recording|production|edition)"
            ),
            &text_of(book, &["title", "subtitle"]),
        )
}

/// Unix time of a "YYYY-MM-DD" (or "YYYY") release date.
fn day_of(date: &str) -> Option<f64> {
    let d: Vec<i64> = date
        .get(..10.min(date.len()))?
        .split('-')
        .filter_map(|x| x.parse().ok())
        .collect();
    let (y, m, dd) = match d.as_slice() {
        [y, m, dd] => (*y, *m, *dd),
        [y] => (*y, 1, 1),
        _ => return None,
    };
    if !(1900..=2200).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&dd) {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(((era * 146_097 + doe - 719_468) * 86400) as f64)
}

fn release_year(book: &Value) -> Option<i64> {
    book["release_date"]
        .as_str()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse().ok())
        .filter(|y| (1900..=2200).contains(y))
}

fn short_list(names: &[String]) -> String {
    let mut list: Vec<String> = names.iter().take(3).cloned().collect();
    if names.len() > 3 {
        list.push("…".into());
    }
    list.join(", ")
}

/// Production and narrator: what a name (release or folder) says against the book.
fn people_and_production(name: &str, book: &Value) -> Option<String> {
    let drama = quality::is_drama(book, "") || production_marked(&text_of(book, &["title", "subtitle", "series"]));
    let marked = production_marked(name);
    if drama && !marked && reading_marked(name) {
        return Some(t(
            "Lesung – gesucht ist das Hörspiel bzw. die Fassung mit voller Besetzung",
            &[],
        ));
    }
    let narrators = strs(&book["narrators"]);
    if !drama && marked && (1..=3).contains(&narrators.len()) {
        return Some(t(
            "Hörspiel bzw. volle Besetzung – gesucht ist die Lesung von {names}",
            &[("names", &short_list(&narrators))],
        ));
    }
    let named = named_readers(name);
    if same_people(&named, &narrators) == Some(false) {
        return Some(t(
            "Gelesen von {who} – das Hörbuch liest {names}",
            &[("who", &named.join(" ")), ("names", &short_list(&narrators))],
        ));
    }
    None
}

/// Before the download: is this release clearly another recording than the book? (reason) – else None.
pub fn release_conflict(name: &str, published: f64, book: &Value) -> Option<String> {
    if let Some(why) = people_and_production(name, book) {
        return Some(why);
    }
    if !new_production(book) {
        return None;
    }
    let released = book["release_date"].as_str().unwrap_or("");
    if let Some(day) = day_of(released)
        && published > 0.0
        && published + EARLY_GRACE < day
    {
        return Some(t(
            "Älter als diese Fassung (Release von {posted}, die Fassung erschien {released})",
            &[
                ("posted", &crate::text::local_date(published)),
                ("released", &released.chars().take(10).collect::<String>()),
            ],
        ));
    }
    if let Some(year) = release_year(book) {
        let in_title: Vec<String> = findall(
            rx!(r"(?<!\d)((?:19|20)\d\d)(?!\d)"),
            &text_of(book, &["title", "subtitle"]),
        );
        let named = findall(rx!(r"(?<!\d)((?:19|20)\d\d)(?!\d)"), name)
            .into_iter()
            .filter(|y| !in_title.contains(y))
            .filter_map(|y| y.parse::<i64>().ok())
            .max();
        if let Some(n) = named.filter(|n| *n < year) {
            return Some(t(
                "Jahr {named} im Namen – diese Fassung erschien {year}",
                &[("named", &n.to_string()), ("year", &year.to_string())],
            ));
        }
    }
    None
}

/// After the download: a folder name that clearly is another recording (reason) – else None.
pub fn content_conflict(name: &str, book: &Value) -> Option<String> {
    people_and_production(name, book)
}

/// Uncertain without ruling it out: the file's narrator tag names someone else (the tag is often the author,
/// a composer or a label – so it is a reason to look, not a veto).
pub fn narrator_doubt(tag_narrators: &[String], book: &Value) -> Option<String> {
    let narrators = strs(&book["narrators"]);
    // many files carry the author in the narrator or composer tag: that says nothing about the recording
    let authors = strs(&book["authors"]);
    let tag_narrators: Vec<String> = tag_narrators
        .iter()
        .filter(|n| same_people(std::slice::from_ref(n), &authors) != Some(true))
        .cloned()
        .collect();
    (same_people(&tag_narrators, &narrators) == Some(false)).then(|| {
        t(
            "Sprecher in der Datei: {who} – das Hörbuch liest {names}",
            &[("who", &short_list(&tag_narrators)), ("names", &short_list(&narrators))],
        )
    })
}

/// Names from a narrator tag ("Rufus Beck", "A und B", "A; B").
pub fn tag_names(tag: &str) -> Vec<String> {
    rx::split(rx!(r"\s*(?:[,;/&]|\bund\b|\band\b)\s*"), tag)
        .into_iter()
        .map(|x| x.trim().to_string())
        .filter(|x| x.chars().count() >= 3)
        .collect()
}

// -- other recordings of the same work ("Fassungen") -------------------------------------------------------------
// A title often exists several times: other narrators, other years, abridged or not, reading or radio play. Knowing
// them is what tells a release or a download of another recording apart from the one asked for.

/// From shop hits (Audible, BookBeat …): the other recordings of the same work by the same author – not the book
/// itself. Each {asin, title, narrators, year, runtime_min, format_type, source}; one per narrator and year.
pub fn siblings(book: &Value, hits: &[Value]) -> Vec<Value> {
    let words = |b: &Value| crate::releases::title_words(b["title"].as_str().unwrap_or(""));
    let want = words(book);
    if want.is_empty() {
        return Vec::new();
    }
    let author = surnames(&strs(&book["authors"]));
    let own_asin = book["asin"].as_str().unwrap_or("").to_uppercase();
    let own_people = surnames(&strs(&book["narrators"]));
    let mut out: Vec<Value> = Vec::new();
    for h in hits {
        let theirs = words(h);
        // the same work: every word of the title is there, and not much more (not "Odyssee – Kommentar zu …")
        let same_title = want.iter().all(|w| theirs.contains(w)) && theirs.len() <= want.len() + 2;
        let same_author = author.is_empty() || same_people(&strs(&h["authors"]), &strs(&book["authors"])) == Some(true);
        let asin = h["asin"].as_str().unwrap_or("").to_uppercase();
        if !same_title || !same_author || (!asin.is_empty() && asin == own_asin) {
            continue;
        }
        let people = surnames(&strs(&h["narrators"]));
        let year = release_year(h);
        // the requested recording listed again (another shop, another region): not another one
        if !people.is_empty() && people == own_people && year == release_year(book) {
            continue;
        }
        let key = (people.clone(), year);
        if out
            .iter()
            .any(|o| (surnames(&strs(&o["narrators"])), o["year"].as_i64()) == key)
        {
            continue;
        }
        out.push(serde_json::json!({"asin": h["asin"], "title": h["title"], "narrators": h["narrators"], "year": year,
                                     "runtime_min": h["runtime_min"], "format_type": h["format_type"], "source": h["source"]}));
    }
    out
}

fn who(edition: &Value) -> String {
    let names = strs(&edition["narrators"]);
    let year = edition["year"].as_i64().map(|y| y.to_string()).unwrap_or_default();
    match (names.is_empty(), year.is_empty()) {
        (false, false) => format!("{} ({year})", short_list(&names)),
        (false, true) => short_list(&names),
        (true, false) => year,
        (true, true) => String::new(),
    }
}

/// A release against the other recordings: (reason it is another one, or None; note when it names the book's own
/// narrator). Only clear signals count – a narrator's surname or a year that belongs to another recording only.
pub fn sibling_verdict(name: &str, book: &Value, others: &[Value]) -> (Option<String>, Option<String>) {
    let words: Vec<String> = match_words(&plain(name));
    let own = strs(&book["narrators"]);
    let own_names = surnames(&own);
    let named = |n: &str| {
        words
            .iter()
            .any(|w| w == n || (n.chars().count() >= 5 && jaro_winkler(w, n) >= SAME_NAME))
    };
    if let Some(n) = own.iter().find(|n| {
        surnames(std::slice::from_ref(n))
            .first()
            .is_some_and(|s| s.chars().count() >= 4 && named(s))
    }) {
        return (None, Some(t("Sprecher passt: {name}", &[("name", n)])));
    }
    for o in others {
        for (full, sur) in strs(&o["narrators"])
            .iter()
            .map(|n| (n.clone(), surnames(std::slice::from_ref(n))))
        {
            let Some(sur) = sur.first() else { continue };
            if sur.chars().count() >= 4 && !own_names.contains(sur) && named(sur) {
                return (
                    Some(
                        t("Andere Fassung: gelesen von {name}", &[("name", &full)])
                            + &o["year"].as_i64().map(|y| format!(" ({y})")).unwrap_or_default(),
                    ),
                    None,
                );
            }
        }
    }
    let own_year = release_year(book);
    let in_title: Vec<String> = findall(
        rx!(r"(?<!\d)((?:19|20)\d\d)(?!\d)"),
        &text_of(book, &["title", "subtitle"]),
    );
    for y in findall(rx!(r"(?<!\d)((?:19|20)\d\d)(?!\d)"), name)
        .into_iter()
        .filter(|y| !in_title.contains(y))
        .filter_map(|y| y.parse::<i64>().ok())
    {
        if Some(y) == own_year {
            break;
        }
        if let Some(o) = others.iter().find(|o| o["year"].as_i64() == Some(y)) {
            return (
                Some(t(
                    "Andere Fassung: {year} erschien die von {who}",
                    &[("year", &y.to_string()), ("who", &who(o))],
                )),
                None,
            );
        }
    }
    (None, None)
}

/// After the download: the length fits another recording clearly better than the requested one (reason) – else None.
pub fn runtime_sibling(duration_sec: f64, book: &Value, others: &[Value]) -> Option<String> {
    let mins = duration_sec / 60.0;
    let own = book["runtime_min"].as_f64().filter(|m| *m > 0.0)?;
    if mins <= 0.0 || (mins - own).abs() / own < 0.08 {
        return None;
    }
    others
        .iter()
        .filter_map(|o| {
            o["runtime_min"]
                .as_f64()
                .filter(|m| *m > 0.0)
                .map(|m| (o, (mins - m).abs() / m))
        })
        .filter(|(_, off)| *off < 0.04)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(o, _)| {
            t(
                "Die Länge passt zur Fassung von {who}, nicht zur angefragten",
                &[("who", &who(o))],
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn full_cast() -> Value {
        json!({"title": "Harry Potter und der Stein der Weisen (Hörbuch mit voller Besetzung)",
               "series": "Harry Potter (Hörbücher mit voller Besetzung)", "release_date": "2026-11-10",
               "authors": ["J.K. Rowling"],
               "narrators": ["Daniel Brühl", "Nina Hoss", "Abak Safaei-Rad", "Armin Rohde", "Robert Stadlober"]})
    }

    fn reading() -> Value {
        json!({"title": "Harry Potter und der Stein der Weisen", "series": "Harry Potter - Gesprochen von Rufus Beck",
               "release_date": "2016-01-01", "authors": ["J.K. Rowling"], "narrators": ["Rufus Beck"]})
    }

    const BECK: &str = "J.K.Rowling-Harry Potter und der Stein der Weisen Gesprochen von Rufus Beck-AudioBook-2016-M4B";
    const CAST: &str =
        "J.K.Rowling-Harry.Potter.und.der.Stein.der.Weisen.Hoerbuch.mit.voller.Besetzung.German.2026.M4B";

    #[test]
    fn names_and_similarity() {
        assert_eq!(named_readers(BECK), vec!["Rufus", "Beck"]);
        assert_eq!(
            named_readers("Ein.Buch.gelesen.von.Anna.Thalbach.German.MP3"),
            vec!["Anna", "Thalbach"]
        );
        assert_eq!(
            named_readers("Ein Buch - read by Stephen Fry (Unabridged)"),
            vec!["Stephen", "Fry"]
        );
        assert!(named_readers(CAST).is_empty());
        assert!(jaro_winkler("bruhl", "bruehl") > SAME_NAME);
        assert!(jaro_winkler("beck", "bruhl") < 0.6);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(same_people(&s(&["R. Beck"]), &s(&["Rufus Beck"])), Some(true));
        assert_eq!(
            same_people(&s(&["Rufus Beck"]), &s(&["Daniel Brühl", "Nina Hoss"])),
            Some(false)
        );
        assert_eq!(same_people(&s(&[]), &s(&["Rufus Beck"])), None);
        assert_eq!(
            tag_names("Daniel Brühl, Nina Hoss und Armin Rohde"),
            s(&["Daniel Brühl", "Nina Hoss", "Armin Rohde"])
        );
    }

    #[test]
    fn another_recording_is_refused_missing_information_is_not() {
        crate::text::set_language("de");
        let posted_2016 = day_of("2016-10-01").unwrap();
        let posted_now = day_of("2026-11-10").unwrap();
        // the full-cast production asked for: the 2016 reading is refused – for each of three reasons
        assert!(
            release_conflict(BECK, posted_now, &full_cast())
                .unwrap()
                .starts_with("Lesung")
        );
        let only_named = "J.K.Rowling-Harry.Potter.und.der.Stein.der.Weisen.read.by.Stephen.Fry.M4B";
        assert!(release_conflict(only_named, posted_now, &full_cast()).is_some());
        let only_old = "J.K.Rowling-Harry.Potter.und.der.Stein.der.Weisen.German.M4B";
        assert!(
            release_conflict(only_old, posted_2016, &full_cast())
                .unwrap()
                .starts_with("Älter")
        );
        let only_year = "J.K.Rowling-Harry.Potter.und.der.Stein.der.Weisen.German.2016.M4B";
        assert!(
            release_conflict(only_year, 0.0, &full_cast())
                .unwrap()
                .starts_with("Jahr 2016")
        );
        // the right release – and one that says nothing about the recording – are fine
        assert_eq!(release_conflict(CAST, posted_now, &full_cast()), None);
        assert_eq!(release_conflict(only_old, posted_now, &full_cast()), None);
        assert_eq!(release_conflict(only_old, 0.0, &full_cast()), None);

        // the reading asked for: the full cast is refused, the reading (also posted long ago) is fine
        assert!(
            release_conflict(CAST, posted_now, &reading())
                .unwrap()
                .starts_with("Hörspiel")
        );
        assert_eq!(release_conflict(BECK, posted_2016, &reading()), None);
        assert_eq!(
            release_conflict(only_old, day_of("2008-03-01").unwrap(), &reading()),
            None
        ); // CD long before the shop
        let other_reader = "J.K.Rowling-Harry.Potter.und.der.Stein.der.Weisen.gelesen.von.Anna.Thalbach.M4B";
        assert!(
            release_conflict(other_reader, posted_now, &reading())
                .unwrap()
                .starts_with("Gelesen von Anna Thalbach")
        );
        // a year that belongs to the title is no hint
        let orwell = json!({"title": "1984", "release_date": "2024-05-01", "narrators": ["A", "B", "C", "D"],
                            "subtitle": "Hörspiel"});
        assert_eq!(
            release_conflict("George.Orwell-1984.Hoerspiel.German.M4B", 0.0, &orwell),
            None
        );
    }

    #[test]
    fn after_the_download() {
        crate::text::set_language("de");
        assert!(content_conflict(BECK, &full_cast()).is_some());
        assert_eq!(content_conflict(CAST, &full_cast()), None);
        assert_eq!(content_conflict("Harry Potter 1", &full_cast()), None);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(narrator_doubt(&s(&["Rufus Beck"]), &full_cast()).is_some());
        assert_eq!(narrator_doubt(&s(&["Nina Hoss"]), &full_cast()), None);
        assert_eq!(narrator_doubt(&[], &full_cast()), None);
        assert_eq!(narrator_doubt(&s(&["J.K. Rowling"]), &full_cast()), None); // the author in the tag
    }
    #[test]
    fn other_recordings_tell_releases_and_downloads_apart() {
        let book = json!({"asin": "B00CU7MW7U", "title": "Odyssee", "authors": ["Homer"], "narrators": ["Manfred Schradi"],
                          "release_date": "2013-05-01", "runtime_min": 735});
        let hits = vec![
            json!({"asin": "B00CU7MW7U", "title": "Odyssee", "authors": ["Homer"], "narrators": ["Manfred Schradi"], "release_date": "2013-05-01"}),
            json!({"asin": "B1", "title": "Odyssee", "authors": ["Homer"], "narrators": ["Christian Brückner"], "release_date": "2009-01-01", "runtime_min": 1014}),
            json!({"asin": "B2", "title": "Die Odyssee", "authors": ["Homer"], "narrators": ["Rolf Boysen"], "release_date": "1998-01-01", "runtime_min": 900}),
            json!({"asin": "B3", "title": "Odyssee", "authors": ["Anna Andere"], "narrators": ["X Y"], "release_date": "2020-01-01"}),
            json!({"asin": "B4", "title": "Homers Odyssee – ein Kommentar für Schulen und Studium", "authors": ["Homer"], "narrators": ["Z Z"]}),
        ];
        let others = siblings(&book, &hits);
        let asins: Vec<&str> = others.iter().map(|o| o["asin"].as_str().unwrap()).collect();
        assert_eq!(asins, ["B1", "B2"]); // not itself, not another author's book, not a book about it
        // releases
        let (no, _) = sibling_verdict(
            "Homer - Odyssee - gelesen von Christian Brueckner - MP3",
            &book,
            &others,
        );
        assert!(no.unwrap().contains("Brückner"));
        let (no, _) = sibling_verdict("Homer.-.Odyssee.DE.2009.ABook.MP3-PaZ", &book, &others);
        assert!(no.unwrap().contains("2009"));
        let (no, yes) = sibling_verdict("Homer - Odyssee (Manfred Schradi) 2013", &book, &others);
        assert!(no.is_none() && yes.unwrap().contains("Manfred Schradi"));
        assert_eq!(sibling_verdict("Homer - Odyssee - MP3", &book, &others), (None, None)); // nothing says: no veto
        // downloads: 16 h 54 is Brückner's length, not the 12 h 15 asked for
        assert!(
            runtime_sibling(1014.0 * 60.0, &book, &others)
                .unwrap()
                .contains("Brückner")
        );
        assert!(runtime_sibling(737.0 * 60.0, &book, &others).is_none());
        assert!(runtime_sibling(1300.0 * 60.0, &book, &others).is_none()); // fits nothing known: no claim
    }
}
