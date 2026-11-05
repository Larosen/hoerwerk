//! hw-tags write BOOK.m4b --book BOOK.json [--settings SETTINGS.json] [--cover COVER.jpg]
//! Writes the audiobook tags (see the library). Exit code 0 on success, 1 with a message otherwise.

use std::path::PathBuf;
use std::process::ExitCode;

fn read_json(path: &str) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut it = args.into_iter();
    if it.next().as_deref() != Some("write") {
        return Err("usage: hw-tags write BOOK.m4b --book BOOK.json [--settings S.json] [--cover C.jpg]".into());
    }
    let file = PathBuf::from(it.next().ok_or("no M4B given")?);
    let (mut book, mut settings, mut cover) = (None, serde_json::json!({}), None);
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--book" => book = Some(read_json(&value)?),
            "--settings" => settings = read_json(&value)?,
            "--cover" => cover = Some(std::fs::read(&value).map_err(|e| format!("{value}: {e}"))?),
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    let book = book.ok_or("--book is missing")?;
    hw_tags::write(&file, &book, &settings, cover).map_err(|e| format!("{}: {e}", file.display()))
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hw-tags: {e}");
            ExitCode::from(1)
        }
    }
}
