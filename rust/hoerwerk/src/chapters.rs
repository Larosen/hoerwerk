//! Chapter names from track files and tags (encoder.chapter_title / chapter_titles / looks_generic).

use std::sync::LazyLock;

use regex_lite::Regex;

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
static COUNTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(?:(?:track|titel|nr|cd|disc|disk)\.?\s*)?(\d{1,4})(?:\s*[-_.]\s*(\d{1,4}))?(?:\s*[-_.:)]+\s*|\s+|$)",
    )
    .expect("valid regex")
});
static MURK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(?:track|titel|nr|no|cd|disc|disk)?[\s_.#-]*\d+\s*$").expect("valid regex"));
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^)]*\)|\[[^\]]*\]").expect("valid regex"));
static CHAPTER_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(kapitel|chapter|chapitre|cap[ií]tulo|capitolo|hoofdstuk|teil|part|folge)\s+(.+)$")
        .expect("valid regex")
});
static ORDINAL_CHAPTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^([a-zäöüß]+)\s+(kapitel|chapter|teil|part)\b(.*)$").expect("valid regex"));
static ENGLISH_CHAPTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|\s)chapter\s").expect("valid regex"));
static GERMAN_CHAPTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|\s)kapitel\s").expect("valid regex"));
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

/// Clean chapter names of one book together (only the book title known, language from the UI).
pub fn chapter_titles<S: AsRef<str>>(names: &[S], book_title: &str) -> Vec<String> {
    let known: Vec<String> = Some(book_title.trim())
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .into_iter()
        .collect();
    clean_titles(names, &known, "").names
}

/// What the cleaning made of the names – and whether most of the raw names was repeated text.
pub struct Cleaned {
    pub names: Vec<String>,
    pub repetitive: bool,
}

/// The chapter names of one book, cleaned together. A chapter name keeps only what tells this chapter from
/// the others; the order of the chapters is never touched (it comes from the files before this).
///
/// 1. raw name: years in brackets go, "_" and dots between words of a scene-style name become spaces
/// 2. leading counters ("01 - ", "001.", "Track 3 -", "CD1-07") go – only when (nearly) every name has one
///    and they count up, so "1984" stays a title
/// 3. book title, series and author go from the start or end of a name (case, accents and "(Ungekürzt)",
///    "(Full-Cast Edition)" … don't matter)
/// 4. text most names repeat at their start or end goes – word by word, cut only at a separator
/// 5. "Chapter One - The Worst Birthday" becomes "Chapter 1: The Worst Birthday", "Erstes Kapitel" "Kapitel 1"
/// 6. what says nothing becomes "Kapitel N" / "Chapter N"; doubles get their number; very long names are cut;
///    when more than half is like that, every chapter is numbered
pub fn clean_titles<S: AsRef<str>>(names: &[S], known: &[String], lang: &str) -> Cleaned {
    let raw_names: Vec<String> = names.iter().map(|n| raw(n.as_ref())).collect();
    let lang = if lang.is_empty() {
        guess_lang(&raw_names)
    } else {
        lang.to_string()
    };
    let mut out = raw_names;
    let counters = strip_counters(&mut out);
    let length = |v: &[String]| v.iter().map(|n| n.chars().count()).sum::<usize>();
    let before = length(&out);
    let mut known: Vec<Vec<String>> = known
        .iter()
        .map(|k| crate::words::match_words(&without_brackets(k)))
        .filter(|k| !k.is_empty())
        .collect();
    known.sort_by_key(|k| std::cmp::Reverse(k.len()));
    known.dedup();
    for n in out.iter_mut() {
        *n = strip_known(n, &known);
    }
    strip_shared(&mut out);
    let repetitive = before > 0 && (before - length(&out)) * 2 > before;
    for n in out.iter_mut() {
        *n = chapter_form(n);
    }
    Cleaned {
        names: settle(out, &counters, &lang),
        repetitive,
    }
}

