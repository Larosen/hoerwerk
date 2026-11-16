//! Python-style regular expressions (lookarounds, Unicode \w \d \b, re.I) for the ported text rules.

pub use fancy_regex::{Captures, Regex};

/// A regex compiled once: `rx!(r"…")`.
#[macro_export]
macro_rules! rx {
    ($p:expr) => {{
        static R: std::sync::LazyLock<fancy_regex::Regex> =
            std::sync::LazyLock::new(|| fancy_regex::Regex::new($p).expect("valid regex"));
        &*R
    }};
}

pub fn is_match(re: &Regex, text: &str) -> bool {
    re.is_match(text).unwrap_or(false)
}

pub fn search<'t>(re: &Regex, text: &'t str) -> Option<Captures<'t, str>> {
    re.captures(text).ok().flatten()
}

/// re.sub with a replacement template in Rust syntax ("$1", "${1}x"). A pattern that runs into the backtracking
/// limit on a pathological text (thousands of characters) leaves the text as it is – `replace_all` would panic, and
/// with `panic = "abort"` that ends the whole server.
pub fn sub(re: &Regex, text: &str, rep: &str) -> String {
    re.try_replacen(text, 0, rep)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| text.to_string())
}

/// re.sub, at most once.
pub fn sub1(re: &Regex, text: &str, rep: &str) -> String {
    re.try_replacen(text, 1, rep)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| text.to_string())
}

/// re.sub with a function for each match – the same safety as `sub`.
pub fn sub_with(re: &Regex, text: &str, rep: impl FnMut(&Captures<str>) -> String) -> String {
    re.try_replacen(text, 0, rep)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| text.to_string())
}

/// re.findall with one group (or the whole match without groups).
pub fn findall(re: &Regex, text: &str) -> Vec<String> {
    re.captures_iter(text)
        .filter_map(Result::ok)
        .map(|c| {
            c.get(1)
                .or_else(|| c.get(0))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// re.split (pattern without groups).
pub fn split(re: &Regex, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0;
    for m in re.find_iter(text).filter_map(Result::ok) {
        if m.start() == m.end() && m.start() == last && m.start() != 0 {
            continue;
        }
        out.push(text[last..m.start()].to_string());
        last = m.end();
    }
    out.push(text[last..].to_string());
    out
}

/// A literal inside a pattern (re.escape).
pub fn escape(s: &str) -> String {
    fancy_regex::escape(s).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pathological_text_does_not_end_the_server() {
        // thousands of word characters run some lookaround patterns into the backtracking limit: no panic
        let long = "a".repeat(5000);
        let _ = crate::audible::clean_query(&long);
        let _ = crate::filing::sanitize(&long);
        let _ = crate::releases::search_stages(&serde_json::json!({"title": long, "authors": [long]}));
    }
}
