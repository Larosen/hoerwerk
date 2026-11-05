//! Audiobook tags for M4B files – atom for atom what Hörwerk's Python tagger (`app/tagger.py`) writes, so
//! Audiobookshelf, Plex (Audnexus agent), Prologue, BookPlayer & co. read everything.
//! The yardstick is `contract/golden/tags.json`; `contract/test_tags.py` compares both writers against it.

use mp4ameta::ident::FreeformIdentBorrowed;
use mp4ameta::{Data, DataIdent, Fourcc, Img, ImgFmt, MediaType, Tag};
use serde_json::Value;

pub const MEAN: &str = "com.apple.iTunes";

/// Atoms the tagger owns: removed before writing, so nothing of an earlier tagging survives.
pub const OWN_KEYS: &[&[u8; 4]] = &[
    b"\xa9nam", b"\xa9alb", b"\xa9ART", b"aART", b"\xa9wrt", b"\xa9gen", b"\xa9day", b"\xa9cmt", b"desc", b"ldes",
    b"\xa9grp", b"\xa9mvn", b"\xa9mvi", b"shwm", b"soal", b"sonm", b"soaa", b"soar", b"stik", b"trkn", b"disk",
    b"cprt", b"\xa9too", b"pgap", b"\xa9lyr", b"tmpo",
];

#[derive(Debug)]
pub enum Error {
    Tag(mp4ameta::Error),
    Book(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Tag(e) => write!(f, "{e}"),
            Error::Book(e) => write!(f, "{e}"),
        }
    }
}

impl From<mp4ameta::Error> for Error {
    fn from(e: mp4ameta::Error) -> Self {
        Error::Tag(e)
    }
}

