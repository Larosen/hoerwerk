//! Upload from the browser into the inbox: file by file, streamed to disk into a hidden staging
//! folder `<inbox>/.upload-<id>/`; only when finished, each top-level folder is moved into the inbox in one step.
//! Relative paths only, only audio, covers and the text files Hörwerk understands – checked by name *and* first
//! bytes (a program renamed to .mp3 is refused). Limits per file, upload and account; a size is reserved before
//! the first byte is written. Finishing leaves a marker, so a crash in the middle is completed at the next start.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use rand::RngCore;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::media::AUDIO_EXTS;
use crate::text::{self, now, t};

pub const MAX_FILE: u64 = 4 << 30;
const RESERVE: u64 = 512 << 20;
const STALE: f64 = 24.0 * 3600.0;
const TEXT_NAMES: [&str; 6] = [
    "chapters.txt",
    "metadata.yml",
    "metadata.yaml",
    "metadata.json",
    "desc.txt",
    "reader.txt",
];
const MAX_SESSIONS: usize = 8;
const MAX_SESSIONS_PER_USER: usize = 3;
const MAX_SESSION_BYTES: u64 = 64 << 30;
const MAX_USER_BYTES: u64 = 128 << 30;
const MAX_FILES: usize = 3000;
const FINISHING: &str = ".finishing";
const TEXT_EXTS: [&str; 6] = ["cue", "txt", "yml", "yaml", "json", "nfo"];
const IMAGE_EXTS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
const MAX_TEXT: u64 = 1 << 20;
const DANGEROUS: [(&[u8], &str); 15] = [
    (b"MZ", "exe"),
    (b"\x7fELF", "elf"),
    (b"%PDF", "pdf"),
    (b"PK\x03\x04", "zip"),
    (b"Rar!", "archive"),
    (b"7z\xbc\xaf\x27\x1c", "archive"),
    (b"\x1f\x8b", "archive"),
    (b"BZh", "archive"),
    (b"\xfd7zXZ", "archive"),
    (b"#!", "script"),
    (b"\xca\xfe\xba\xbe", "mac"),
    (b"\xcf\xfa\xed\xfe", "mac"),
    (b"\xfe\xed\xfa", "mac"),
    (b"\xd0\xcf\x11\xe0", "office"),
    (b"{\\rtf", "office"),
];

#[derive(Debug)]
pub struct UploadError(pub String);

type Res<T> = Result<T, UploadError>;

fn err<T>(m: String) -> Res<T> {
    Err(UploadError(m))
}

fn what(kind: &str) -> String {
    match kind {
        "exe" => t("ein Windows-Programm (.exe/.dll)", &[]),
        "elf" => t("ein Linux-Programm", &[]),
        "pdf" => t("ein PDF-Dokument", &[]),
        "zip" => t("ein ZIP-Archiv (auch .docx/.jar/.apk)", &[]),
        "archive" => t("ein Archiv", &[]),
        "script" => t("ein Skript", &[]),
        "mac" => t("ein macOS-Programm", &[]),
        _ => t("ein Office-Dokument", &[]),
    }
}

fn is_audio(head: &[u8]) -> bool {
    if head.starts_with(b"ID3")
        || head.starts_with(b"fLaC")
        || head.starts_with(b"OggS")
        || head.get(4..8) == Some(b"ftyp")
    {
        return true;
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        return true;
    }
    if head.starts_with(&hex::decode("3026b2758e66cf11a6d900aa0062ce6c").unwrap_or_default()) {
        return true; // WMA (ASF)
    }
    // MPEG audio / AAC (ADTS) frame sync near the start (some rips begin with a little padding)
    let n = head.len().saturating_sub(1).min(4096);
    (0..n).any(|i| {
        (head[i] == 0xFF && head[i + 1] & 0xE0 == 0xE0 && head[i + 1] & 0x06 != 0x06)
            || (head[i] == 0xFF && matches!(head[i + 1], 0xF1 | 0xF9))
    })
}

