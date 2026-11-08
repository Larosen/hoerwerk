//! Audio to one chapterised M4B with ffmpeg – every call jailed through hw-run (encoder.py).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::chapters::{chapter_title, chapter_titles};
use crate::media::{self, AUDIO_DEMUXERS, command, ffmpeg_bin, ffprobe_bin, probe_cached};
use crate::quality;
use crate::rx;
use crate::text::t;

const AAC_RATES: [i64; 9] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];
const STD_BITRATES: [i64; 11] = [32, 48, 64, 80, 96, 128, 160, 192, 224, 256, 320];
const COVER_NAMES: [&str; 5] = ["cover", "folder", "front", "albumart", "poster"];
const IMAGE_EXTS: [&str; 3] = [".jpg", ".jpeg", ".png"];
pub const MAX_TEXT_FILE: u64 = 512 * 1024;

#[derive(Debug)]
pub enum EncodeError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::Cancelled => write!(f, "{}", t("Abgebrochen", &[])),
            EncodeError::Failed(m) => write!(f, "{m}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, EncodeError>;

fn fail<T>(m: impl Into<String>) -> Result<T> {
    Err(EncodeError::Failed(m.into()))
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn safe_input() -> Vec<String> {
    s(&["-protocol_whitelist", "file", "-format_whitelist", AUDIO_DEMUXERS])
}

fn own_metadata() -> Vec<String> {
    s(&["-protocol_whitelist", "file", "-f", "ffmetadata"])
}

fn p(path: &Path) -> String {
    path.display().to_string()
}

// -- running ffmpeg -------------------------------------------------------------------------------

/// Starts ffmpeg processes and kills them all on cancel. A cancel can arrive at any moment – also between
/// starting a process and noting it down – so `kill()` leaves a mark, and a process started after (or while)
/// it is killed right away. Each ffmpeg runs in its own process group, so a kill also reaches its children.
pub struct Runner {
    cancelled: Arc<AtomicBool>,
    timeout: Duration,
    procs: Mutex<HashMap<u32, ()>>,
    killed: AtomicBool,
}

pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

fn terminate(pid: u32) {
    // SAFETY: killpg only sends a signal to the group we started
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
}

impl Runner {
    pub fn new(cancelled: Arc<AtomicBool>) -> Self {
        Runner {
            cancelled,
            timeout: Duration::from_secs(6 * 3600),
            procs: Mutex::new(HashMap::new()),
            killed: AtomicBool::new(false),
        }
    }

    fn stop_now(&self) -> bool {
        self.killed.load(Ordering::SeqCst) || self.cancelled.load(Ordering::SeqCst)
    }

    pub fn run(&self, argv: &[String]) -> Result<Output> {
        if self.stop_now() {
            return Err(EncodeError::Cancelled);
        }
        let mut child: Child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| EncodeError::Failed(format!("{}: {e}", argv[0])))?;
        let pid = child.id();
        let late = {
            let mut procs = self.procs.lock().unwrap_or_else(|p| p.into_inner());
            procs.insert(pid, ());
            self.stop_now() // cancelled while it was starting: kill() missed it
        };
        if late {
            terminate(pid);
        }
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(o) = out.as_mut() {
                let _ = o.read_to_end(&mut b);
            }
            b
        });
        let ereader = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = err.as_mut() {
                let mut b = Vec::new();
                let _ = e.read_to_end(&mut b);
                s = String::from_utf8_lossy(&b).into_owned();
            }
            s
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) if start.elapsed() < self.timeout => {
                    if self.stop_now() {
                        terminate(pid);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    terminate(pid);
                    let _ = child.wait();
                    break None;
                }
                Err(_) => break None,
            }
        };
        self.procs.lock().unwrap_or_else(|p| p.into_inner()).remove(&pid);
        let (stdout, stderr) = (reader.join().unwrap_or_default(), ereader.join().unwrap_or_default());
        if self.stop_now() {
            return Err(EncodeError::Cancelled);
        }
        let Some(status) = status else {
            return fail(t(
                "ffmpeg hat nach {hours} h nicht geantwortet – abgebrochen",
                &[("hours", &(self.timeout.as_secs() / 3600).to_string())],
            ));
        };
        Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }

    pub fn kill(&self) {
        let procs = self.procs.lock().unwrap_or_else(|p| p.into_inner());
        self.killed.store(true, Ordering::SeqCst);
        for pid in procs.keys() {
            terminate(*pid);
        }
    }
}