fn text<'a>(book: &'a Value, key: &str) -> Option<&'a str> {
    book.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn list(book: &Value, key: &str) -> Vec<String> {
    match book.get(key) {
        Some(Value::Array(v)) => v
            .iter()
            .filter_map(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => vec![],
    }
}

/// The series position as text ("3", "1.5") – Audible sends it as a string, other sources as a number.
fn position(book: &Value) -> Option<String> {
    match book.get("series_position") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn setting(settings: &Value, key: &str, default: bool) -> bool {
    settings.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// The title without "Band 3" & co. at the end when the setting asks for it.
pub fn clean_title(book: &Value, settings: &Value) -> String {
    let title = text(book, "title").unwrap_or("").to_string();
    match position(book) {
        Some(pos) if !setting(settings, "keep_series_in_title", true) => {
            let rx = regex_lite::Regex::new(&format!(
                r"(?i),?\s*(?:book|band|teil|folge|vol\.?|volume|part)\s*{}\s*$",
                regex_lite::escape(&pos)
            ))
            .expect("valid pattern");
            let cut = rx.replace(&title, "").trim().to_string();
            if cut.is_empty() { title } else { cut }
        }
        _ => title,
    }
}

pub fn clean_subtitle(book: &Value, settings: &Value) -> Option<String> {
    let sub = text(book, "subtitle")?;
    let series = text(book, "series").unwrap_or("").to_lowercase();
    let marker = regex_lite::Regex::new(r"(?i)book|band|teil|folge|\d").expect("valid pattern");
    if !series.is_empty()
        && !setting(settings, "keep_series_in_subtitle", true)
        && sub.to_lowercase().contains(&series)
        && marker.is_match(sub)
    {
        return None;
    }
    Some(sub.to_string())
}

fn album_sort(book: &Value, title: &str, subtitle: Option<&str>) -> String {
    match (text(book, "series"), position(book)) {
        (Some(series), Some(pos)) => format!("{series} {pos} - {title}"),
        (Some(series), None) => format!("{series} - {title}"),
        _ => match subtitle {
            Some(sub) => format!("{title} - {sub}"),
            None => title.to_string(),
        },
    }
}

/// One tag to write: the atom and its values.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Text(Fourcc, String),
    Freeform(&'static str, String),
}

/// What the tagger writes for a book (besides media type, track/disc numbers, movement flags and the cover).
pub fn build(book: &Value, settings: &Value) -> (Vec<Item>, Extras) {
    let title = clean_title(book, settings);
    let subtitle = clean_subtitle(book, settings);
    let authors = {
        let a = list(book, "authors").join(", ");
        if a.is_empty() { "Unbekannt".to_string() } else { a }
    };
    let narrators = list(book, "narrators").join(", ");
    let artist = if setting(settings, "narrator_in_artist", false) && !narrators.is_empty() {
        format!("{authors}, {narrators}")
    } else {
        authors.clone()
    };
    let fc = |b: &[u8; 4]| Fourcc(*b);
    let mut items = vec![
        Item::Text(fc(b"\xa9nam"), title.clone()),
        Item::Text(fc(b"\xa9alb"), title.clone()),
        Item::Text(fc(b"\xa9ART"), artist),
        Item::Text(fc(b"aART"), authors),
        Item::Text(fc(b"\xa9too"), "Hörwerk".into()),
        Item::Text(fc(b"soal"), album_sort(book, &title, subtitle.as_deref())),
    ];
    if !narrators.is_empty() {
        items.push(Item::Text(fc(b"\xa9wrt"), narrators.clone()));
    }
    let genres = list(book, "genres");
    if !genres.is_empty() {
        items.push(Item::Text(fc(b"\xa9gen"), genres.join("/")));
    }
    if let Some(day) = text(book, "release_date") {
        items.push(Item::Text(fc(b"\xa9day"), day.into()));
    }
    if let Some(desc) = text(book, "description") {
        for atom in [b"\xa9cmt", b"desc", b"ldes"] {
            items.push(Item::Text(fc(atom), desc.into()));
        }
    }
    if let Some(publisher) = text(book, "publisher") {
        items.push(Item::Text(fc(b"cprt"), publisher.into()));
    }
    let mut extras = Extras::default();
    let series = text(book, "series");
    let pos = position(book);
    if let Some(series) = series {
        items.push(Item::Text(fc(b"\xa9mvn"), series.into()));
        extras.show_movement = true;
        if let Some(p) = &pos {
            items.push(Item::Text(fc(b"\xa9grp"), format!("{series}, Book #{p}")));
            if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
                extras.movement_index = p.parse().ok();
            }
        }
    }
    let free: [(&'static str, Option<String>); 11] = [
        ("ASIN", text(book, "asin").map(String::from)),
        ("AUDIBLE_ASIN", text(book, "asin").map(String::from)),
        ("SERIES", series.map(String::from)),
        ("SERIES-PART", pos),
        ("SUBTITLE", subtitle),
        ("PUBLISHER", text(book, "publisher").map(String::from)),
        ("LANGUAGE", text(book, "language").map(String::from)),
        ("NARRATOR", (!narrators.is_empty()).then_some(narrators)),
        ("WWWAUDIOFILE", text(book, "url").map(String::from)),
        ("REGION", text(book, "region").map(String::from)),
        ("RELEASETYPE", Some("audiobook".into())),
    ];
    items.extend(
        free.into_iter()
            .filter_map(|(k, v)| v.filter(|s| !s.is_empty()).map(|v| Item::Freeform(k, v))),
    );
    (items, extras)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Extras {
    pub show_movement: bool,
    pub movement_index: Option<u16>,
}

fn cover_format(data: &[u8]) -> ImgFmt {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        ImgFmt::Png
    } else {
        ImgFmt::Jpeg
    }
}

/// Replace the book's tags in ``path`` (chapters stay; the old cover stays unless a new one is given).
pub fn write(path: &std::path::Path, book: &Value, settings: &Value, cover: Option<Vec<u8>>) -> Result<(), Error> {
    if !book.is_object() {
        return Err(Error::Book("book must be a JSON object".into()));
    }
    let mut tag = Tag::read_from_path(path)?;
    let old_cover: Vec<_> = tag.artworks().map(|i| (i.fmt, i.data.to_vec())).collect();
    for key in OWN_KEYS {
        tag.remove_data_of(&Fourcc(**key));
    }
    let freeform: Vec<DataIdent> = tag
        .data()
        .filter(|(id, _)| matches!(id, DataIdent::Freeform { .. }))
        .map(|(id, _)| id.clone())
        .collect();
    for id in freeform {
        tag.remove_data_of(&id);
    }
    tag.remove_artworks();
    let (items, extras) = build(book, settings);
    for item in items {
        match item {
            Item::Text(fourcc, value) => tag.set_data(fourcc, Data::Utf8(value)),
            Item::Freeform(name, value) => {
                tag.set_data(FreeformIdentBorrowed::new_borrowed(MEAN, name), Data::Utf8(value))
            }
        }
    }
    tag.set_media_type(MediaType::AudioBook);
    tag.set_track(1, 1);
    tag.set_disc(1, 1);
    if extras.show_movement {
        tag.set_show_movement();
    }
    if let Some(i) = extras.movement_index {
        tag.set_movement_index(i);
    }
    match cover {
        Some(data) => tag.set_artwork(Img::new(cover_format(&data), data)),
        None => {
            for (fmt, data) in old_cover {
                tag.add_artwork(Img::new(fmt, data));
            }
        }
    }
    tag.write_to_path(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn texts(items: &[Item]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|i| match i {
                Item::Text(f, v) => (String::from_utf8_lossy(&f.0).into_owned(), v.clone()),
                Item::Freeform(k, v) => (format!("----:{MEAN}:{k}"), v.clone()),
            })
            .collect()
    }

    #[test]
    fn series_and_freeform_atoms() {
        let book = json!({"title": "Der Schimmelreiter", "authors": ["Theodor Storm"], "series": "Novellen",
                          "series_position": "3", "asin": "B0X", "narrators": ["Eva"]});
        let (items, extras) = build(&book, &json!({}));
        let t = texts(&items);
        assert!(t.contains(&("soal".into(), "Novellen 3 - Der Schimmelreiter".into())));
        assert!(t.contains(&("----:com.apple.iTunes:SERIES-PART".into(), "3".into())));
        assert!(t.contains(&("----:com.apple.iTunes:NARRATOR".into(), "Eva".into())));
        assert_eq!(
            extras,
            Extras {
                show_movement: true,
                movement_index: Some(3)
            }
        );
    }

    #[test]
    fn unknown_author_and_numeric_position() {
        let (items, extras) = build(
            &json!({"title": "X", "series": "S", "series_position": 1.5}),
            &json!({}),
        );
        assert!(texts(&items).contains(&("aART".into(), "Unbekannt".into())));
        assert_eq!(extras.movement_index, None); // "1.5" is no whole number
    }

    #[test]
    fn title_cleaning_follows_the_settings() {
        let book = json!({"title": "Die Reise, Band 2", "series_position": "2"});
        assert_eq!(clean_title(&book, &json!({})), "Die Reise, Band 2");
        assert_eq!(clean_title(&book, &json!({"keep_series_in_title": false})), "Die Reise");
        let book = json!({"subtitle": "Die Reise-Saga 2", "series": "Die Reise-Saga"});
        assert_eq!(clean_subtitle(&book, &json!({"keep_series_in_subtitle": false})), None);
        assert_eq!(clean_subtitle(&book, &json!({})).as_deref(), Some("Die Reise-Saga 2"));
    }
}
