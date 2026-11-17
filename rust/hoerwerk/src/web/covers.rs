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

/// Amazon's image hosts hand out a smaller picture when the address says so ("…/I/51abc._SL300_.jpg"); the
/// stored addresses carry no size, so a list gets 300 px and the detail column 600 px instead of the original.
/// Other hosts are left as they are.
fn sized(url: &str, width: u32) -> String {
    let Some((head, tail)) = url.split_once("/images/I/") else {
        return url.to_string();
    };
    let host = head.trim_start_matches("https://");
    let amazon = host.ends_with(".media-amazon.com") || host.ends_with(".ssl-images-amazon.com");
    match tail.rsplit_once('.') {
        Some((id, ext))
            if amazon
                && matches!(ext, "jpg" | "png")
                && !id.is_empty()
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '%' || c == '-') =>
        {
            format!("{head}/images/I/{id}._SL{width}_.{ext}")
        }
        _ => url.to_string(),
    }
}

/// The list size is the default for `warm`, so the page finds exactly these files in the cache.
pub const LIST_WIDTH: u32 = 300;
pub const BIG_WIDTH: u32 = 600;

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
        .filter(|u| u.len() <= MAX_URL && u.to_lowercase().starts_with("https://"))
        .map(|u| sized(&u, LIST_WIDTH))
        .filter(|u| !dir.join(key(u)).exists())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(60)
        .collect();
    if todo.is_empty() {
        return;
    }
    for url in todo {
        enqueue(&dir, url);
    }
}

/// Background fetches wait here; a fixed set of workers takes them, so the number of threads and of waiting
/// jobs stays bounded however often lists are shown. When the queue is full, new jobs are dropped: the page then
/// loads those covers through `proxy` one by one instead.
const WORKERS: usize = 6;
const QUEUE_MAX: usize = 240;
type Job = (std::path::PathBuf, String);
static QUEUE: Mutex<std::collections::VecDeque<Job>> = Mutex::new(std::collections::VecDeque::new());
static WAKE: std::sync::Condvar = std::sync::Condvar::new();
static STARTED: std::sync::Once = std::sync::Once::new();

fn enqueue(dir: &Path, url: String) {
    STARTED.call_once(|| {
        for _ in 0..WORKERS {
            std::thread::spawn(|| {
                loop {
                    let job = {
                        let mut q = QUEUE.lock().unwrap_or_else(|p| p.into_inner());
                        loop {
                            if let Some(j) = q.pop_front() {
                                break j;
                            }
                            q = WAKE.wait(q).unwrap_or_else(|p| p.into_inner());
                        }
                    };
                    let _ = cached_or_fetch(&job.0, &job.1);
                }
            });
        }
    });
    let mut q = QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    if q.len() < QUEUE_MAX && !q.iter().any(|(_, u)| *u == url) {
        q.push_back((dir.to_path_buf(), url));
        WAKE.notify_one();
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
    if let Some(data) = &fetched
        && std::fs::create_dir_all(dir).is_ok()
    {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, data).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        prune(dir);
    }
    // only now: a request that waited for this fetch finds the file
    release(&k);
    fetched
}

/// `GET /api/cover?u=<https address>` – signed-in users only; anything that is not a picture is refused.
pub async fn proxy(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let url = req.query.get("u").map(|s| s.trim().to_string()).unwrap_or_default();
    let valid =
        url.len() <= MAX_URL && url.to_lowercase().starts_with("https://") && !url.contains(char::is_whitespace);
    if !valid {
        return error(400, t("Ungültige Daten", &[]));
    }
    let url = match req.query.get("w").map(String::as_str) {
        Some("300") => sized(&url, LIST_WIDTH),
        Some("600") => sized(&url, BIG_WIDTH),
        _ => url,
    };
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
    fn amazon_covers_get_a_size_others_do_not() {
        let a = "https://m.media-amazon.com/images/I/51AbC-d%2B.jpg";
        assert_eq!(
            sized(a, 300),
            "https://m.media-amazon.com/images/I/51AbC-d%2B._SL300_.jpg"
        );
        assert_eq!(
            sized(a, 600),
            "https://m.media-amazon.com/images/I/51AbC-d%2B._SL600_.jpg"
        );
        let done = "https://m.media-amazon.com/images/I/51AbC._SL500_.jpg";
        assert_eq!(sized(done, 300), done); // already sized: left alone
        let other = "https://covers.openlibrary.org/b/id/1-L.jpg";
        assert_eq!(sized(other, 300), other);
        let evil = "https://evil.example/images/I/51AbC.jpg";
        assert_eq!(sized(evil, 300), evil);
    }

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