fn ffmpeg(args: Vec<String>, read: &[&Path], write: &[&Path], nice: &str) -> Result<Vec<String>> {
    command(&ffmpeg_bin(), &args, read, write, nice).map_err(EncodeError::Failed)
}

fn err_tail(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| t("unbekannter Fehler", &[]))
}

// -- files ------------------------------------------------------------------------------------------

/// cover.jpg/folder.jpg/... or, failing that, the only image in the folder.
pub fn find_cover(folder: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(folder).ok()?;
    let images: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .filter(|n| IMAGE_EXTS.iter().any(|x| n.to_lowercase().ends_with(x)))
        .filter(|n| {
            media::is_regular(&folder.join(n))
                && std::fs::metadata(folder.join(n)).is_ok_and(|m| m.len() < 20 * 1024 * 1024)
        })
        .collect();
    let lower: HashMap<String, &String> = images.iter().map(|n| (n.to_lowercase(), n)).collect();
    for base in COVER_NAMES {
        for ext in IMAGE_EXTS {
            if let Some(n) = lower.get(&format!("{base}{ext}")) {
                return Some(folder.join(n));
            }
        }
    }
    (images.len() == 1).then(|| folder.join(&images[0]))
}

/// libfdk_aac if this ffmpeg has it, otherwise the (good) native encoder.
pub fn ffmpeg_encoder() -> String {
    let argv = match command(&ffmpeg_bin(), &s(&["-hide_banner", "-encoders"]), &[], &[], "") {
        Ok(a) => a,
        Err(_) => return "aac".into(),
    };
    match media::run_output(&argv, Duration::from_secs(20)) {
        Some((_, out, _)) if String::from_utf8_lossy(&out).contains(" libfdk_aac ") => "libfdk_aac".into(),
        _ => "aac".into(),
    }
}

// -- chapters -----------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub start: f64,
    pub title: String,
}

impl Chapter {
    pub fn json(&self) -> Value {
        json!({"start": self.start, "title": self.title})
    }
}

fn seconds(h: Option<&str>, m: &str, s: &str) -> f64 {
    h.and_then(|h| h.parse::<f64>().ok()).unwrap_or(0.0) * 3600.0
        + m.parse::<f64>().unwrap_or(0.0) * 60.0
        + s.replace(',', ".").parse::<f64>().unwrap_or(0.0)
}

/// mp4chaps format (`00:01:02.500 Title`) or OGM (`CHAPTER01=…` / `CHAPTER01NAME=…`).
pub fn parse_chapters_txt(text: &str) -> Vec<Chapter> {
    let time = rx!(r"^\s*(?:(\d+):)?(\d{1,2}):(\d{1,2}(?:[.,]\d+)?)\s+(.*\S)?\s*$");
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut ogm: BTreeMap<i64, (Option<f64>, Option<String>)> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(m) = rx::search(rx!(r"(?i)^CHAPTER(\d+)(NAME)?\s*=\s*(.*)$"), line) {
            let key: i64 = m[1].parse().unwrap_or(0);
            let entry = ogm.entry(key).or_default();
            if m.get(2).is_some() {
                entry.1 = Some(m[3].trim().to_string());
            } else if let Some(tm) = rx::search(time, &format!("{} x", &m[3])) {
                entry.0 = Some(seconds(tm.get(1).map(|x| x.as_str()), &tm[2], &tm[3]));
            }
            continue;
        }
        if let Some(tm) = rx::search(time, line) {
            chapters.push(Chapter {
                start: seconds(tm.get(1).map(|x| x.as_str()), &tm[2], &tm[3]),
                title: tm.get(4).map(|x| x.as_str().trim().to_string()).unwrap_or_default(),
            });
        }
    }
    if !ogm.is_empty() {
        chapters = ogm
            .into_values()
            .filter_map(|(st, ti)| {
                st.map(|start| Chapter {
                    start,
                    title: ti.unwrap_or_default(),
                })
            })
            .collect();
    }
    chapters.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    for (i, c) in chapters.iter_mut().enumerate() {
        if c.title.is_empty() {
            c.title = t("Kapitel {n}", &[("n", &(i + 1).to_string())]);
        }
    }
    chapters
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn ffmetadata(chapters: &[Chapter], total: f64, tags: &[(&str, &str)]) -> String {
    let mut lines = vec![";FFMETADATA1".to_string()];
    for (k, v) in tags {
        if !v.is_empty() {
            lines.push(format!("{k}={}", esc(v)));
        }
    }
    for (i, c) in chapters.iter().enumerate() {
        let start = (c.start * 1000.0).round_ties_even() as i64;
        let end = (chapters.get(i + 1).map_or(total, |n| n.start) * 1000.0).round_ties_even() as i64;
        if end <= start {
            continue;
        }
        lines.extend([
            "[CHAPTER]".into(),
            "TIMEBASE=1/1000".into(),
            format!("START={start}"),
            format!("END={end}"),
            format!("title={}", esc(&c.title)),
        ]);
    }
    lines.join("\n") + "\n"
}