// -- rule 1: the raw name -------------------------------------------------------------------------

fn raw(name: &str) -> String {
    let mut n = YEAR.replace_all(name, "").replace('_', " ");
    if !n.trim().contains(' ') && n.contains('.') {
        // "Harry.Potter.Kapitel.1": a scene-style name – dots between words are spaces
        let chars: Vec<char> = n.chars().collect();
        n = chars
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let word = |j: Option<&char>| j.is_some_and(|c| c.is_alphanumeric());
                if *c == '.' && i > 0 && word(chars.get(i - 1)) && word(chars.get(i + 1)) {
                    ' '
                } else {
                    *c
                }
            })
            .collect();
    }
    SPACES.replace_all(n.trim(), " ").to_string()
}

/// "en" when most names say "Chapter", "de" when they say "Kapitel" – else unknown.
fn guess_lang(names: &[String]) -> String {
    let count = |rx: &Regex| names.iter().filter(|n| rx.is_match(n)).count();
    let (en, de) = (count(&ENGLISH_CHAPTER), count(&GERMAN_CHAPTER));
    if en > de && en * 2 >= names.len() {
        "en".into()
    } else if de > en && de * 2 >= names.len() {
        "de".into()
    } else {
        String::new()
    }
}

// -- rule 2: counters -----------------------------------------------------------------------------

/// Leading counters go when at least 90 % of the names (two or more) have one and they count up. Returns the
/// counter of every name (for "Kapitel N" later) when they are plain numbers.
fn strip_counters(names: &mut [String]) -> Vec<Option<u64>> {
    let n = names.len();
    let none = vec![None; n];
    if n < 2 {
        return none;
    }
    let found: Vec<Option<(u64, Option<u64>, usize)>> = names
        .iter()
        .map(|name| {
            COUNTER.captures(name).map(|c| {
                let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u64>().ok());
                (num(1).unwrap_or(0), num(2), c.get(0).map_or(0, |m| m.end()))
            })
        })
        .collect();
    let have = found.iter().filter(|f| f.is_some()).count();
    if have * 10 < n * 9 {
        return none;
    }
    let keys: Vec<(u64, u64)> = found.iter().flatten().map(|(a, b, _)| (*a, b.unwrap_or(0))).collect();
    if !keys.windows(2).all(|w| w[1] > w[0]) {
        return none; // not counting up: the number belongs to the name ("1984")
    }
    let plain = found.iter().flatten().all(|(_, b, _)| b.is_none());
    for (name, f) in names.iter_mut().zip(&found) {
        if let Some((_, _, end)) = f {
            *name = name[*end..].trim().to_string();
        }
    }
    found
        .iter()
        .map(|f| f.and_then(|(a, _, _)| plain.then_some(a)))
        .collect()
}

// -- word spans -----------------------------------------------------------------------------------

/// The words of a name with their byte range and whether they sit in brackets.
fn spans(name: &str) -> Vec<(String, usize, usize, bool)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let push = |a: usize, b: usize, inside: bool, out: &mut Vec<(String, usize, usize, bool)>| {
        let w = crate::words::match_words(&name[a..b]).join("");
        if !w.is_empty() {
            out.push((w, a, b, inside));
        }
    };
    for (i, c) in name.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
            continue;
        }
        if let Some(a) = start.take() {
            push(a, i, depth > 0, &mut out);
        }
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    if let Some(a) = start {
        push(a, name.len(), depth > 0, &mut out);
    }
    out
}

fn without_brackets(text: &str) -> String {
    BRACKETS.replace_all(text, " ").to_string()
}

fn is_sep(c: char) -> bool {
    SEP.contains(c) || c.is_whitespace()
}