fn is_image(head: &[u8]) -> bool {
    head.starts_with(b"\xff\xd8\xff")
        || head.starts_with(b"\x89PNG\r\n\x1a\n")
        || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP"))
}

fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// cp1252 has five unassigned bytes; everything else decodes.
fn cp1252_ok(head: &[u8]) -> bool {
    !head.iter().any(|b| matches!(b, 0x81 | 0x8d | 0x8f | 0x90 | 0x9d))
}

/// The file must really be what its name says: audio, a cover image or a small text file.
pub fn check_content(name: &str, head: &[u8], size: u64) -> Res<()> {
    for (magic, kind) in DANGEROUS {
        if head.starts_with(magic) {
            return err(t(
                "{name} ist {what} – nicht erlaubt",
                &[("name", name), ("what", &what(kind))],
            ));
        }
    }
    let ext = ext_of(name);
    let no_text = || err(t("{name} ist keine Textdatei", &[("name", name)]));
    if TEXT_EXTS.contains(&ext.as_str()) {
        if size > MAX_TEXT || head.contains(&0) {
            return no_text();
        }
        if let Err(e) = std::str::from_utf8(head) {
            // a split character at the very end is fine; old cue sheets are cp1252
            if e.valid_up_to() < head.len().saturating_sub(4) && !cp1252_ok(head) {
                return no_text();
            }
        }
        let start: Vec<u8> = head
            .iter()
            .skip_while(|b| b.is_ascii_whitespace())
            .take(9)
            .map(|b| b.to_ascii_lowercase())
            .collect();
        if start.starts_with(b"<html") || start.starts_with(b"<script") || start.starts_with(b"<!doctype") {
            return no_text();
        }
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        if !is_image(head) {
            return err(t("{name} ist kein Bild (JPEG, PNG oder WebP)", &[("name", name)]));
        }
    } else if !is_audio(head) {
        return err(t("{name} ist keine Audiodatei", &[("name", name)]));
    }
    Ok(())
}

fn valid_part(p: &str) -> bool {
    let n = p.chars().count();
    (1..=200).contains(&n) && !p.chars().any(|c| (c as u32) < 0x20 || "/\\:*?\"<>|".contains(c))
}

/// Path parts of an uploaded file, or an error for anything that could escape or hide.
pub fn clean_path(rel: &str) -> Res<Vec<String>> {
    let norm = rel.replace('\\', "/");
    let parts: Vec<String> = norm
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .map(str::to_string)
        .collect();
    if parts.is_empty() || parts.len() > 6 {
        return err(t("Ungültiger Dateipfad", &[]));
    }
    for p in &parts {
        if p == ".." || p.starts_with(['.', '_']) || !valid_part(p) || p.as_str() != p.trim() {
            return err(t(
                "Ungültiger Dateipfad: {path}",
                &[("path", &rel.chars().take(120).collect::<String>())],
            ));
        }
    }
    let name = &parts[parts.len() - 1];
    let ext = ext_of(name);
    let allowed =
        AUDIO_EXTS.contains(&ext.as_str()) || IMAGE_EXTS.contains(&ext.as_str()) || TEXT_EXTS.contains(&ext.as_str());
    if !allowed
        || (matches!(ext.as_str(), "txt" | "yml" | "yaml" | "json")
            && !TEXT_NAMES.contains(&name.to_lowercase().as_str()))
    {
        return err(t(
            "Dateityp nicht erlaubt: {name}",
            &[("name", &name.chars().take(120).collect::<String>())],
        ));
    }
    Ok(parts)
}

pub fn session_id(v: &str) -> bool {
    v.len() == 16 && v.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

struct Session {
    user: String,
    label: String,
    started: f64,
    files: usize,
    bytes: u64,
    reserved: u64,
    active: usize,
    paths: HashSet<String>,
    state: &'static str,
}

pub struct Uploads {
    inbox: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    finishing: Mutex<()>,
}

fn free_space(p: &Path) -> u64 {
    let Ok(c) = std::ffi::CString::new(p.display().to_string()) else {
        return 0;
    };
    // SAFETY: statvfs only writes into the struct we own
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return 0;
        }
        st.f_bavail as u64 * st.f_frsize as u64
    }
}

