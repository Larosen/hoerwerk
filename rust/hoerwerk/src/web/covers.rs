//! Covers of online results (Audible, BookBeat, Apple Books, Open Library …), cached on the server: the browser
//! never talks to the shops' image hosts, the pictures load faster, and the page can read them (for the colour of
//! a book) because they come from the same origin.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use bytes::Bytes;
use sha2::{Digest, Sha256};

use super::{Reply, Req, error};
use crate::app::App;
use crate::text::t;
use crate::{audible, media};

const MAX_URL: usize = 2000;
/// At most this many cached covers; when it gets more, the oldest are removed down to `PRUNE_TO`.
const KEEP: usize = 3000;
const PRUNE_TO: usize = 2500;

fn key(url: &str) -> String {
    hex::encode(Sha256::digest(url.as_bytes()))
}

fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort();
    for (_, p) in files.iter().take(files.len() - PRUNE_TO) {
        let _ = std::fs::remove_file(p);
    }
}

/// Covers being fetched right now, so a request for the same one waits instead of fetching it a second time.
static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn claim(k: &str) -> bool {
    BUSY.lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(HashSet::new)
        .insert(k.to_string())
}

fn release(k: &str) {
    if let Some(set) = BUSY.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        set.remove(k);
    }
}

/// Fetch the covers of a list in the background, several at once, so the page finds them cached
/// instead of the browser asking for them a few at a time.
pub fn warm(app: &App, urls: Vec<String>) {
    let dir = app.cfg.config_dir.join("covers");
    let todo: Vec<String> = urls
        .into_iter()
        .filter(|u| u.len() <= MAX_URL && u.to_lowercase().starts_with("https://") && !dir.join(key(u)).exists())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(60)
        .collect();
    if todo.is_empty() {
        return;
    }
    let queue = Arc::new(Mutex::new(todo));
    for _ in 0..8 {
        let (q, dir) = (queue.clone(), dir.clone());
        std::thread::spawn(move || {
            loop {
                let next = q.lock().unwrap_or_else(|p| p.into_inner()).pop();
                let Some(url) = next else { break };
                let _ = cached_or_fetch(&dir, &url);
            }
        });
    }
}

/// The cached picture, or fetched now (public hosts only, size-capped, it must really be a JPEG or PNG).
fn cached_or_fetch(dir: &Path, url: &str) -> Option<Vec<u8>> {
    let path = dir.join(key(url));
    if let Ok(data) = std::fs::read(&path)
        && media::image_type(&data).is_some()
    {
        return Some(data);
    }
    let k = key(url);
    if !claim(&k) {
        // somebody else is fetching it: wait for the file
        for _ in 0..300 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(data) = std::fs::read(&path)
                && media::image_type(&data).is_some()
            {
                return Some(data);
            }
            if !BUSY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .is_some_and(|s| s.contains(&k))
            {
                break;
            }
        }
        return std::fs::read(&path).ok().filter(|d| media::image_type(d).is_some());
    }
    let fetched = audible::fetch_image(url).ok().flatten();
    release(&k);
    let data = fetched?;
    if std::fs::create_dir_all(dir).is_ok() {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &data).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        prune(dir);
    }
    Some(data)
}

/// `GET /api/cover?u=<https address>` – signed-in users only; anything that is not a picture is refused.
pub async fn proxy(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let url = req.query.get("u").map(|s| s.trim().to_string()).unwrap_or_default();
    let valid =
        url.len() <= MAX_URL && url.to_lowercase().starts_with("https://") && !url.contains(char::is_whitespace);
    if !valid {
        return error(400, t("Ungültige Daten", &[]));
    }
    let dir = app.cfg.config_dir.join("covers");
    let found = tokio::task::spawn_blocking(move || cached_or_fetch(&dir, &url))
        .await
        .ok()
        .flatten();
    match found {
        Some(data) => match media::image_type(&data) {
            Some(ctype) => Reply::Raw(
                StatusCode::OK,
                ctype,
                Bytes::from(data),
                vec![("Cache-Control".into(), "private, max-age=604800".into())],
            ),
            None => error(502, t("Cover nicht erreichbar", &[])),
        },
        None => error(502, t("Cover nicht erreichbar", &[])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keys_differ_and_are_stable() {
        assert_eq!(key("https://a/b.jpg"), key("https://a/b.jpg"));
        assert_ne!(key("https://a/b.jpg"), key("https://a/c.jpg"));
        assert_eq!(key("x").len(), 64);
    }

    #[test]
    fn prune_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("hw-covers-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..(KEEP + 5) {
            std::fs::write(dir.join(format!("{i:05}")), b"x").unwrap();
        }
        prune(&dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), PRUNE_TO);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