/// Chapters embedded in a file (start in seconds, title).
pub fn file_chapters(path: &Path) -> Vec<Chapter> {
    let mut args = s(&["-v", "error"]);
    args.extend(safe_input());
    args.extend(s(&["-print_format", "json", "-show_chapters"]));
    args.push(p(path));
    let Ok(argv) = command(&ffprobe_bin(), &args, &[path], &[], "") else {
        return Vec::new();
    };
    let data: Value = media::run_output(&argv, Duration::from_secs(60))
        .and_then(|(_, o, _)| serde_json::from_slice(&o).ok())
        .unwrap_or(json!({}));
    data["chapters"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, c)| {
            let title = c["tags"]["title"].as_str().unwrap_or("").trim().to_string();
            Chapter {
                start: c["start_time"]
                    .as_str()
                    .and_then(|x| x.parse().ok())
                    .or_else(|| c["start_time"].as_f64())
                    .unwrap_or(0.0),
                title: if title.is_empty() {
                    t("Kapitel {n}", &[("n", &(i + 1).to_string())])
                } else {
                    title
                },
            }
        })
        .collect()
}

// -- choosing the format ---------------------------------------------------------------------------------

fn bit_rate(i: &Value) -> i64 {
    i["bit_rate"].as_i64().unwrap_or(0)
}

/// `auto` follows the source, never above `cap`; otherwise the fixed value.
pub fn pick_bitrate(setting: &str, infos: &[Value], channels: i64, cap: Option<i64>) -> String {
    let setting = setting.trim().to_lowercase();
    let setting = if setting.is_empty() {
        "auto".to_string()
    } else {
        setting
    };
    if setting != "auto" {
        return setting;
    }
    let mut rates: Vec<i64> = infos.iter().map(bit_rate).filter(|r| *r != 0).collect();
    rates.sort();
    let src = if rates.is_empty() {
        128
    } else {
        rates[rates.len() / 2] / 1000
    };
    let cap = cap.filter(|c| *c != 0).unwrap_or(if channels > 1 { 128 } else { 64 });
    let target = src.min(cap);
    let best = STD_BITRATES
        .iter()
        .copied()
        .min_by_key(|b| ((b - target).abs(), *b))
        .unwrap_or(128);
    format!("{}k", best.max(32))
}

/// The most common sample rate, moved to the nearest one AAC supports.
pub fn pick_rate(infos: &[Value]) -> i64 {
    let mut counts: Vec<(i64, usize)> = Vec::new();
    for r in infos
        .iter()
        .filter_map(|i| i["sample_rate"].as_i64())
        .filter(|r| *r != 0)
    {
        match counts.iter_mut().find(|(x, _)| *x == r) {
            Some(e) => e.1 += 1,
            None => counts.push((r, 1)),
        }
    }
    // Counter.most_common(1): the highest count, the first seen among equals
    let rate = counts.iter().fold(None::<(i64, usize)>, |best, c| match best {
        Some(b) if b.1 >= c.1 => Some(b),
        _ => Some(*c),
    });
    let rate = rate.map_or(44100, |r| r.0).min(48000);
    AAC_RATES
        .iter()
        .copied()
        .min_by_key(|r| ((r - rate).abs(), -r))
        .unwrap_or(44100)
}