/// From `at` on: skip separators and whole bracket groups – where the rest of the name starts.
fn skip_filler(name: &str, mut at: usize) -> usize {
    loop {
        let rest = &name[at..];
        let trimmed = rest.trim_start_matches(is_sep);
        at += rest.len() - trimmed.len();
        let Some(open) = trimmed.chars().next().filter(|c| *c == '(' || *c == '[') else {
            return at;
        };
        let close = if open == '(' { ')' } else { ']' };
        match trimmed.find(close) {
            Some(i) => at += i + 1,
            None => return at,
        }
    }
}

// -- rule 3: known names --------------------------------------------------------------------------

/// The book title, series or author at the start or end of a name goes (also a name that is just that).
fn strip_known(name: &str, known: &[Vec<String>]) -> String {
    let mut name = name.to_string();
    for k in known {
        let words: Vec<(String, usize, usize, bool)> = spans(&name).into_iter().filter(|w| !w.3).collect();
        if words.len() < k.len() {
            continue;
        }
        let keys: Vec<&String> = words.iter().map(|w| &w.0).collect();
        if keys.iter().map(|x| x.as_str()).eq(k.iter().map(String::as_str)) {
            return String::new();
        }
        if keys[..k.len()]
            .iter()
            .map(|x| x.as_str())
            .eq(k.iter().map(String::as_str))
        {
            let cut = skip_filler(&name, words[k.len() - 1].2);
            name = name[cut..].trim().to_string();
            continue;
        }
        let tail = words.len() - k.len();
        if keys[tail..].iter().map(|x| x.as_str()).eq(k.iter().map(String::as_str)) {
            name = name[..words[tail].1].trim_end_matches(is_sep).to_string();
        }
    }
    name
}

// -- rule 4: shared text --------------------------------------------------------------------------

