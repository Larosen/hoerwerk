//! Where a finished audiobook goes (library.py): path templates with optional parts, existing folders under
//! another spelling, atomic moves. Template: `{field}`, `{field:2}` (zero-padded numbers), `[ … ]` optional.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::rx;
use crate::text::t;

pub const DEFAULT_FOLDER: &str = "{author}/[{series}/][{series_position:2} - ]{title}";
pub const DEFAULT_FILE: &str = "{title}";
const PARTIAL: &str = ".hw-part-";

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn fields(book: &Value, title: Option<&str>) -> HashMap<&'static str, String> {
    let authors = strs(&book["authors"]);
    let narrators = strs(&book["narrators"]);
    let or = |s: String, d: String| if s.is_empty() { d } else { s };
    let title = title
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| or(text(&book["title"]), t("Unbenannt", &[])));
    HashMap::from([
        (
            "author",
            authors.first().cloned().unwrap_or_else(|| t("Unbekannt", &[])),
        ),
        ("authors", or(authors.join(", "), t("Unbekannt", &[]))),
        ("narrator", narrators.first().cloned().unwrap_or_default()),
        ("narrators", narrators.join(", ")),
        ("series", text(&book["series"])),
        ("series_position", text(&book["series_position"])),
        ("title", title),
        ("subtitle", text(&book["subtitle"])),
        ("year", text(&book["release_date"]).chars().take(4).collect()),
        ("asin", text(&book["asin"])),
        ("publisher", text(&book["publisher"])),
        ("language", text(&book["language"])),
        ("genre", strs(&book["genres"]).first().cloned().unwrap_or_default()),
        ("region", text(&book["region"])),
    ])
}

fn pad(value: &str, width: Option<&str>) -> String {
    let Some(w) = width
        .and_then(|w| w.parse::<usize>().ok())
        .filter(|_| !value.is_empty())
    else {
        return value.to_string();
    };
    rx!(r"(?<![.\d])\d+")
        .replace_all(value, |c: &fancy_regex::Captures| format!("{:0>w$}", &c[0]))
        .into_owned()
}

/// One path component, safe for SMB/Windows clients and Unraid shares.
pub fn sanitize(part: &str) -> String {
    let part = rx::sub(rx!(r#"[*?"<>|\x00-\x1f]"#), part, "");
    let part = rx::sub(rx!(r"\s*:\s*"), &part, " - ");
    let part = rx::sub(rx!(r"[\\/]"), &part, "-");
    let part = rx::sub(rx!(r"\s+"), &part, " ").replace(" - - ", " - ");
    let mut part = part.trim().trim_matches([' ', '.']).to_string();
    while part.len() > 180 {
        part.pop();
    }
    let part = part.trim_end_matches([' ', '.']).to_string();
    if part.is_empty() { "_".into() } else { part }
}

/// Expand placeholders; `[...]` blocks disappear if any field in them is empty.
pub fn render(template: &str, values: &HashMap<&'static str, String>) -> String {
    let expand = |text: &str| -> (String, bool) {
        let mut empty = false;
        let out = rx!(r"\{(\w+)(?::(\d+))?\}")
            .replace_all(text, |c: &fancy_regex::Captures| {
                let v = values.get(&c[1]).cloned().unwrap_or_default();
                if v.is_empty() {
                    empty = true;
                }
                pad(&v, c.get(2).map(|m| m.as_str())).replace('/', "\0") // "/" inside a value is no folder
            })
            .into_owned();
        (out, empty)
    };
    let mut out = String::new();
    let mut i = 0;
    for m in rx!(r"\[([^\[\]]*)\]").captures_iter(template).filter_map(Result::ok) {
        let whole = m.get(0).expect("match");
        out.push_str(&expand(&template[i..whole.start()]).0);
        let (text, empty) = expand(&m[1]);
        if !empty {
            out.push_str(&text);
        }
        i = whole.end();
    }
    out.push_str(&expand(&template[i..]).0);
    out.split('/')
        .filter(|p| !p.trim().is_empty())
        .map(|p| sanitize(&p.replace('\0', "-")))
        .collect::<Vec<_>>()
        .join("/")
}

/// (folder, file name) of a book below `root`.
pub fn destination(
    root: &Path,
    book: &Value,
    folder_tpl: &str,
    file_tpl: &str,
    title: Option<&str>,
) -> (PathBuf, String) {
    let values = fields(book, title);
    let folder = render(
        if folder_tpl.is_empty() {
            DEFAULT_FOLDER
        } else {
            folder_tpl
        },
        &values,
    );
    let folder = if folder.is_empty() {
        sanitize(&values["title"])
    } else {
        folder
    };
    let name = render(if file_tpl.is_empty() { DEFAULT_FILE } else { file_tpl }, &values).replace('/', "-");
    let name = if name.is_empty() { t("Hörbuch", &[]) } else { name };
    (root.join(folder), format!("{name}.m4b"))
}

/// Spelling-insensitive keys of a folder name: ("etahoffmann", "e hoffmann t a") – umlauts as ae/oe/ue,
/// accents and punctuation ignored, "01" equals "1".
pub fn folder_keys(name: &str) -> (String, String) {
    let s = name
        .to_lowercase()
        .replace('ß', "ss")
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue");
    let s: String = s.nfkd().filter(|c| canonical_combining_class(*c) == 0).collect();
    let mut words: Vec<String> = rx::findall(rx!(r"[^\W_]+"), &s)
        .into_iter()
        .map(|w| {
            if !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()) {
                let n = w.trim_start_matches('0');
                if n.is_empty() { "0".to_string() } else { n.to_string() }
            } else {
                w
            }
        })
        .collect();
    let joined = words.concat();
    words.sort();
    (joined, words.join(" "))
}

/// Swap in folders that already exist under another spelling, part by part below `root`.
pub fn reuse_existing(root: &Path, folder: &Path) -> PathBuf {
    let Ok(rel) = folder.strip_prefix(root) else {
        return folder.to_path_buf();
    };
    let mut current = root.to_path_buf();
    for part in rel.iter().map(|p| p.to_string_lossy().into_owned()) {
        let mut target = current.join(&part);
        if !target.is_dir() {
            let keys = folder_keys(&part);
            let mut existing: Vec<String> = std::fs::read_dir(&current)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|d| !d.starts_with(['.', '_']) && current.join(d).is_dir())
                        .collect()
                })
                .unwrap_or_default();
            existing.sort();
            let found = existing
                .iter()
                .find(|d| folder_keys(d).0 == keys.0)
                .or_else(|| existing.iter().find(|d| folder_keys(d).1 == keys.1));
            if let Some(m) = found {
                target = current.join(m);
            }
        }
        current = target;
    }
    current
}

