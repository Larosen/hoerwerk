//! How good a book should sound: radio play or reading, and how much bitrate each may keep.

use serde_json::Value;

use crate::rx;
use crate::rx::{is_match, sub};

/// cap in kbit/s for stereo when re-encoding: (reading, radio play); mono gets half
fn caps(profile: &str) -> (i64, i64) {
    match profile {
        "standard" => (128, 192),
        "original" => (320, 320),
        _ => (160, 256),
    }
}

/// A radio play rather than a reading – from the shop data, the release or folder name, or many speakers.
pub fn is_drama(book: &Value, name: &str) -> bool {
    let field = |k: &str| match &book[k] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let text = format!(
        "{} {}",
        ["title", "subtitle", "format_type", "publisher"].map(field).join(" "),
        name
    );
    let text = sub(rx!(r"[._]+"), &text, " ");
    if is_match(
        rx!(
            r"(?i)h(?:ö|oe)rspiel|dramati[sz]|full[\s-]?cast|volle[rn]?\s+besetzung|radio\s?(?:play|drama)|audio\s?drama|inszenierte\s+lesung"
        ),
        &text,
    ) {
        return true;
    }
    let genres = book["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(book["tags"].as_array().into_iter().flatten());
    if genres.filter_map(Value::as_str).any(|g| {
        is_match(
            rx!(r"(?i)h(?:ö|oe)rspiel|radio\s?drama|dramatized|drama\s*&\s*plays"),
            g,
        )
    }) {
        return true;
    }
    if matches!(
        field("format_type").to_lowercase().as_str(),
        "dramatized" | "radio_play" | "original_recording"
    ) {
        return true;
    }
    book["narrators"].as_array().map_or(0, |a| {
        a.iter().filter(|n| n.as_str().is_some_and(|s| !s.is_empty())).count()
    }) >= 4
}

/// Highest bitrate (kbit/s) a re-encode may use.
pub fn cap(profile: &str, drama: bool, channels: i64) -> i64 {
    let (reading, play) = caps(profile);
    let top = if drama { play } else { reading };
    if channels > 1 { top } else { (top / 2).max(64) }
}

/// Audio that is packed into the M4B as it is instead of being re-encoded.
pub fn keep_codec(codec: &str, profile: &str) -> bool {
    matches!(codec, "eac3" | "ac3") || (profile == "original" && codec == "alac")
}