/// Where a name may be cut: (end of the part before, start of the part after) at every separator –
/// ":", "|", "/", "–", "—", " - " – and after a closing bracket.
fn cuts(name: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if SEP.contains(c) || c.is_whitespace() {
            let mut j = i;
            while j < chars.len() && (SEP.contains(chars[j].1) || chars[j].1.is_whitespace()) {
                j += 1;
            }
            let run: String = chars[i..j].iter().map(|x| x.1).collect();
            let strong = run.contains([':', '|', '/', '–', '—']) || run.contains(" -") || run.contains("- ");
            let end = chars.get(j).map_or(name.len(), |x| x.0);
            let after_bracket = i > 0 && matches!(chars[i - 1].1, ')' | ']');
            if (strong || after_bracket) && pos > 0 && end < name.len() {
                out.push((pos, end));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Text that most names (80 %, all when there are fewer than five) repeat at the start or end goes – cut
/// at a separator, never a bare chapter word, and something always stays.
fn strip_shared(names: &mut [String]) {
    let n = names.len();
    if n < 2 {
        return;
    }
    let need = if n >= 5 { (n * 4).div_ceil(5) } else { n };
    let key = |text: &str| crate::words::match_words(text).join(" ");
    let pick = |options: Vec<Vec<(String, usize)>>| -> Option<String> {
        let mut count: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for o in &options {
            let mut seen = std::collections::HashSet::new();
            for (k, _) in o {
                if seen.insert(k.as_str()) {
                    *count.entry(k.as_str()).or_default() += 1;
                }
            }
        }
        count
            .into_iter()
            .filter(|(k, c)| {
                let words: Vec<&str> = k.split(' ').collect();
                let only_chapter = words.iter().all(|w| CHAPTER_WORD.is_match(w));
                // a shared part ending in a number ("Teil 1") may only go when every name has it
                let numbered = words.last().is_some_and(|w| w.chars().all(|c| c.is_ascii_digit()));
                !k.is_empty() && *c >= need && !only_chapter && (!numbered || *c == n)
            })
            .max_by_key(|(k, c)| (k.split(' ').count(), *c))
            .map(|(k, _)| k.to_string())
    };
    // start
    let options: Vec<Vec<(String, usize)>> = names
        .iter()
        .map(|name| cuts(name).into_iter().map(|(a, b)| (key(&name[..a]), b)).collect())
        .collect();
    if let Some(best) = pick(options.clone()) {
        for (name, o) in names.iter_mut().zip(&options) {
            if let Some((_, b)) = o.iter().find(|(k, _)| *k == best) {
                *name = name[*b..].trim().to_string();
            }
        }
    }
    // end
    let options: Vec<Vec<(String, usize)>> = names
        .iter()
        .map(|name| {
            let mut v: Vec<(String, usize)> = cuts(name).into_iter().map(|(a, b)| (key(&name[b..]), a)).collect();
            // "… (Ungekürzt)": a trailing bracket is cut off as well
            if let Some(open) = name.rfind([' ']).filter(|i| name[i + 1..].starts_with(['(', '['])) {
                v.push((key(&name[open + 1..]), open));
            }
            v
        })
        .collect();
    if let Some(best) = pick(options.clone()) {
        for (name, o) in names.iter_mut().zip(&options) {
            if let Some((_, a)) = o.iter().find(|(k, _)| *k == best) {
                *name = name[..*a].trim_end_matches(is_sep).to_string();
            }
        }
    }
}

// -- rule 5: chapter numbers ----------------------------------------------------------------------

fn number_word(word: &str) -> Option<u32> {
    let w = word.to_lowercase();
    if let Ok(n) = w.parse::<u32>() {
        return Some(n);
    }
    const EN: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const EN_TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    const DE: [&str; 20] = [
        "null",
        "eins",
        "zwei",
        "drei",
        "vier",
        "fünf",
        "sechs",
        "sieben",
        "acht",
        "neun",
        "zehn",
        "elf",
        "zwölf",
        "dreizehn",
        "vierzehn",
        "fünfzehn",
        "sechzehn",
        "siebzehn",
        "achtzehn",
        "neunzehn",
    ];
    const DE_TENS: [&str; 8] = [
        "zwanzig", "dreißig", "vierzig", "fünfzig", "sechzig", "siebzig", "achtzig", "neunzig",
    ];
    let small = |x: &str| {
        EN.iter()
            .position(|e| *e == x)
            .or_else(|| DE.iter().position(|e| *e == x))
            .or_else(|| (x == "ein").then_some(1))
    };
    if let Some(n) = small(&w) {
        return Some(n as u32);
    }
    let tens = |x: &str| {
        EN_TENS
            .iter()
            .position(|e| *e == x)
            .or_else(|| DE_TENS.iter().position(|e| *e == x))
            .map(|i| (i as u32 + 2) * 10)
    };
    if let Some(n) = tens(&w) {
        return Some(n);
    }
    // "twenty-one", "twenty one"
    if let Some((a, b)) = w.split_once(['-', ' '])
        && let (Some(t), Some(u)) = (tens(a), small(b).filter(|u| (1..10).contains(u)))
    {
        return Some(t + u as u32);
    }
    // "einundzwanzig"
    if let Some((a, b)) = w.split_once("und")
        && let (Some(u), Some(t)) = (small(a).filter(|u| (1..10).contains(u)), tens(b))
    {
        return Some(t + u as u32);
    }
    None
}

/// "erstes", "zweite", "dritter", "zwanzigstes", "first", "second" … → 1, 2, 3, 20, 1, 2
fn ordinal_word(word: &str) -> Option<u32> {
    let w = word.to_lowercase();
    const SPECIAL: [(&str, u32); 10] = [
        ("erst", 1),
        ("dritt", 3),
        ("siebt", 7),
        ("acht", 8),
        ("first", 1),
        ("second", 2),
        ("third", 3),
        ("fifth", 5),
        ("eighth", 8),
        ("ninth", 9),
    ];
    let stem = w
        .trim_end_matches(['s', 'r', 'n', 'm'])
        .trim_end_matches('e')
        .to_string();
    if let Some((_, n)) = SPECIAL.iter().find(|(s, _)| stem == *s || w == *s) {
        return Some(*n);
    }
    for end in ["st", "t"] {
        if let Some(base) = stem.strip_suffix(end)
            && let Some(n) = number_word(base)
        {
            return Some(n);
        }
    }
    if let Some(base) = w.strip_suffix("th") {
        return number_word(base).or_else(|| number_word(&format!("{base}e")));
    }
    None
}

fn capitalized(word: &str) -> String {
    let mut c = word.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
        .unwrap_or_default()
}

/// "Chapter One - The Worst Birthday" → "Chapter 1: The Worst Birthday"; "Erstes Kapitel" → "Kapitel 1".
fn chapter_form(name: &str) -> String {
    let rest_of = |r: &str| {
        r.trim_start_matches(|c: char| SEP.contains(c) || c.is_whitespace())
            .trim()
            .to_string()
    };
    let join = |word: &str, n: u32, rest: String| {
        if rest.is_empty() {
            format!("{} {n}", capitalized(word))
        } else {
            format!("{} {n}: {rest}", capitalized(word))
        }
    };
    if let Some(c) = CHAPTER_START.captures(name) {
        let word = c.get(1).map_or("", |m| m.as_str());
        let after = c.get(2).map_or("", |m| m.as_str());
        // words with their end in `after`
        let mut tokens: Vec<(&str, usize)> = Vec::new();
        let mut from = 0;
        for tok in after.split_whitespace() {
            let at = after[from..].find(tok).map_or(from, |i| from + i);
            from = at + tok.len();
            tokens.push((tok, from));
        }
        // the number may be one or two words ("Twenty One"), a hyphenated word or digits
        for take in [2usize, 1] {
            if tokens.len() < take {
                continue;
            }
            let candidate = tokens[..take].iter().map(|t| t.0).collect::<Vec<_>>().join(" ");
            let clean = candidate.trim_end_matches(|c: char| SEP.contains(c));
            if let Some(n) = number_word(clean) {
                return join(word, n, rest_of(&after[tokens[take - 1].1..]));
            }
        }
        return name.to_string();
    }
    if let Some(c) = ORDINAL_CHAPTER.captures(name)
        && let Some(n) = c.get(1).and_then(|m| ordinal_word(m.as_str()))
    {
        let word = c.get(2).map_or("", |m| m.as_str());
        return join(word, n, rest_of(c.get(3).map_or("", |m| m.as_str())));
    }
    name.to_string()
}

// -- rule 6: check and fall back ------------------------------------------------------------------

/// Junk: empty, only a number, "Track 3", a bare chapter word – counts towards "everything is junk".
fn junk(name: &str) -> bool {
    let n = name.trim();
    n.is_empty() || !n.chars().any(char::is_alphanumeric) || MURK.is_match(n) || BARE_WORD.is_match(n)
}

/// Says nothing beyond its number: junk, or a plain "Kapitel 3" / "Chapter 3" / "Teil 3" – these are named
/// "Kapitel N" in the book's language, but they are not junk.
fn says_nothing(name: &str) -> bool {
    junk(name) || is_generic(name.trim())
}

const MAX_LEN: usize = 80;

fn settle(names: Vec<String>, counters: &[Option<u64>], lang: &str) -> Vec<String> {
    let n = names.len();
    let nums: Vec<Option<u128>> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            NUMBER
                .find(name)
                .map(|m| m.as_str().parse().unwrap_or(u128::MAX))
                .or_else(|| counters.get(i).copied().flatten().map(u128::from))
        })
        .collect();
    let counted = nums.iter().all(Option::is_some) && nums.windows(2).all(|w| w[1] > w[0]);
    let number = |i: usize| if counted { nums[i].unwrap_or(0) as usize } else { i + 1 };
    let empty: Vec<bool> = names.iter().map(|x| says_nothing(x)).collect();
    let rubbish: Vec<bool> = names.iter().map(|x| junk(x)).collect();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (name, e) in names.iter().zip(&empty) {
        if !e {
            *seen.entry(name.to_lowercase()).or_default() += 1;
        }
    }
    let double: Vec<bool> = names
        .iter()
        .zip(&empty)
        .map(|(x, e)| !e && seen.get(&x.to_lowercase()).is_some_and(|c| *c > 1))
        .collect();
    let bad = (0..n).filter(|i| rubbish[*i] || double[*i]).count();
    if bad * 2 > n {
        return (0..n).map(|i| crate::encoder::chapter_name(number(i), lang)).collect();
    }
    names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            if empty[i] {
                crate::encoder::chapter_name(number(i), lang)
            } else if double[i] {
                shorten(&format!("{}: {name}", number(i)))
            } else {
                shorten(&name)
            }
        })
        .collect()
}