pub struct Merge<'a> {
    pub bitrate: &'a str,
    pub mono: bool,
    pub jobs: usize,
    pub chapter_mode: &'a str,
    pub album: &'a str,
    pub titles: Option<&'a [String]>,
    pub nice: &'a str,
    pub profile: &'a str,
    pub drama: bool,
}

/// One chapter per file – or the file's own chapters if it has several.
fn chapter_list(files: &[String], paths: &[PathBuf], infos: &[Value], lengths: &[f64], m: &Merge) -> Vec<Chapter> {
    let mut names: Vec<String> = files
        .iter()
        .enumerate()
        .map(|(n, rel)| chapter_title(rel, infos[n]["tags"]["title"].as_str(), m.chapter_mode))
        .collect();
    if m.chapter_mode == "auto" {
        names = chapter_titles(&names, m.album);
    }
    let mut chapters = Vec::new();
    let mut pos = 0.0;
    for n in 0..files.len() {
        let length = lengths[n];
        let own = m
            .titles
            .filter(|t| t.len() == files.len())
            .map(|t| t[n].trim().to_string())
            .unwrap_or_default();
        let inner = if own.is_empty() {
            file_chapters(&paths[n])
        } else {
            Vec::new()
        };
        if inner.len() > 1 {
            let d = infos[n]["duration"].as_f64().filter(|d| *d != 0.0).unwrap_or(length);
            let scale = length / d;
            chapters.extend(inner.into_iter().map(|c| Chapter {
                start: pos + c.start * scale,
                title: c.title,
            }));
        } else {
            chapters.push(Chapter {
                start: pos,
                title: if own.is_empty() { names[n].clone() } else { own },
            });
        }
        pos += length;
    }
    chapters
}

