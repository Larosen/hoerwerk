//! Tags and sidecar files of a filed book (with metadata.json for Audiobookshelf): the tags themselves are hw-tags.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::encoder::Chapter;
use crate::rx;
use crate::text::t;

pub const METADATA_FILE: &str = "metadata.json";
const MANUAL_FILES: [&str; 3] = ["metadata.yml", "metadata.yaml", "metadata.json"];

pub fn write_tags(path: &Path, book: &Value, settings: &Value, cover: Option<Vec<u8>>) -> Result<(), String> {
    hw_tags::write(path, book, settings, cover).map_err(|e| e.to_string())
}

pub fn copy_tags(src: &Path, dst: &Path) -> Result<(), String> {
    hw_tags::copy_tags(src, dst).map_err(|e| e.to_string())
}

pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
    hw_tags::embedded_cover(path)
}

pub fn read_book(path: &Path) -> Value {
    hw_tags::read_book(path)
}

pub fn clean_title(book: &Value, settings: &Value) -> String {
    hw_tags::clean_title(book, settings)
}

fn png(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// desc.txt, reader.txt, cover.jpg – what Audiobookshelf/Booksonic pick up.
pub fn write_sidecars(
    folder: &Path,
    book: &Value,
    settings: &Map<String, Value>,
    cover: Option<&[u8]>,
) -> std::io::Result<Vec<String>> {
    let on = |k: &str, d: bool| settings.get(k).and_then(Value::as_bool).unwrap_or(d);
    let mut written = Vec::new();
    if on("write_desc", false)
        && let Some(d) = book["description"].as_str().filter(|d| !d.is_empty())
    {
        std::fs::write(folder.join("desc.txt"), format!("{d}\n"))?;
        written.push("desc.txt".to_string());
    }
    let narrators: Vec<&str> = book["narrators"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if on("write_reader", false) && !narrators.is_empty() {
        std::fs::write(folder.join("reader.txt"), format!("{}\n", narrators.join(", ")))?;
        written.push("reader.txt".to_string());
    }
    if on("write_cover", true)
        && let Some(c) = cover.filter(|c| !c.is_empty())
    {
        let name = if png(c) { "cover.png" } else { "cover.jpg" };
        std::fs::write(folder.join(name), c)?;
        written.push(name.to_string());
    }
    Ok(written)
}

/// Python's round(x, 3)
fn round3(x: f64) -> f64 {
    (x * 1000.0).round_ties_even() / 1000.0
}

fn year(date: &str) -> Value {
    rx::search(rx!(r"^(\d{4})"), date)
        .map(|c| json!(&c[1]))
        .unwrap_or(Value::Null)
}

fn or_null(v: &Value) -> Value {
    match v {
        Value::Null | Value::Bool(false) => Value::Null,
        Value::String(s) if s.is_empty() => Value::Null,
        other => other.clone(),
    }
}

fn list(v: &Value) -> Value {
    match v {
        Value::Array(a) => json!(a),
        _ => json!([]),
    }
}

/// Audiobookshelf's metadata.json for a book (chapters with start/end in seconds).
pub fn abs_metadata(book: &Value, chapters: &[Chapter], duration: f64) -> Value {
    let mut series = Vec::new();
    if let Some(s) = book["series"].as_str().filter(|s| !s.is_empty()) {
        series.push(match &book["series_position"] {
            Value::Null => s.to_string(),
            Value::String(p) if p.is_empty() => s.to_string(),
            Value::String(p) => format!("{s} #{p}"),
            other => format!("{s} #{other}"),
        });
    }
    let chs: Vec<Value> = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let end = chapters
                .get(i + 1)
                .map_or(if duration != 0.0 { duration } else { c.start }, |n| n.start);
            let title = if c.title.is_empty() {
                t("Kapitel {n}", &[("n", &(i + 1).to_string())])
            } else {
                c.title.clone()
            };
            json!({"id": i, "start": round3(c.start), "end": round3(end), "title": title})
        })
        .collect();
    let date = book["release_date"].as_str().unwrap_or("");
    json!({
        "tags": list(&book["tags"]), "chapters": chs, "title": book["title"].as_str().unwrap_or(""),
        "subtitle": or_null(&book["subtitle"]), "authors": list(&book["authors"]), "narrators": list(&book["narrators"]),
        "series": series, "genres": list(&book["genres"]), "publishedYear": year(date),
        "publishedDate": or_null(&book["release_date"]), "publisher": or_null(&book["publisher"]),
        "description": or_null(&book["description"]), "isbn": or_null(&book["isbn"]), "asin": or_null(&book["asin"]),
        "language": or_null(&book["language"]), "explicit": book["is_adult"].as_bool().unwrap_or(false),
        "abridged": book["format_type"].as_str().unwrap_or("").to_lowercase() == "abridged",
    })
}