/// Longer than 80 characters: cut at the end of a word, with "…".
fn shorten(name: &str) -> String {
    if name.chars().count() <= MAX_LEN {
        return name.to_string();
    }
    let cut: String = name.chars().take(MAX_LEN - 1).collect();
    let at = cut.rfind(' ').filter(|i| *i > MAX_LEN / 2).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end_matches(is_sep))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn clean(names: &[&str], known: &[&str], lang: &str) -> Vec<String> {
        let known: Vec<String> = known.iter().map(|k| k.to_string()).collect();
        clean_titles(names, &known, lang).names
    }

    #[test]
    fn repeated_book_title_and_track_numbers_go() {
        crate::text::set_language("de");
        let hp = [
            "01 - Harry Potter and the Chamber of Secrets (Full-Cast Edition): Opening Credits",
            "02 - Harry Potter and the Chamber of Secrets (Full-Cast Edition): Chapter One - The Worst Birthday",
            "03 - Harry Potter and the Chamber of Secrets (Full-Cast Edition): Chapter Two - Dobby's Warning",
            "04 - Harry Potter and the Chamber of Secrets (Full-Cast Edition): Chapter Three - The Burrow",
            "05 - Harry Potter and the Chamber of Secrets (Full-Cast Edition): Chapter Twenty-One - Dumbledore's Return",
            "06 - Harry Potter and the Chamber of Secrets (Full-Cast Edition): End Credits",
        ];
        let want = [
            "Opening Credits",
            "Chapter 1: The Worst Birthday",
            "Chapter 2: Dobby's Warning",
            "Chapter 3: The Burrow",
            "Chapter 21: Dumbledore's Return",
            "End Credits",
        ];
        // without knowing the book (rule 4: the text every track repeats) …
        assert_eq!(clean(&hp, &[], ""), want);
        // … and with it (rule 3: the book title, whatever brackets it has)
        let cleaned = clean_titles(&hp, &["Harry Potter and the Chamber of Secrets".to_string()], "en");
        assert_eq!(cleaned.names, want);
        assert!(cleaned.repetitive);
        // most tracks share it, the credits don't (80 % is enough)
        let mut mixed: Vec<String> = (1..=9).map(|i| format!("{i:02} - Das Buch: Kapitel {i}")).collect();
        mixed.insert(0, "00 - Vorspann".into());
        let got = clean(&mixed.iter().map(String::as_str).collect::<Vec<_>>(), &[], "de");
        assert_eq!(got[0], "Vorspann");
        assert_eq!(got[3], "Kapitel 3");
    }

    #[test]
    fn counters_go_only_when_they_count() {
        crate::text::set_language("de");
        assert_eq!(
            clean(&["CD1-01 Anfang", "CD1-02 Mitte", "CD2-01 Ende"], &[], ""),
            ["Anfang", "Mitte", "Ende"]
        );
        assert_eq!(
            clean(&["Track 1 - Prolog", "Track 2 - Der Sturm"], &[], ""),
            ["Prolog", "Der Sturm"]
        );
        // a number that does not count up belongs to the name
        // (the shared "1984" goes; "Teil 1" alone says nothing and is named like every plain chapter)
        assert_eq!(
            clean(&["1984 - Teil Eins: Ankunft", "1984 - Teil Zwei: Flucht"], &[], ""),
            ["Teil 1: Ankunft", "Teil 2: Flucht"]
        );
        assert_eq!(
            clean(&["1984 - Teil Eins", "1984 - Teil Zwei"], &[], ""),
            ["Kapitel 1", "Kapitel 2"]
        );
        assert_eq!(
            clean(&["20.000 Meilen", "Die Nautilus"], &[], ""),
            ["20.000 Meilen", "Die Nautilus"]
        );
    }

    #[test]
    fn chapter_words_and_number_words() {
        crate::text::set_language("de");
        assert_eq!(chapter_form("Chapter Twenty One - Back"), "Chapter 21: Back");
        assert_eq!(chapter_form("Kapitel Einundzwanzig: Heimweg"), "Kapitel 21: Heimweg");
        assert_eq!(chapter_form("Erstes Kapitel – Die Ankunft"), "Kapitel 1: Die Ankunft");
        assert_eq!(chapter_form("Drittes Kapitel"), "Kapitel 3");
        assert_eq!(chapter_form("KAPITEL 12 - Ende"), "Kapitel 12: Ende");
        assert_eq!(chapter_form("Part One: The Boy Who Lived"), "Part 1: The Boy Who Lived");
        // no number after the word: unchanged
        assert_eq!(chapter_form("Kapitel der Wahrheit"), "Kapitel der Wahrheit");
        assert_eq!(chapter_form("Opening Credits"), "Opening Credits");
    }

    #[test]
    fn nothing_too_much_and_a_safe_fallback() {
        crate::text::set_language("de");
        // the same first words without a separator are part of the names
        let tag = ["Der Tag, an dem es regnete", "Der Tag, an dem es schneite"];
        assert_eq!(clean(&tag, &[], ""), tag);
        // collections with one-word titles stay
        assert_eq!(
            clean(&["Montag", "Dienstag", "Mittwoch"], &[], ""),
            ["Montag", "Dienstag", "Mittwoch"]
        );
        // a shared bare chapter word stays
        assert_eq!(
            clean(&["Kapitel 1: Anfang", "Kapitel 2: Ende"], &[], ""),
            ["Kapitel 1: Anfang", "Kapitel 2: Ende"]
        );
        // doubles get their number, nonsense falls back to numbers in the book's language
        assert_eq!(
            clean(&["Prolog", "Zwischenspiel", "Zwischenspiel", "Epilog"], &[], ""),
            ["Prolog", "2: Zwischenspiel", "3: Zwischenspiel", "Epilog"]
        );
        assert_eq!(
            clean(&["x", "x", "x"], &[], "en"),
            ["Chapter 1", "Chapter 2", "Chapter 3"]
        );
        assert_eq!(clean(&["Track 01", "Track 02"], &[], "en"), ["Chapter 1", "Chapter 2"]);
        assert_eq!(clean(&["Chapter 1", "Chapter 2"], &[], ""), ["Chapter 1", "Chapter 2"]); // language from the names
        // very long names are cut at a word
        let long = "Ein sehr langer Kapitelname der einfach nicht aufhören will und immer weiter geht bis zum Ende";
        let got = clean(&[long, "Kurz"], &[], "");
        assert!(got[0].ends_with('…') && got[0].chars().count() <= 80, "{}", got[0]);
        // the author at the start, "(Ungekürzt)" at the end
        assert_eq!(
            clean(
                &[
                    "Theodor Storm - Der Anfang (Ungekürzt)",
                    "Theodor Storm - Das Ende (Ungekürzt)"
                ],
                &["Theodor Storm"],
                ""
            ),
            ["Der Anfang", "Das Ende"]
        );
    }

    #[test]
    fn absurd_titles_do_no_harm() {
        let title = "Ä".repeat(200_000);
        assert_eq!(chapter_titles(&["01 - Anfang", "02 - Ende"], &title).len(), 2);
    }

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