/// Exact length of a raw AAC segment: packets x 1024 samples.
fn adts_seconds(path: &Path, rate: i64) -> f64 {
    let args = s(&[
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "aac",
        "-count_packets",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=nb_read_packets",
        "-of",
        "csv=p=0",
    ])
    .into_iter()
    .chain([p(path)])
    .collect::<Vec<_>>();
    let out = command(&ffprobe_bin(), &args, &[path], &[], "")
        .ok()
        .and_then(|a| media::run_output(&a, Duration::from_secs(600)));
    match out.and_then(|(_, o, _)| {
        String::from_utf8_lossy(&o)
            .trim()
            .split(',')
            .next()
            .and_then(|x| x.parse::<f64>().ok())
    }) {
        Some(packets) => packets * 1024.0 / rate as f64,
        None => probe_cached(path)["duration"].as_f64().unwrap_or(0.0),
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(parent: &Path, prefix: &str) -> Result<Self> {
        use rand::RngCore;
        let mut b = [0u8; 6];
        rand::rng().fill_bytes(&mut b);
        let dir = parent.join(format!("{prefix}{}", hex::encode(b)));
        std::fs::create_dir_all(&dir).map_err(|e| EncodeError::Failed(e.to_string()))?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| EncodeError::Failed(e.to_string()))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

pub struct Merged {
    pub duration: f64,
    pub chapters: Vec<Chapter>,
    pub bitrate: String,
    pub copied: bool,
}

/// Encode `files` (relative to `folder`) into one chapterised M4B; `out_path` appears only on success.
pub fn merge_to_m4b(
    folder: &Path,
    files: &[String],
    out_path: &Path,
    m: &Merge,
    progress: &(dyn Fn(f64, String) + Sync),
    runner: &Runner,
) -> Result<Merged> {
    if files.is_empty() {
        return fail(t("Keine Audiodateien gefunden", &[]));
    }
    let paths: Vec<PathBuf> = files.iter().map(|f| folder.join(f)).collect();
    let infos: Vec<Value> = paths.iter().map(|x| probe_cached(x)).collect();
    let bad: Vec<&str> = files
        .iter()
        .zip(&infos)
        .filter(|(_, i)| i["sample_rate"].as_i64().unwrap_or(0) == 0)
        .map(|(f, _)| f.as_str())
        .take(3)
        .collect();
    if !bad.is_empty() {
        return fail(t("Keine Tonspur in: {files}", &[("files", &bad.join(", "))]));
    }
    let codecs: HashSet<&str> = infos.iter().map(|i| i["audio_codec"].as_str().unwrap_or("")).collect();
    let shapes: HashSet<(i64, i64)> = infos
        .iter()
        .map(|i| {
            (
                i["sample_rate"].as_i64().unwrap_or(0),
                i["channels"].as_i64().unwrap_or(0),
            )
        })
        .collect();
    if codecs.len() == 1
        && quality::keep_codec(codecs.iter().next().copied().unwrap_or(""), m.profile)
        && !m.mono
        && shapes.len() == 1
    {
        return pack_as_is(files, &paths, &infos, out_path, m, progress, runner);
    }
    let channels = if m.mono {
        1
    } else {
        infos
            .iter()
            .map(|i| i["channels"].as_i64().filter(|c| *c != 0).unwrap_or(2))
            .max()
            .unwrap_or(2)
            .min(2)
    };
    let rate = pick_rate(&infos);
    let br = pick_bitrate(
        m.bitrate,
        &infos,
        channels,
        Some(quality::cap(m.profile, m.drama, channels)),
    );
    // AAC sources that already match: copy instead of transcoding (no quality loss)
    let copy = infos.iter().all(|i| {
        i["audio_codec"] == "aac"
            && i["sample_rate"].as_i64() == Some(rate)
            && i["channels"].as_i64().filter(|c| *c != 0).unwrap_or(2) == channels
    });
    let codec = ffmpeg_encoder();
    let total_src: f64 = infos.iter().map(|i| i["duration"].as_f64().unwrap_or(0.0)).sum::<f64>();
    let total_src = if total_src == 0.0 { 1.0 } else { total_src };
    let work = TempDir::new(out_path.parent().unwrap_or(Path::new(".")), ".m4b-")?;
    let segs: Vec<PathBuf> = (0..paths.len()).map(|n| work.0.join(format!("{n:05}.aac"))).collect();
    let done = Mutex::new(0.0f64);
    let what = |n: usize| {
        let codec_name = if copy {
            t("AAC kopieren", &[])
        } else {
            format!("AAC {br}")
        };
        let ch = if channels == 1 { "Mono" } else { "Stereo" };
        let rate_s = rate.to_string();
        if n == 1 {
            t(
                "1 Datei, {codec}, {rate} Hz, {channels}",
                &[("codec", &codec_name), ("rate", &rate_s), ("channels", ch)],
            )
        } else {
            t(
                "{n} Dateien, {codec}, {rate} Hz, {channels}",
                &[
                    ("n", &n.to_string()),
                    ("codec", &codec_name),
                    ("rate", &rate_s),
                    ("channels", ch),
                ],
            )
        }
    };
    progress(0.0, what(paths.len()));
    let encode = |n: usize| -> Result<()> {
        let mut args = s(&["-nostdin", "-hide_banner", "-v", "error", "-y"]);
        args.extend(safe_input());
        args.extend(["-i".into(), p(&paths[n])]);
        args.extend(s(&["-map", "0:a:0", "-vn", "-sn", "-dn", "-map_metadata", "-1"]));
        if copy {
            args.extend(s(&["-c:a", "copy"]));
        } else {
            args.extend([
                "-c:a".into(),
                codec.clone(),
                "-b:a".into(),
                br.clone(),
                "-ar".into(),
                rate.to_string(),
                "-ac".into(),
                channels.to_string(),
            ]);
        }
        args.extend(["-f".into(), "adts".into(), p(&segs[n])]);
        let res = runner.run(&ffmpeg(args, &[&paths[n]], &[&work.0], m.nice)?)?;
        let ok = res.code == 0 && std::fs::metadata(&segs[n]).is_ok_and(|x| x.len() > 0);
        if !ok {
            return fail(format!("{}: {}", files[n], err_tail(&res.stderr)));
        }
        let mut d = done.lock().unwrap_or_else(|p| p.into_inner());
        *d += infos[n]["duration"].as_f64().unwrap_or(0.0);
        let (k, total) = ((n + 1).to_string(), paths.len().to_string());
        let msg = if copy {
            t("Kopiere Datei {n}/{total}", &[("n", &k), ("total", &total)])
        } else {
            t("Kodiere Datei {n}/{total}", &[("n", &k), ("total", &total)])
        };
        progress(0.95 * *d / total_src, msg);
        Ok(())
    };
    // a pool of `jobs` workers taking the files in order; the first error stops the others
    let next = Mutex::new(0usize);
    let first_err: Mutex<Option<EncodeError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..m.jobs.max(1).min(paths.len()) {
            scope.spawn(|| {
                loop {
                    let n = {
                        let mut i = next.lock().unwrap_or_else(|p| p.into_inner());
                        if *i >= paths.len() || first_err.lock().unwrap_or_else(|p| p.into_inner()).is_some() {
                            return;
                        }
                        *i += 1;
                        *i - 1
                    };
                    if let Err(e) = encode(n) {
                        let mut fe = first_err.lock().unwrap_or_else(|p| p.into_inner());
                        if fe.is_none() {
                            *fe = Some(e);
                            runner.kill();
                        }
                        return;
                    }
                }
            });
        }
    });
    if let Some(e) = first_err.into_inner().unwrap_or_else(|p| p.into_inner()) {
        return Err(if runner.cancelled.load(Ordering::SeqCst) {
            EncodeError::Cancelled
        } else {
            e
        });
    }
    let lengths: Vec<f64> = segs.iter().map(|x| adts_seconds(x, rate)).collect();
    let chapters = chapter_list(files, &paths, &infos, &lengths, m);
    let total: f64 = lengths.iter().sum();
    let meta = work.0.join("meta.txt");
    std::fs::write(&meta, ffmetadata(&chapters, total, &[("genre", "Audiobook")]))
        .map_err(|e| EncodeError::Failed(e.to_string()))?;
    let concat = format!("concat:{}", segs.iter().map(|x| p(x)).collect::<Vec<_>>().join("|"));
    let part = work.0.join("out.m4b");
    progress(0.96, t("Erzeuge M4B …", &[]));
    let mut args = s(&[
        "-nostdin",
        "-hide_banner",
        "-v",
        "error",
        "-y",
        "-protocol_whitelist",
        "file,concat",
        "-f",
        "aac",
        "-i",
    ]);
    args.push(concat);
    args.extend(own_metadata());
    args.extend(["-i".into(), p(&meta)]);
    args.extend(s(&[
        "-map",
        "0:a",
        "-map_metadata",
        "1",
        "-map_chapters",
        "1",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
    ]));
    args.push(p(&part));
    let res = runner.run(&ffmpeg(args, &[&work.0], &[&work.0], m.nice)?)?;
    if res.code != 0 {
        return fail(t(
            "Zusammenfügen fehlgeschlagen: {error}",
            &[("error", &err_tail(&res.stderr))],
        ));
    }
    move_file(&part, out_path)?;
    progress(1.0, t("Fertig", &[]));
    Ok(Merged {
        duration: total,
        chapters,
        bitrate: if copy { "copy".into() } else { br },
        copied: copy,
    })
}

