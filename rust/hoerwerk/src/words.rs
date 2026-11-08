//! Words for matching names and titles – exactly like the Python helpers (usenet.match_words,
//! library_index.title_words/names_of, natsort.natural_key). contract/golden/words.json holds the yardstick.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

/// Words that say nothing about which book it is (audible._STOP).
pub const STOP: [&str; 19] = [
    "der", "die", "das", "the", "a", "an", "und", "and", "of", "von", "ein", "eine", "teil", "band", "book", "buch",
    "folge", "vol", "volume",
];
const STOP_PART: &str = "part";

pub fn is_stop(w: &str) -> bool {
    w == STOP_PART || STOP.contains(&w)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// "Bis(s)" is one word: "(x)" of at most three word characters right after a word character loses its brackets.
fn join_bracket_letters(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        if c[i] == '(' && i > 0 && is_word(c[i - 1]) {
            let mut j = i + 1;
            while j < c.len() && is_word(c[j]) && j - i <= 3 {
                j += 1;
            }
            let n = j - i - 1;
            if (1..=3).contains(&n) && j < c.len() && c[j] == ')' && (j + 1 == c.len() || !is_word(c[j + 1])) {
                out.extend(&c[i + 1..j]);
                i = j + 1;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

fn special(c: char) -> Option<&'static str> {
    Some(match c {
        'ä' => "a",
        'ö' => "o",
        'ü' => "u",
        'ø' => "o",
        'æ' => "ae",
        'œ' => "oe",
        'ł' => "l",
        'đ' | 'ð' => "d",
        'þ' => "th",
        'ı' => "i",
        _ => return None,
    })
}

/// Spelling-insensitive words: "Fräulein", "Fraeulein", "Fraulein" and "FRAULEIN" all become "fraulein".
pub fn match_words(text: &str) -> Vec<String> {
    let lower = join_bracket_letters(text).to_lowercase().replace('ß', "ss");
    let mut plain = String::with_capacity(lower.len());
    for c in lower.chars() {
        match special(c) {
            Some(s) => plain.push_str(s),
            None => plain.push(c),
        }
    }
    // "ae" -> "a", "oe" -> "o", "ue" -> "u"
    let chars: Vec<char> = plain.chars().collect();
    let mut folded = String::with_capacity(plain.len());
    let mut i = 0;
    while i < chars.len() {
        folded.push(chars[i]);
        if matches!(chars[i], 'a' | 'o' | 'u') && chars.get(i + 1) == Some(&'e') {
            i += 1;
        }
        i += 1;
    }
    let ascii: String = folded.nfkd().filter(|c| canonical_combining_class(*c) == 0).collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in ascii.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Significant words of a title (no articles, no "Band"/"Teil", no series cruft in brackets).
pub fn title_words(title: &str) -> Vec<String> {
    // re.sub(r"\([^)]*\)|\[[^\]]*\]", " ", title): a bracket up to the next closing one goes
    let c: Vec<char> = title.chars().collect();
    let mut cleaned = String::new();
    let mut i = 0;
    while i < c.len() {
        let close = match c[i] {
            '(' => Some(')'),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(end) = close.and_then(|e| c[i + 1..].iter().position(|x| *x == e)) {
            cleaned.push(' ');
            i += end + 2;
            continue;
        }
        cleaned.push(c[i]);
        i += 1;
    }
    let words = match_words(&cleaned);
    let significant: Vec<String> = words
        .iter()
        .filter(|w| !is_stop(w) && (w.chars().count() > 1 || w.chars().all(|c| c.is_ascii_digit())))
        .cloned()
        .collect();
    if significant.is_empty() { words } else { significant }
}

fn lower_eq_at(c: &[char], at: usize, word: &str) -> bool {
    let w: Vec<char> = word.chars().collect();
    at + w.len() <= c.len()
        && c[at..at + w.len()]
            .iter()
            .zip(&w)
            .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
}

fn boundary(c: &[char], at: usize) -> bool {
    let before = at > 0 && is_word(c[at - 1]);
    let after = at < c.len() && is_word(c[at]);
    before != after
}

/// After a comma there must be at least two words ("Storm, Theodor" is one name, "A, Bert Brecht" two).
fn comma_splits(c: &[char], after: usize) -> bool {
    let mut i = after;
    while i < c.len() && c[i].is_whitespace() {
        i += 1;
    }
    let start = i;
    while i < c.len() && !c[i].is_whitespace() {
        i += 1;
    }
    if i == start {
        return false;
    }
    let ws = i;
    while i < c.len() && c[i].is_whitespace() {
        i += 1;
    }
    i > ws && i < c.len()
}

/// The separator at `at` (after optional whitespace) – its end, or None.
fn separator_at(c: &[char], at: usize) -> Option<usize> {
    let ch = *c.get(at)?;
    if ";/&|+".contains(ch) {
        return Some(at + 1);
    }
    if ch == ',' && comma_splits(c, at + 1) {
        return Some(at + 1);
    }
    for word in ["und", "and"] {
        if boundary(c, at) && lower_eq_at(c, at, word) && boundary(c, at + 3) {
            return Some(at + 3);
        }
    }
    if boundary(c, at) && lower_eq_at(c, at, "feat") {
        return Some(if c.get(at + 4) == Some(&'.') { at + 5 } else { at + 4 });
    }
    for phrase in ["gelesen von", "read by"] {
        let n = phrase.chars().count();
        if boundary(c, at) && lower_eq_at(c, at, phrase) && boundary(c, at + n) {
            return Some(at + n);
        }
    }
    None
}

/// library_index.NAME_SPLIT: "A, B" / "A & B" / "A und B" / "A feat. B" / "A gelesen von B" -> parts.
fn split_names(v: &str) -> Vec<String> {
    let c: Vec<char> = v.chars().collect();
    let mut parts = Vec::new();
    let (mut piece_start, mut i) = (0, 0);
    while i < c.len() {
        let mut j = i;
        while j < c.len() && c[j].is_whitespace() {
            j += 1;
        }
        if let Some(mut end) = separator_at(&c, j) {
            while end < c.len() && c[end].is_whitespace() {
                end += 1;
            }
            parts.push(c[piece_start..i].iter().collect());
            piece_start = end;
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    parts.push(c[piece_start..].iter().collect());
    parts
}

/// Every name in author-like tags: the whole value and its parts ("A, B" -> "A, B", "A", "B").
pub fn names_of(values: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        for n in std::iter::once(v.to_string()).chain(split_names(v)) {
            let n = n.trim_matches([' ', ',']).to_string();
            if !n.is_empty() && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

pub fn surname(name: &str) -> String {
    let words = match_words(name);
    let pick = if name.contains(',') {
        words.first()
    } else {
        words.last()
    };
    pick.cloned().unwrap_or_default()
}

pub fn surnames<S: AsRef<str>>(names: &[S]) -> BTreeSet<String> {
    names
        .iter()
        .map(|n| surname(n.as_ref()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Folder names like "Storm, Theodor - Der Schimmelreiter" -> (author, title).
pub fn split_author_title(name: &str) -> Option<(String, String)> {
    let c: Vec<char> = name.chars().collect();
    let start = c.iter().position(|x| !x.is_whitespace()).unwrap_or(c.len());
    for p in start + 1..c.len() {
        let mut i = p;
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if i == p || i >= c.len() || !(c[i] == '-' || c[i] == '–') {
            continue;
        }
        let mut j = i + 1;
        while j < c.len() && c[j].is_whitespace() {
            j += 1;
        }
        if j == i + 1 || (j >= c.len() && j - i < 3) {
            continue;
        }
        let author: String = c[start..p].iter().collect();
        // only whitespace after the dash: the regex gives the title its last whitespace character
        let title: String = if j >= c.len() {
            c[j - 1].to_string()
        } else {
            c[j..].iter().collect::<String>().trim_end().to_string()
        };
        if author.chars().all(|x| x.is_ascii_digit()) {
            return None; // "01 - Title" is a volume number, not an author
        }
        return Some((author, title));
    }
    None
}

pub fn book_key(authors: &[String], title: &str) -> Option<String> {
    let s = surname(authors.first().map(String::as_str).unwrap_or(""));
    let w = title_words(title);
    (!s.is_empty() && !w.is_empty()).then(|| format!("{s}|{}", w.join(" ")))
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone)]
pub enum Part {
    Num(u128),
    Text(String),
}

/// Natural sort order: "2" before "10", case ignored (natsort.natural_key).
pub fn natural_key(s: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if num.is_empty() {
                out.push(Part::Text(std::mem::take(&mut text).to_lowercase()));
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push(Part::Num(num.parse().unwrap_or(u128::MAX)));
                num.clear();
            }
            text.push(c);
        }
    }
    if !num.is_empty() {
        out.push(Part::Num(num.parse().unwrap_or(u128::MAX)));
        out.push(Part::Text(String::new()));
    } else {
        out.push(Part::Text(text.to_lowercase()));
    }
    out
}

/// Compare paths part by part ("CD1/2.mp3" before "CD1/10.mp3", "CD2" after "CD1").
pub fn natural_path_cmp(a: &str, b: &str) -> Ordering {
    let ka: Vec<Vec<Part>> = a.split('/').map(natural_key).collect();
    let kb: Vec<Vec<Part>> = b.split('/').map(natural_key).collect();
    ka.cmp(&kb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn golden() -> Value {
        serde_json::from_str(include_str!("../../../contract/golden/words.json")).expect("words.json")
    }

    fn strs(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn like_python() {
        let g = golden();
        for case in g["match_words"].as_array().unwrap() {
            assert_eq!(
                match_words(case[0].as_str().unwrap()),
                strs(&case[1]),
                "match_words {}",
                case[0]
            );
        }
        for case in g["title_words"].as_array().unwrap() {
            assert_eq!(
                title_words(case[0].as_str().unwrap()),
                strs(&case[1]),
                "title_words {}",
                case[0]
            );
        }
        for case in g["names_of"].as_array().unwrap() {
            assert_eq!(
                names_of(&[case[0].as_str().unwrap()]),
                strs(&case[1]),
                "names_of {}",
                case[0]
            );
        }
        for case in g["split_author_title"].as_array().unwrap() {
            let want = case[1]
                .as_array()
                .map(|a| (a[0].as_str().unwrap().to_string(), a[1].as_str().unwrap().to_string()));
            assert_eq!(
                split_author_title(case[0].as_str().unwrap()),
                want,
                "split_author_title {}",
                case[0]
            );
        }
        let mut names = strs(&g["natural_sort"]["input"]);
        names.sort_by(|a, b| natural_path_cmp(a, b));
        assert_eq!(names, strs(&g["natural_sort"]["sorted"]));
    }
}