impl Uploads {
    pub fn new(inbox: &Path) -> Self {
        Uploads {
            inbox: inbox.to_path_buf(),
            sessions: Mutex::new(HashMap::new()),
            finishing: Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Session>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn dir(&self, uid: &str) -> PathBuf {
        self.inbox.join(format!(".upload-{uid}"))
    }

    /// A new upload for `user` (only this account may add to it, finish or cancel it).
    pub fn start(&self, user: &str, label: &str) -> Res<String> {
        if self.inbox.as_os_str().is_empty() || !self.inbox.is_dir() {
            return err(t("Eingangsordner nicht gefunden", &[]));
        }
        self.cleanup();
        let uid = {
            let mut s = self.lock();
            if s.values().filter(|x| x.state != "cancelled").count() >= MAX_SESSIONS {
                return err(t("Gerade laufen zu viele Uploads – bitte später noch einmal", &[]));
            }
            if s.values().filter(|x| x.user == user && x.state != "cancelled").count() >= MAX_SESSIONS_PER_USER {
                return err(t(
                    "Du hast schon {n} Uploads offen – erst diese abschließen",
                    &[("n", &MAX_SESSIONS_PER_USER.to_string())],
                ));
            }
            let mut b = [0u8; 8];
            rand::rng().fill_bytes(&mut b);
            let uid = hex::encode(b);
            s.insert(
                uid.clone(),
                Session {
                    user: user.into(),
                    label: if label.is_empty() { user.into() } else { label.into() },
                    started: now(),
                    files: 0,
                    bytes: 0,
                    reserved: 0,
                    active: 0,
                    paths: HashSet::new(),
                    state: "open",
                },
            );
            uid
        };
        let made = {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o775).create(self.dir(&uid))
        };
        if let Err(e) = made {
            self.lock().remove(&uid);
            return err(e.to_string());
        }
        Ok(uid)
    }

    fn check_session(&self, uid: &str, owner: &str) -> Res<()> {
        let gone = || err(t("Upload nicht gefunden – bitte neu starten", &[]));
        if !session_id(uid) {
            return gone();
        }
        let ok = self
            .lock()
            .get(uid)
            .is_some_and(|s| s.user == owner && s.state != "cancelled");
        if !ok || !self.dir(uid).is_dir() {
            return gone(); // someone else's upload looks exactly like a missing one
        }
        Ok(())
    }

    /// One file of the upload, streamed from the request body.
    pub async fn put(&self, uid: &str, rel: &str, body: Body, length: u64, owner: &str) -> Res<Value> {
        self.check_session(uid, owner)?;
        let parts = clean_path(rel)?;
        if length == 0 || length > MAX_FILE {
            return err(t(
                "Datei zu groß oder leer: {name}",
                &[("name", &parts[parts.len() - 1])],
            ));
        }
        if free_space(&self.inbox) < length + RESERVE {
            return err(t("Zu wenig Platz im Eingang", &[]));
        }
        let root = std::fs::canonicalize(self.dir(uid)).map_err(|e| UploadError(e.to_string()))?;
        let target = parts.iter().fold(root.clone(), |p, x| p.join(x));
        if !target.starts_with(&root) {
            return err(t("Ungültiger Dateipfad", &[]));
        }
        let key = parts.join("/").to_lowercase();
        self.reserve(uid, &key, length)?;
        let result = self.write(uid, &parts, &target, body, length).await;
        let last_out = {
            let mut s = self.lock();
            let mut last = false;
            if let Some(x) = s.get_mut(uid) {
                x.reserved -= length;
                x.active -= 1;
                if result.is_ok() {
                    x.files += 1;
                    x.bytes += length;
                } else {
                    x.paths.remove(&key);
                }
                last = x.state == "cancelled" && x.active == 0;
            }
            if last {
                s.remove(uid);
            }
            last
        };
        if last_out {
            let _ = std::fs::remove_dir_all(self.dir(uid)); // cancelled meanwhile: the last writer tidies up
        }
        result
    }

    /// Book a file's size and path before writing – all limits checked in one locked step.
    fn reserve(&self, uid: &str, key: &str, length: u64) -> Res<()> {
        let mut all = self.lock();
        let user = all.get(uid).map(|s| s.user.clone()).unwrap_or_default();
        let mine: u64 = all
            .values()
            .filter(|x| x.user == user)
            .map(|x| x.bytes + x.reserved)
            .sum();
        let Some(s) = all.get_mut(uid) else {
            return err(t("Upload abgebrochen", &[]));
        };
        if s.state != "open" {
            return err(t("Dieser Upload wird schon abgeschlossen", &[]));
        }
        if s.paths.contains(key) {
            return err(t(
                "Diese Datei ist schon im Upload: {name}",
                &[(
                    "name",
                    &key.chars()
                        .rev()
                        .take(120)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<String>(),
                )],
            ));
        }
        if s.files + s.active >= MAX_FILES {
            return err(t(
                "Zu viele Dateien in einem Upload (höchstens {n})",
                &[("n", &MAX_FILES.to_string())],
            ));
        }
        if s.bytes + s.reserved + length > MAX_SESSION_BYTES {
            return err(t("Der Upload wird zu groß – bitte in mehreren Teilen hochladen", &[]));
        }
        if mine + length > MAX_USER_BYTES {
            return err(t(
                "Zu viele Daten in deinen offenen Uploads – erst diese abschließen",
                &[],
            ));
        }
        s.paths.insert(key.to_string());
        s.reserved += length;
        s.active += 1;
        Ok(())
    }

    fn still_open(&self, uid: &str) -> Res<()> {
        if self.lock().get(uid).is_some_and(|s| s.state == "open") {
            Ok(())
        } else {
            err(t("Upload abgebrochen", &[]))
        }
    }

    async fn write(&self, uid: &str, parts: &[String], target: &Path, mut body: Body, length: u64) -> Res<Value> {
        let name = &parts[parts.len() - 1];
        {
            // folders are made only while the upload is still open – checked under the lock, so a cancel in
            // between can never be followed by a folder that brings the deleted upload back
            let s = self.lock();
            if !s.get(uid).is_some_and(|x| x.state == "open") {
                return err(t("Upload abgebrochen", &[]));
            }
            std::fs::create_dir_all(target.parent().unwrap_or(Path::new("")))
                .map_err(|e| UploadError(e.to_string()))?;
        }
        if target.exists() {
            return err(t("Diese Datei ist schon im Upload: {name}", &[("name", name)]));
        }
        let mut b = [0u8; 4];
        rand::rng().fill_bytes(&mut b);
        let tmp = PathBuf::from(format!("{}.{}.part", target.display(), hex::encode(b)));
        let done = self.stream(&tmp, name, &mut body, length).await.and_then(|_| {
            let s = self.lock();
            if !s.get(uid).is_some_and(|x| x.state == "open") {
                return err(t("Upload abgebrochen", &[]));
            }
            std::fs::rename(&tmp, target).map_err(|e| UploadError(e.to_string()))
        });
        if let Err(e) = done {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        let s = self.lock();
        let (files, bytes) = s.get(uid).map_or((0, 0), |x| (x.files, x.bytes));
        Ok(json!({"ok": true, "files": files + 1, "bytes": bytes + length}))
    }

    /// The bytes of one file into `tmp`; the start of the file decides – before anything lands on the disk.
    async fn stream(&self, tmp: &Path, name: &str, body: &mut Body, length: u64) -> Res<()> {
        let aborted = || UploadError(t("Upload abgebrochen", &[]));
        let mut file = tokio::fs::File::create(tmp)
            .await
            .map_err(|e| UploadError(e.to_string()))?;
        let mut left = length;
        let mut head: Vec<u8> = Vec::new();
        let mut checked = false;
        while left > 0 {
            let frame = tokio::time::timeout(Duration::from_secs(120), body.frame())
                .await
                .map_err(|_| aborted())?;
            let Some(frame) = frame else { return Err(aborted()) };
            let frame = frame.map_err(|_| aborted())?;
            let Ok(mut data) = frame.into_data() else { continue };
            if data.len() as u64 > left {
                data.truncate(left as usize);
            }
            left -= data.len() as u64;
            if !checked {
                head.extend_from_slice(&data);
                if (head.len() as u64) < 65536.min(length) && left > 0 {
                    continue;
                }
                check_content(name, &head, length)?;
                checked = true;
                file.write_all(&head).await.map_err(|e| UploadError(e.to_string()))?;
                head.clear();
                continue;
            }
            file.write_all(&data).await.map_err(|e| UploadError(e.to_string()))?;
        }
        if !checked {
            check_content(name, &head, length)?;
            file.write_all(&head).await.map_err(|e| UploadError(e.to_string()))?;
        }
        file.flush().await.map_err(|e| UploadError(e.to_string()))?;
        Ok(())
    }

    /// Move every top-level entry of the upload into the inbox (in one step each); returns the names.
    pub fn finish(&self, uid: &str, owner: &str) -> Res<Vec<String>> {
        self.check_session(uid, owner)?;
        {
            let mut s = self.lock();
            let x = s
                .get_mut(uid)
                .ok_or_else(|| UploadError(t("Upload nicht gefunden – bitte neu starten", &[])))?;
            if x.state != "open" {
                return err(t("Dieser Upload wird schon abgeschlossen", &[]));
            }
            if x.active > 0 {
                return err(t("Es kommen noch Dateien an – bitte warten", &[]));
            }
            x.state = "finishing";
        }
        let moved = match self.move_in(&self.dir(uid)) {
            Ok(m) => m,
            Err(e) => {
                if let Some(x) = self.lock().get_mut(uid) {
                    x.state = "open"; // the rest stays in the staging folder: try again
                }
                return Err(e);
            }
        };
        let s = self.lock().remove(uid);
        if let Some(s) = s {
            let files = if s.files == 1 {
                t("1 Datei", &[])
            } else {
                t("{n} Dateien", &[("n", &s.files.to_string())])
            };
            text::info(
                "Eingang",
                &t(
                    "Hochgeladen von {user}: {names} ({files})",
                    &[("user", &s.label), ("names", &moved.join(", ")), ("files", &files)],
                ),
            );
        }
        Ok(moved)
    }

    /// Every top-level entry into the inbox, one atomic rename each; the marker makes it resumable.
    fn move_in(&self, src: &Path) -> Res<Vec<String>> {
        let _g = self.finishing.lock().unwrap_or_else(|p| p.into_inner());
        let io = |e: std::io::Error| UploadError(e.to_string());
        std::fs::write(src.join(FINISHING), b"").map_err(io)?;
        let mut names: Vec<String> = std::fs::read_dir(src)
            .map_err(io)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut moved = Vec::new();
        for name in names {
            if name.ends_with(".part") || name == FINISHING {
                continue;
            }
            let from = src.join(&name);
            let mut dest = self.inbox.join(&name);
            let mut n = 2;
            while std::fs::symlink_metadata(&dest).is_ok() {
                // never overwrite something in the inbox
                let (base, ext) = match (from.is_file(), name.rfind('.')) {
                    (true, Some(i)) if i > 0 => (&name[..i], &name[i..]),
                    _ => (name.as_str(), ""),
                };
                dest = self.inbox.join(format!("{base} ({n}){ext}"));
                n += 1;
            }
            std::fs::rename(&from, &dest).map_err(io)?;
            moved.push(
                dest.file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
        let _ = std::fs::remove_dir_all(src);
        Ok(moved)
    }

    pub fn cancel(&self, uid: &str, owner: &str) -> Res<()> {
        self.check_session(uid, owner)?;
        {
            let mut s = self.lock();
            if s.get(uid).is_some_and(|x| x.state != "open") {
                return err(t("Dieser Upload wird schon abgeschlossen", &[]));
            }
            match s.get_mut(uid) {
                Some(x) if x.active > 0 => {
                    x.state = "cancelled"; // files still arriving fail at their next check; the last one tidies up
                    return Ok(());
                }
                _ => {
                    s.remove(uid);
                }
            }
        }
        let _ = std::fs::remove_dir_all(self.dir(uid));
        Ok(())
    }

    /// At start: half-finished uploads (marker) are completed, everything else removed. Returns moved names.
    pub fn recover(&self) -> Vec<String> {
        let mut all = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.inbox) else {
            return all;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if !name.starts_with(".upload-") || !e.file_type().is_ok_and(|f| f.is_dir()) {
                continue;
            }
            if path.join(FINISHING).exists() {
                match self.move_in(&path) {
                    Ok(moved) => {
                        text::info(
                            "Eingang",
                            &t(
                                "Unterbrochenen Upload abgeschlossen: {names}",
                                &[("names", &moved.join(", "))],
                            ),
                        );
                        all.extend(moved);
                    }
                    Err(e) => text::warn(
                        "Eingang",
                        &t("Upload-Reste nicht aufgeräumt: {error}", &[("error", &e.0)]),
                    ),
                }
            } else {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
        all
    }

    /// Remove staging folders of uploads that never finished (older than a day).
    pub fn cleanup(&self) {
        let Ok(rd) = std::fs::read_dir(&self.inbox) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(uid) = name.strip_prefix(".upload-") else {
                continue;
            };
            let open = {
                let mut s = self.lock();
                if s.get(uid).is_some_and(|x| now() - x.started > STALE && x.active == 0) {
                    s.remove(uid);
                }
                s.contains_key(uid)
            };
            if !open
                && e.file_type().is_ok_and(|f| f.is_dir())
                && crate::media::mtime(&e.path()).is_some_and(|m| now() - m > STALE)
            {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_contents() {
        crate::text::set_language("de");
        assert_eq!(
            clean_path("Autor - Titel/CD1/01.mp3").unwrap(),
            ["Autor - Titel", "CD1", "01.mp3"]
        );
        for bad in [
            "../x.mp3",
            "a/.versteckt.mp3",
            "_x/a.mp3",
            "a/b/c/d/e/f/g.mp3",
            "run.sh",
            "a/notes.txt",
            "a/x .mp3 ",
        ] {
            assert!(clean_path(bad).is_err(), "{bad}");
        }
        // like Python: a leading "/" is no way out – the path stays inside the upload
        assert_eq!(clean_path("/etc/x.mp3").unwrap(), ["etc", "x.mp3"]);
        assert!(clean_path("A/chapters.txt").is_ok() && clean_path("a\\b\\cover.jpg").is_ok());
        assert!(check_content("a.mp3", b"ID3\x04rest", 10).is_ok());
        assert!(
            check_content("a.mp3", b"MZ\x90\x00", 10)
                .unwrap_err()
                .0
                .contains("Windows-Programm")
        );
        assert!(check_content("a.mp3", b"%PDF-1.7", 10).is_err());
        assert!(check_content("a.mp3", b"#!/bin/sh", 10).is_err());
        assert!(check_content("cover.jpg", b"\x89PNG\r\n\x1a\nxx", 10).is_ok());
        assert!(check_content("cover.jpg", b"GIF89a", 10).is_err());
        assert!(check_content("chapters.txt", b"00:00:00 Anfang\n", 10).is_ok());
        assert!(check_content("chapters.txt", b"  <!DOCTYPE html>", 10).is_err());
        assert!(check_content("chapters.txt", b"a\x00b", 10).is_err());
        assert!(check_content("x.cue", b"TITLE \"Gr\xfc\xdfe\"", 20).is_ok()); // cp1252
        assert!(session_id("0123456789abcdef") && !session_id("0123456789ABCDEF") && !session_id("../x"));
    }
}