/// Dolby (E-AC-3 / AC-3) or lossless ALAC: the audio goes into the M4B unchanged – no re-encode, no downmix.
fn pack_as_is(
    files: &[String],
    paths: &[PathBuf],
    infos: &[Value],
    out_path: &Path,
    m: &Merge,
    progress: &(dyn Fn(f64, String) + Sync),
    runner: &Runner,
) -> Result<Merged> {
    let codec = infos[0]["audio_codec"].as_str().unwrap_or("").to_uppercase();
    progress(
        0.0,
        if paths.len() > 1 {
            t(
                "{n} Dateien, {codec} unverändert übernommen",
                &[("n", &paths.len().to_string()), ("codec", &codec)],
            )
        } else {
            t("1 Datei, {codec} unverändert übernommen", &[("codec", &codec)])
        },
    );
    let work = TempDir::new(out_path.parent().unwrap_or(Path::new(".")), ".m4b-")?;
    let lengths: Vec<f64> = infos.iter().map(|i| i["duration"].as_f64().unwrap_or(0.0)).collect();
    let chapters = chapter_list(files, paths, infos, &lengths, m);
    let total: f64 = lengths.iter().sum();
    let meta = work.0.join("meta.txt");
    std::fs::write(&meta, ffmetadata(&chapters, total, &[("genre", "Audiobook")]))
        .map_err(|e| EncodeError::Failed(e.to_string()))?;
    let listing = work.0.join("files.txt");
    let text: String = paths
        .iter()
        .map(|x| {
            format!(
                "file '{}'\n",
                std::path::absolute(x)
                    .unwrap_or(x.clone())
                    .display()
                    .to_string()
                    .replace('\'', "'\\''")
            )
        })
        .collect();
    std::fs::write(&listing, text).map_err(|e| EncodeError::Failed(e.to_string()))?;
    let part = work.0.join("out.m4b");
    let mut args = s(&[
        "-nostdin",
        "-hide_banner",
        "-v",
        "error",
        "-y",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "concat,mov",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]);
    args.push(p(&listing));
    args.extend(own_metadata());
    args.extend(["-i".into(), p(&meta)]);
    args.extend(s(&[
        "-map",
        "0:a",
        "-map_metadata",
        "1",
        "-map_chapters",
        "1",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
    ]));
    args.push(p(&part));
    let dirs: std::collections::BTreeSet<PathBuf> =
        paths.iter().filter_map(|x| x.parent().map(Path::to_path_buf)).collect();
    let mut read: Vec<&Path> = vec![&work.0];
    read.extend(dirs.iter().map(PathBuf::as_path));
    let res = runner.run(&ffmpeg(args, &read, &[&work.0], m.nice)?)?;
    if res.code != 0 || !part.exists() {
        return fail(t(
            "Zusammenfügen fehlgeschlagen: {error}",
            &[("error", &err_tail(&res.stderr))],
        ));
    }
    move_file(&part, out_path)?;
    progress(1.0, t("Fertig", &[]));
    Ok(Merged {
        duration: total,
        chapters,
        bitrate: "copy".into(),
        copied: true,
    })
}

