//! ISBNs: a book or CD is often known by its ISBN instead of an Audible ASIN – in the `isbn` tag, or written into the
//! ASIN field (an old ISBN-10 looks like an ASIN). Both forms are recognised by their check digit and compared as
//! ISBN-13, which is also the EAN barcode on the CD.

/// The ISBN-13 of a text that is an ISBN (ISBN-10 or ISBN-13, with or without hyphens and spaces, "ISBN" in front
/// allowed) – None for anything else, also for numbers whose check digit does not fit.
pub fn normalize(text: &str) -> Option<String> {
    let t = text.trim();
    let t = t
        .strip_prefix("ISBN")
        .or_else(|| t.strip_prefix("isbn"))
        .unwrap_or(t)
        .trim_start_matches([':', ' ']);
    let raw: String = t.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
    let up = raw.to_uppercase();
    match up.len() {
        10 if valid10(&up) => Some(to13(&up[..9])),
        13 if (up.starts_with("978") || up.starts_with("979")) && valid13(&up) => Some(up),
        _ => None,
    }
}

/// An Audible ASIN that is really an ISBN (a ten-digit number with a fitting check digit, not "B0…").
pub fn in_asin(asin: &str) -> Option<String> {
    let a = asin.trim();
    if a.len() != 10 || a.to_uppercase().starts_with('B') {
        return None;
    }
    normalize(a)
}

/// The first ISBN in some texts (tags, a search, a folder name): a text that is an ISBN, an ISBN-10 in an ASIN field,
/// or one written inside ("ISBN 978-3-…").
pub fn find_in(texts: &[&str]) -> Option<String> {
    for text in texts {
        if let Some(i) = normalize(text) {
            return Some(i);
        }
        for word in text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '[' | ']' | ':')) {
            if let Some(i) = normalize(word) {
                return Some(i);
            }
        }
    }
    None
}

fn valid10(s: &str) -> bool {
    let b = s.as_bytes();
    if !b[..9].iter().all(u8::is_ascii_digit) || !(b[9].is_ascii_digit() || b[9] == b'X') {
        return false;
    }
    let sum: u32 = b
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let v = if *c == b'X' { 10 } else { u32::from(c - b'0') };
            v * (10 - i as u32)
        })
        .sum();
    sum.is_multiple_of(11)
}

fn check13(first12: &str) -> u32 {
    let sum: u32 = first12
        .bytes()
        .enumerate()
        .map(|(i, c)| u32::from(c - b'0') * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    (10 - sum % 10) % 10
}

fn valid13(s: &str) -> bool {
    s.bytes().all(|c| c.is_ascii_digit()) && check13(&s[..12]) == u32::from(s.as_bytes()[12] - b'0')
}

fn to13(nine: &str) -> String {
    let first12 = format!("978{nine}");
    format!("{first12}{}", check13(&first12))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn_10_and_13_are_one_number() {
        // "Eine kurze Weltgeschichte für junge Leser" (from the library): ISBN-10 in the ASIN field
        assert_eq!(normalize("3866100981").as_deref(), Some("9783866100985"));
        assert_eq!(normalize("ISBN 978-3-86610-098-5").as_deref(), Some("9783866100985"));
        assert_eq!(normalize("386610099X").as_deref(), Some("9783866100992"));
        assert_eq!(normalize("3-86610-098-1"), normalize("9783866100985"));
        // wrong check digit, Audible ASIN, other EAN: no ISBN
        assert_eq!(normalize("3866100982"), None);
        assert_eq!(normalize("B0D2RZPYT9"), None);
        assert_eq!(normalize("4006680012345"), None);
        assert_eq!(in_asin("3866100981").as_deref(), Some("9783866100985"));
        assert_eq!(in_asin("B004V5411E"), None);
        assert_eq!(
            find_in(&["", "Hörbuch, ISBN 978-3-86610-098-5, 2 CDs"]).as_deref(),
            Some("9783866100985")
        );
        assert_eq!(find_in(&["Der Schimmelreiter 1888"]), None);
    }
}