/// The chapter names of a book changed in its M4B: the `metadata.json` beside it (which Audiobookshelf reads
/// before the file's own chapters) gets the same names – its other fields and the chapter times stay.
/// Ok(false): there is no such file, or its chapters don't line up with the file's.
pub fn rename_abs_chapters(folder: &Path, chapters: &[Chapter]) -> std::io::Result<bool> {
    let path = folder.join(METADATA_FILE);
    if !crate::media::is_regular(&path) {
        return Ok(false);
    }
    let Ok(mut data) = serde_json::from_str::<Value>(&std::fs::read_to_string(&path)?) else {
        return Ok(false);
    };
    let Some(list) = data["chapters"].as_array_mut().filter(|l| l.len() == chapters.len()) else {
        return Ok(false);
    };
    for (c, new) in list.iter_mut().zip(chapters) {
        c["title"] = json!(new.title);
    }
    let tmp = folder.join(format!(".{METADATA_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_string_pretty(&data).unwrap_or_default() + "\n")?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

pub fn write_abs_metadata(folder: &Path, book: &Value, chapters: &[Chapter], duration: f64) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(&abs_metadata(book, chapters, duration)).unwrap_or_default();
    std::fs::write(folder.join(METADATA_FILE), text + "\n")
}

fn yaml_to_json(y: &yaml_rust2::Yaml) -> Value {
    use yaml_rust2::Yaml;
    match y {
        Yaml::String(s) => json!(s),
        Yaml::Integer(i) => json!(i),
        Yaml::Real(r) => r.parse::<f64>().map(|f| json!(f)).unwrap_or(json!(r)),
        Yaml::Boolean(b) => json!(b),
        Yaml::Array(a) => Value::Array(a.iter().map(yaml_to_json).collect()),
        Yaml::Hash(h) => Value::Object(
            h.iter()
                .map(|(k, v)| {
                    (
                        match k {
                            Yaml::String(s) => s.clone(),
                            Yaml::Integer(i) => i.to_string(),
                            other => format!("{other:?}"),
                        },
                        yaml_to_json(v),
                    )
                })
                .collect(),
        ),
        _ => Value::Null,
    }
}

/// YAML without anchors/aliases ("billion laughs" memory bombs) – a metadata file needs none.
fn parse_yaml(text: &str, name: &str) -> Result<Value, String> {
    use yaml_rust2::parser::{Event, EventReceiver, Parser};
    struct Check(bool);
    impl EventReceiver for Check {
        fn on_event(&mut self, ev: Event) {
            let anchored = match ev {
                Event::Alias(_) => true,
                Event::Scalar(_, _, a, _) | Event::SequenceStart(a, _) | Event::MappingStart(a, _) => a != 0,
                _ => false,
            };
            self.0 |= anchored;
        }
    }
    let mut check = Check(false);
    Parser::new_from_str(text)
        .load(&mut check, true)
        .map_err(|e| format!("{name}: {e}"))?;
    if check.0 {
        return Err(t(
            "{file}: YAML-Anker (& / *) werden aus Sicherheitsgründen nicht unterstützt",
            &[("file", name)],
        ));
    }
    let docs = yaml_rust2::YamlLoader::load_from_str(text).map_err(|e| format!("{name}: {e}"))?;
    Ok(docs.first().map(yaml_to_json).unwrap_or(Value::Null))
}

fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
        && v != &json!("")
        && v != &json!([])
        && v != &json!({})
        && v != &json!(0)
}

/// beets-audible compatible `metadata.yml` (or the same keys as JSON) in a download folder.
pub fn load_manual(folder: &Path) -> Result<Option<Value>, String> {
    for name in MANUAL_FILES {
        let path = folder.join(name);
        if !crate::media::is_regular(&path) {
            continue;
        }
        if std::fs::metadata(&path).map_or(0, |m| m.len()) > 512 * 1024 {
            return Err(t("{file} ist zu groß", &[("file", name)]));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let data = if name.ends_with(".json") {
            serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?
        } else {
            parse_yaml(&text, name)?
        };
        if !data.is_object() || !truthy(&data["title"]) {
            return Err(t("{file}: Feld „title“ fehlt", &[("file", name)]));
        }
        let lst = |v: &Value| -> Vec<String> {
            match v {
                v if !truthy(v) => Vec::new(),
                Value::Array(a) => a.iter().map(py_str).collect(),
                other => py_str(other).split(',').map(|s| s.trim().to_string()).collect(),
            }
        };
        let first = |keys: &[&str]| {
            keys.iter()
                .map(|k| &data[*k])
                .find(|v| truthy(v))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let rd = first(&["releaseDate", "release_date", "publishedDate", "publishedYear"]);
        let mut series = data["series"].clone();
        let mut position = if data.get("seriesPosition").is_some() {
            data["seriesPosition"].clone()
        } else {
            data["series_position"].clone()
        };
        if let Value::Array(a) = &series {
            // Audiobookshelf metadata.json: ["Serie #3", …]
            let first = a.first().map(py_str).unwrap_or_default();
            match rx::search(rx!(r"^(.*?)\s*#\s*([\d.,]+)\s*$"), &first) {
                Some(m) => {
                    series = json!(&m[1]);
                    position = json!(&m[2]);
                }
                None => series = if first.is_empty() { Value::Null } else { json!(first) },
            }
        }
        let opt = |v: &Value| if v.is_null() { Value::Null } else { v.clone() };
        return Ok(Some(json!({
            "asin": opt(&data["asin"]), "title": py_str(&data["title"]), "subtitle": opt(&data["subtitle"]),
            "authors": lst(&first(&["authors", "author"])), "narrators": lst(&first(&["narrators", "narrator"])),
            "series": if truthy(&series) { series } else { Value::Null },
            "series_position": if position.is_null() { Value::Null } else { json!(py_str(&position)) },
            "publisher": opt(&data["publisher"]),
            "release_date": if truthy(&rd) { json!(py_str(&rd).chars().take(10).collect::<String>()) } else { Value::Null },
            "language": if data.get("language").is_some() { data["language"].clone() } else { json!("German") },
            "genres": lst(&data["genres"]),
            "description": data["description"].as_str().unwrap_or("").trim(), "source": name,
        })));
    }
    Ok(None)
}