/// Copy the audio of `src` into a fresh M4B, optionally with new chapters (tags and cover: the tagger).
pub fn remux(src: &Path, dst: &Path, chapters: Option<&[Chapter]>, runner: &Runner, nice: &str) -> Result<()> {
    let work = TempDir::new(dst.parent().unwrap_or(Path::new(".")), ".remux-")?;
    let mut args = s(&["-nostdin", "-hide_banner", "-v", "error", "-y"]);
    args.extend(safe_input());
    args.extend(["-i".into(), p(src)]);
    if let Some(ch) = chapters {
        let total = probe_cached(src)["duration"]
            .as_f64()
            .filter(|d| *d != 0.0)
            .unwrap_or_else(|| ch.last().map_or(0.0, |c| c.start + 1.0));
        // a chapters.txt for another edition can name chapters behind the end of this file: drop those
        let mut kept: Vec<Chapter> = ch.iter().filter(|c| c.start < total - 0.5).cloned().collect();
        if kept.is_empty() {
            kept = ch.iter().take(1).cloned().collect();
        }
        let meta = work.0.join("meta.txt");
        std::fs::write(&meta, ffmetadata(&kept, total, &[])).map_err(|e| EncodeError::Failed(e.to_string()))?;
        args.extend(own_metadata());
        args.extend(["-i".into(), p(&meta)]);
        args.extend(s(&["-map", "0:a:0", "-map_metadata", "0", "-map_chapters", "1"]));
    } else {
        args.extend(s(&["-map", "0:a:0", "-map_metadata", "0", "-map_chapters", "0"]));
    }
    let part = work.0.join("out.m4b");
    args.extend(s(&["-c", "copy", "-movflags", "+faststart", "-f", "mp4"]));
    args.push(p(&part));
    let res = runner.run(&ffmpeg(args, &[src], &[&work.0], nice)?)?;
    if res.code != 0 {
        return fail(format!("Remux fehlgeschlagen: {}", err_tail(&res.stderr)));
    }
    move_file(&part, dst)
}