/// `Titel` -> `Titel (2)` if the folder already holds an audiobook.
pub fn unique_folder(path: &Path) -> PathBuf {
    let taken = |p: &Path| {
        std::fs::read_dir(p).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.ends_with(".m4b") || n.ends_with(".m4a") || n.ends_with(".mp3")
            })
        })
    };
    if !taken(path) {
        return path.to_path_buf();
    }
    let mut n = 2;
    loop {
        let p = PathBuf::from(format!("{} ({n})", path.display()));
        if !taken(&p) {
            return p;
        }
        n += 1;
    }
}

/// Move a file so that `dest` only ever appears complete (another disk: hidden copy, then rename).
pub fn move_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dest) {
        Ok(()) => return Ok(()),
        Err(e) if e.raw_os_error() != Some(libc::EXDEV) => return Err(e),
        Err(_) => {}
    }
    let tmp = dest.with_file_name(format!(
        "{PARTIAL}{}",
        dest.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let copied = (|| {
        std::fs::copy(src, &tmp)?;
        std::fs::File::open(&tmp)?.sync_all()?;
        std::fs::rename(&tmp, dest)
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::remove_file(src)
}

/// Hidden half-copies left by a crash in `move_atomic` – in this folder only.
pub fn remove_partials(folder: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(folder) else { return 0 };
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(PARTIAL))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// Move the book into the library; returns the final path.
pub fn place(src: &Path, folder: &Path, filename: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let dest = folder.join(filename);
    move_atomic(src, &dest)?;
    Ok(dest)
}

/// Preview for the settings dialog (public-domain works).
pub fn example(folder_tpl: &str, file_tpl: &str) -> Vec<String> {
    let samples = [
        serde_json::json!({"title": "Eine Studie in Scharlachrot", "authors": ["Arthur Conan Doyle"], "narrators": ["Erika Muster"],
                           "series": "Sherlock Holmes", "series_position": "1", "release_date": "1887-11-01"}),
        serde_json::json!({"title": "Die Verwandlung", "authors": ["Franz Kafka"], "narrators": ["Max Muster"], "release_date": "1915-10-01"}),
    ];
    samples
        .iter()
        .map(|b| {
            let (folder, name) = destination(Path::new(""), b, folder_tpl, file_tpl, None);
            folder
                .join(name)
                .display()
                .to_string()
                .trim_start_matches('/')
                .to_string()
        })
        .collect()
}

/// A free name next to `path`: "x (2)", "x (3)" … (files keep their extension at the end).
pub fn free_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let (base, ext) = if path.is_file() {
        match (path.file_stem(), path.extension()) {
            (Some(stem), Some(ext)) => (
                path.with_file_name(stem).display().to_string(),
                format!(".{}", ext.to_string_lossy()),
            ),
            _ => (path.display().to_string(), String::new()),
        }
    } else {
        (path.display().to_string(), String::new())
    };
    let mut n = 2;
    loop {
        let p = PathBuf::from(format!("{base} ({n}){ext}"));
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_python() {
        crate::text::set_language("de");
        let g: Value = serde_json::from_str(include_str!("../../../contract/golden/words.json")).expect("words.json");
        for c in g["destination"].as_array().unwrap() {
            let (folder, name) = destination(
                Path::new("/lib"),
                &c[0],
                c[1].as_str().unwrap(),
                c[2].as_str().unwrap(),
                None,
            );
            assert_eq!(json_pair(&folder, &name), c[3], "{}", c[0]);
        }
        for c in g["folder_keys"].as_array().unwrap() {
            let (a, b) = folder_keys(c[0].as_str().unwrap());
            assert_eq!(serde_json::json!([a, b]), c[1], "{}", c[0]);
        }
        for c in g["sanitize"].as_array().unwrap() {
            assert_eq!(sanitize(c[0].as_str().unwrap()), c[1].as_str().unwrap(), "{}", c[0]);
        }
    }

    fn json_pair(folder: &Path, name: &str) -> Value {
        serde_json::json!([folder.display().to_string(), name])
    }
}
