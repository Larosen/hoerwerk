//! Downloads with several volumes of a series ("Sneijder und Nemez 01 - 07", "Band 1-3").

use serde_json::{Value, json};

use crate::rx;
use crate::rx::{escape, search, sub};

const VOLUME_WORD: &str =
    r"(?:b(?:a|ä|ae)nde?|folgen?|teile?|vol(?:ume)?s?|books?|b(?:u|ü|ue)cher|buch|episoden?|fall|f(?:a|ä|ae)lle)";
const MAX_VOLUMES: i64 = 60;

fn plain(name: &str) -> String {
    sub(rx!(r"[._]+"), name, " ").trim().to_string()
}

fn trim_chars(s: &str, chars: &str) -> String {
    s.trim_matches(|c: char| chars.contains(c)).to_string()
}

/// {volumes, series, author} when the name announces a range of volumes, else None.
pub fn pack_info(name: &str) -> Option<Value> {
    let text = plain(name);
    let range = rx!(r"(?i)(?<![\w])(\d{1,3})\s*(?:-|–|bis|to)\s*(\d{1,3})(?![\d])(?!\s*(?:cd|kbps|kbit|k\b|disc))");
    let m = search(range, &text)?;
    let first: i64 = m[1].parse().ok()?;
    let last: i64 = m[2].parse().ok()?;
    if !(0 <= first && first < last && last <= first + MAX_VOLUMES) {
        return None;
    }
    let start = m.get(0)?.start();
    let tail_word = fancy_regex::Regex::new(&format!(r"(?i)\b{VOLUME_WORD}\.?\s*$")).ok()?;
    let prefix = trim_chars(&rx::sub(&tail_word, &text[..start], ""), " -–:,");
    let parts: Vec<String> = rx::split(rx!(r"\s+-\s+|(?<=\w)-(?=\w)"), &prefix)
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let (author, series) = if parts.len() >= 2 {
        (parts[0].clone(), parts[1..].join(" "))
    } else {
        (String::new(), parts.first().cloned().unwrap_or_default())
    };
    Some(json!({"volumes": (first..=last).collect::<Vec<_>>(), "series": series, "author": author}))
}

fn without_known(folder_name: &str, pack: Option<&Value>) -> String {
    let mut text = plain(folder_name);
    for key in ["author", "series"] {
        let known = pack.and_then(|p| p[key].as_str()).unwrap_or("");
        if !known.is_empty()
            && let Ok(re) = fancy_regex::Regex::new(&format!("(?i){}", escape(known)))
        {
            text = sub(&re, &text, " ");
        }
    }
    text
}

/// Volume number of one book folder of a pack: "03 - Todesmal", "Band 3", "Sneijder und Nemez 3 - …".
pub fn volume_of(folder_name: &str, pack: Option<&Value>) -> Option<i64> {
    let text = without_known(folder_name, pack);
    let word = fancy_regex::Regex::new(&format!(r"(?i)\b{VOLUME_WORD}\.?\s*(\d{{1,3}})\b")).ok()?;
    let m = search(&word, &text).or_else(|| {
        search(
            rx!(r"(?<![\w])(\d{1,3})(?![\d])(?!\s*(?:cd|kbps|kbit|k\b|disc))"),
            &text,
        )
    })?;
    let n: i64 = m[1].parse().ok()?;
    let volumes: Vec<i64> = pack
        .and_then(|p| p["volumes"].as_array())
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    (volumes.is_empty() || volumes.contains(&n)).then_some(n)
}

/// The book title in a pack folder name, without author, series and volume number.
pub fn title_of(folder_name: &str, pack: Option<&Value>) -> String {
    let text = without_known(folder_name, pack);
    let word = fancy_regex::Regex::new(&format!(r"(?i)\b{VOLUME_WORD}\.?\s*\d{{1,3}}\b")).expect("valid regex");
    let text = sub(&word, &text, " ");
    let text = rx::sub1(rx!(r"^\W*\d{1,3}(?![\d])\W*"), text.trim(), "");
    trim_chars(&sub(rx!(r"\s+"), &text, " "), " -–:,")
}