/// Embedded picture of an audio file -> image file, or None.
pub fn extract_cover(path: &Path, dest_dir: &Path) -> Option<PathBuf> {
    let codec = probe_cached(path)["cover_codec"].as_str().map(str::to_string)?;
    if codec != "mjpeg" && codec != "png" {
        return None;
    }
    let out = dest_dir.join(format!("embedded-cover.{}", if codec == "png" { "png" } else { "jpg" }));
    let mut args = s(&["-nostdin", "-v", "error", "-y"]);
    args.extend(safe_input());
    args.extend(["-i".into(), p(path)]);
    args.extend(s(&["-map", "0:v:0", "-c:v", "copy", "-frames:v", "1"]));
    args.push(p(&out));
    let argv = command(&ffmpeg_bin(), &args, &[path], &[dest_dir], "").ok()?;
    media::run_output(&argv, Duration::from_secs(60));
    std::fs::metadata(&out).is_ok_and(|m| m.len() > 0).then_some(out)
}

/// Chapters without real names, numbered 1, 2, 3 … in the book's language (encoder.numbered_chapters).
pub fn numbered_chapters(chapters: &[Chapter], lang: &str) -> Vec<Chapter> {
    let mut ordered = chapters.to_vec();
    ordered.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    ordered
        .into_iter()
        .enumerate()
        .map(|(i, c)| Chapter {
            title: chapter_name(i + 1, lang),
            ..c
        })
        .collect()
}

/// "Chapter 3" for an English book, "Kapitel 3" for a German one.
pub fn chapter_name(n: usize, lang: &str) -> String {
    let word = match lang.to_lowercase().chars().take(2).collect::<String>().as_str() {
        "en" => Some("Chapter"),
        "de" => Some("Kapitel"),
        "fr" => Some("Chapitre"),
        "es" => Some("Capítulo"),
        "it" => Some("Capitolo"),
        "nl" => Some("Hoofdstuk"),
        _ => None,
    };
    match word {
        Some(w) => format!("{w} {n}"),
        None => t("Kapitel {n}", &[("n", &n.to_string())]),
    }
}

pub fn looks_generic(chapters: &[Chapter]) -> bool {
    crate::chapters::looks_generic(&chapters.iter().map(|c| c.title.as_str()).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_txt_and_metadata() {
        crate::text::set_language("de");
        // like Python: a time without a title is no chapter line
        let c =
            parse_chapters_txt("\u{feff}00:00:00.000 Anfang\n# Kommentar\n1:02:03,5 Mitte\n00:10:00 \n00:20:00 x\n");
        assert_eq!(
            c,
            vec![
                Chapter {
                    start: 0.0,
                    title: "Anfang".into()
                },
                Chapter {
                    start: 1200.0,
                    title: "x".into()
                },
                Chapter {
                    start: 3723.5,
                    title: "Mitte".into()
                }
            ]
        );
        let ogm = parse_chapters_txt(
            "CHAPTER02=00:05:00.000\nCHAPTER02NAME=Zwei\nCHAPTER01=00:00:00.000\nCHAPTER01NAME=Eins",
        );
        assert_eq!(
            ogm.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            ["Eins", "Zwei"]
        );
        let m = ffmetadata(&ogm, 600.0, &[("title", "a=b;c")]);
        assert!(m.contains("title=a\\=b\\;c") && m.contains("START=300000\nEND=600000"));
        assert_eq!(
            pick_bitrate(
                "auto",
                &[json!({"bit_rate": 64000}), json!({"bit_rate": 320000})],
                2,
                Some(160)
            ),
            "160k"
        );
        assert_eq!(pick_bitrate("auto", &[], 1, None), "64k");
        assert_eq!(
            pick_rate(&[
                json!({"sample_rate": 22050}),
                json!({"sample_rate": 96000}),
                json!({"sample_rate": 96000})
            ]),
            48000
        );
        assert_eq!(numbered_chapters(&ogm, "en")[1].title, "Chapter 2");
    }

    #[test]
    fn runner_kills_a_cancelled_process() {
        let flag = Arc::new(AtomicBool::new(false));
        let r = Arc::new(Runner::new(flag.clone()));
        let r2 = r.clone();
        let t0 = Instant::now();
        let h = std::thread::spawn(move || r2.run(&s(&["sh", "-c", "sleep 30 & sleep 30"])));
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, Ordering::SeqCst);
        assert!(matches!(h.join().unwrap(), Err(EncodeError::Cancelled)));
        assert!(t0.elapsed() < Duration::from_secs(5));
        assert!(matches!(r.run(&s(&["true"])), Err(EncodeError::Cancelled)));
    }
}
