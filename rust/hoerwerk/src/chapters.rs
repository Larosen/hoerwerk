//! Chapter names from track files and tags (encoder.chapter_title / chapter_titles / looks_generic).

use std::sync::LazyLock;

use regex_lite::Regex;

use crate::text::t;

static GENERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:track|kapitel|chapter|chapitre|cap[ií]tulo|capitolo|hoofdstuk|teil|part|cd|disc|disk|titel)?[\s_.-]*\d+\s*$")
        .expect("valid regex")
});
static LEADING_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d{1,4}(?:\s*[-_.)]\s*|\s+)").expect("valid regex"));
static BARE_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:track|kapitel|chapter|teil|part|titel|cd|disc)$").expect("valid regex"));
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[(\[]\s*(?:19|20)\d\d\s*[)\]]").expect("valid regex"));
static CHAPTER_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:kapitel|chapter|track|teil|part|titel|folge)[\s.]*$").expect("valid regex")
});
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").expect("valid regex"));

const SEP: &str = " -–—:_.,/|";

pub fn is_generic(name: &str) -> bool {
    GENERIC.is_match(name)
}

/// True if chapter names carry no information ("Track 01", "1", "Kapitel 3").
pub fn looks_generic<S: AsRef<str>>(titles: &[S]) -> bool {
    titles.len() <= 1 || titles.iter().all(|x| is_generic(x.as_ref()))
}

fn stem(rel_path: &str) -> &str {
    let base = rel_path.rsplit('/').next().unwrap_or(rel_path);
    match base.rfind('.') {
        Some(i) if i > 0 && !base[..i].chars().all(|c| c == '.') => &base[..i],
        _ => base,
    }
}

/// Name for the chapter of one source file. `tag` prefers the title tag, `filename` the file name;
/// `auto` the tag when there is a real one, otherwise the cleaned file name.
pub fn chapter_title(rel_path: &str, tag_title: Option<&str>, mode: &str) -> String {
    let stem = stem(rel_path);
    let tag = tag_title.unwrap_or("").trim();
    if mode == "tag" && !tag.is_empty() {
        return tag.to_string();
    }
    if mode == "auto" && !tag.is_empty() && !is_generic(tag) {
        return tag.to_string();
    }
    if mode == "filename" {
        return stem.to_string();
    }
    let clean = LEADING_NUMBER.replace(stem, "").replace('_', " ").trim().to_string();
    if clean.is_empty() || BARE_WORD.is_match(&clean) {
        return stem.replace('_', " ").trim().to_string(); // "01 Kapitel": the number is the information
    }
    clean
}

fn strip_sep(s: &str) -> &str {
    s.trim_matches(|c: char| SEP.contains(c))
}

/// Text all names start (or end) with, cut at a separator so no word is split.
fn common_affix(names: &[Vec<char>], suffix: bool) -> Vec<char> {
    if names.len() < 2 {
        return Vec::new();
    }
    let seqs: Vec<Vec<char>> = names
        .iter()
        .map(|n| {
            if suffix {
                n.iter().rev().copied().collect()
            } else {
                n.clone()
            }
        })
        .collect();
    let mut len = 0;
    while seqs.iter().all(|s| len < s.len() && s[len] == seqs[0][len]) {
        len += 1;
    }
    let mut common: Vec<char> = seqs[0][..len].to_vec();
    if suffix {
        common.reverse();
    }
    let shortest = names.iter().map(Vec::len).min().unwrap_or(0);
    while !common.is_empty() && common.len() < shortest {
        let edge = if suffix { common[0] } else { common[common.len() - 1] };
        if SEP.contains(edge) {
            break;
        }
        if suffix {
            common.remove(0);
        } else {
            common.pop();
        }
    }
    common
}

/// A pattern for `text` that ignores case also outside ASCII (Python's re.I).
fn caseless(text: &str) -> String {
    text.chars()
        .map(|c| {
            let lower: String = c.to_lowercase().collect();
            let upper: String = c.to_uppercase().collect();
            if lower.chars().count() == 1 && upper.chars().count() == 1 && lower != upper {
                format!("[{}{}]", regex_lite::escape(&lower), regex_lite::escape(&upper))
            } else {
                regex_lite::escape(&c.to_string())
            }
        })
        .collect()
}

/// Clean chapter names of one book together: text every track repeats (the book title, "(2012)",
/// "Sie werden dich jagen - ") goes; what is left that carries no name becomes "Kapitel N".
pub fn chapter_titles<S: AsRef<str>>(names: &[S], book_title: &str) -> Vec<String> {
    let mut out: Vec<String> = names
        .iter()
        .map(|n| YEAR.replace_all(n.as_ref(), "").trim().to_string())
        .collect();
    let title = book_title.trim();
    if !title.is_empty() {
        let sep = regex_lite::escape(SEP);
        let rx = Regex::new(&format!(r"[\s{sep}]*{}[\s{sep}]*", caseless(title))).expect("valid regex");
        out = out
            .iter()
            .map(|n| {
                if n.to_lowercase() == title.to_lowercase() {
                    String::new()
                } else {
                    strip_sep(&rx.replace_all(n, " ")).to_string()
                }
            })
            .collect();
    }
    if out.len() > 1 {
        let chars: Vec<Vec<char>> = out.iter().map(|n| n.chars().collect()).collect();
        let pre = common_affix(&chars, false);
        let pre_s: String = pre.iter().collect();
        let chars = if !pre.is_empty() && !CHAPTER_WORD.is_match(&pre_s) {
            chars.iter().map(|n| n[pre.len()..].to_vec()).collect()
        } else {
            chars
        };
        let post = common_affix(&chars, true);
        out = chars
            .iter()
            .map(|n| n[..n.len() - post.len()].iter().collect())
            .collect();
    }
    let out: Vec<String> = out
        .iter()
        .map(|n| strip_sep(&SPACES.replace_all(n, " ")).to_string())
        .collect();
    // numbers left ("001", "Track 7"): keep them if they count up, else count ourselves
    let nums: Vec<Option<u128>> = out
        .iter()
        .map(|n| NUMBER.find(n).map(|m| m.as_str().parse().unwrap_or(u128::MAX)))
        .collect();
    let counted = nums.iter().all(Option::is_some) && nums.windows(2).all(|w| w[1] > w[0]);
    out.iter()
        .enumerate()
        .map(|(i, n)| {
            if !n.is_empty() && !is_generic(n) {
                n.clone()
            } else {
                let num = if counted { nums[i].unwrap_or(0) } else { i as u128 + 1 };
                t("Kapitel {n}", &[("n", &num.to_string())])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn like_python() {
        crate::text::set_language("de");
        let g: Value = serde_json::from_str(include_str!("../../../contract/golden/words.json")).expect("words.json");
        for case in g["chapter_title"].as_array().unwrap() {
            let got = chapter_title(
                case[0].as_str().unwrap(),
                case[1]["title"].as_str(),
                case[2].as_str().unwrap(),
            );
            assert_eq!(got, case[3].as_str().unwrap(), "{case}");
        }
        for case in g["chapter_titles"].as_array().unwrap() {
            let names: Vec<&str> = case[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            let want: Vec<&str> = case[2]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            assert_eq!(chapter_titles(&names, case[1].as_str().unwrap()), want, "{case}");
        }
    }
}
