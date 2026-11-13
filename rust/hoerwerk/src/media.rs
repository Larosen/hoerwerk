//! Audio files: which files count, ffprobe through the hw-run jail (cached by modification time), tags and covers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::words::natural_path_cmp;

pub const AUDIO_EXTS: [&str; 16] = [
    "mp3", "m4a", "m4b", "mp4", "aac", "flac", "ogg", "oga", "opus", "wav", "wma", "aif", "aiff", "mka", "ape", "alac",
];
/// Containers ffprobe/ffmpeg may open: audio only (docker/build-ffmpeg.sh builds exactly these).
pub const AUDIO_DEMUXERS: &str = "mp3,mov,aac,flac,ogg,wav,w64,aiff,asf,matroska,ape,wv,caf,mpc,mpc8,tta,au,amr,dsf";

fn ext_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 && !base[..i].chars().all(|c| c == '.') => base[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn is_audio(name: &str) -> bool {
    !name.starts_with('.') && AUDIO_EXTS.contains(&ext_of(name).as_str())
}

/// A real file – symlinks in downloads could point at /config or /etc and are ignored.
pub fn is_regular(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

pub fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// All audio files below `folder` (relative paths, natural order, CD1 before CD2); symlinked folders are not followed.
pub fn collect_audio(folder: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(folder.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let sub = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if is_real_dir(&path) {
                if !name.starts_with(['.', '_']) {
                    stack.push((path, sub));
                }
            } else if is_audio(&name) && is_regular(&path) {
                out.push(sub);
            }
        }
    }
    out.sort_by(|a, b| natural_path_cmp(a, b));
    out
}

// -- the jail -------------------------------------------------------------------------------------

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub fn sandbox_mode() -> String {
    env_or("FFMPEG_SANDBOX", "auto").trim().to_lowercase()
}

/// Path of hw-run: HW_RUN, next to this program, /usr/local/bin or PATH ("" = not installed).
pub fn hw_run() -> String {
    let want = std::env::var("HW_RUN").unwrap_or_default();
    if want.eq_ignore_ascii_case("off") {
        return String::new();
    }
    if !want.is_empty() {
        return if is_executable(Path::new(&want)) {
            want
        } else {
            String::new()
        };
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(d);
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.extend(
        std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs.iter()
        .map(|d| d.join("hw-run"))
        .find(|p| is_executable(p))
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// argv that runs `binary args` jailed to `read`/`write` (sandbox.command). Without hw-run: `require` refuses.
pub fn command(
    binary: &str,
    args: &[String],
    read: &[&Path],
    write: &[&Path],
    nice: &str,
) -> Result<Vec<String>, String> {
    let mode = sandbox_mode();
    let mut argv: Vec<String> = Vec::new();
    if mode == "off" {
        if !nice.is_empty() && nice != "0" {
            argv.extend(["nice".into(), "-n".into(), nice.into()]);
        }
    } else {
        let launcher = hw_run();
        if launcher.is_empty() {
            if mode == "require" {
                return Err("hw-run fehlt (FFMPEG_SANDBOX=require)".into());
            }
        } else {
            argv.push(launcher);
            for p in read {
                argv.extend(["--read".into(), absolute(p)]);
            }
            for p in write {
                argv.extend(["--write".into(), absolute(p)]);
            }
            if !nice.is_empty() && nice != "0" {
                argv.extend(["--nice".into(), nice.into()]);
            }
            argv.push("--".into());
        }
    }
    argv.push(binary.into());
    argv.extend(args.iter().cloned());
    Ok(argv)
}

fn absolute(p: &Path) -> String {
    std::path::absolute(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .display()
        .to_string()
}

/// How ffmpeg is jailed (sandbox.status): hw-run answers with --status.
pub fn sandbox_status() -> Value {
    let mode = sandbox_mode();
    let launcher = hw_run();
    if launcher.is_empty() {
        return json!({"mode": mode, "landlock_abi": 0, "active": false, "network_blocked": false, "launcher": "none", "seccomp": false});
    }
    let out = run_output(&[launcher, "--status".into()], Duration::from_secs(10))
        .and_then(|(_, o, _)| serde_json::from_slice::<Value>(&o).ok())
        .unwrap_or(json!({}));
    let abi = out["landlock_abi"].as_i64().unwrap_or(0);
    let reason = out["reason"].as_str().unwrap_or("").to_string();
    json!({"mode": mode, "landlock_abi": abi, "reason": reason, "active": mode != "off" && abi > 0, "network_blocked": mode != "off",
           "launcher": "hw-run", "seccomp": mode != "off"})
}

static FFMPEG_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// "7.1" & co. – or "nicht gefunden" (asked once).
pub fn ffmpeg_version() -> String {
    FFMPEG_VERSION
        .get_or_init(|| {
            let first = command(&ffmpeg_bin(), &["-hide_banner".into(), "-version".into()], &[], &[], "")
                .ok()
                .and_then(|a| run_output(&a, Duration::from_secs(20)))
                .map(|(_, o, _)| String::from_utf8_lossy(&o).lines().next().unwrap_or("").to_string())
                .unwrap_or_default();
            let v = first.replace("ffmpeg version ", "");
            let v = v.split(" Copyright").next().unwrap_or("").to_string();
            if v.is_empty() {
                crate::text::t("nicht gefunden", &[])
            } else {
                v
            }
        })
        .clone()
}

pub fn ffmpeg_bin() -> String {
    env_or("FFMPEG", "ffmpeg")
}

pub fn ffprobe_bin() -> String {
    env_or("FFPROBE", "ffprobe")
}

/// Run argv, give up after `timeout`; stdout as bytes (None on failure to start or time out).
pub fn run_output(argv: &[String], timeout: Duration) -> Option<(i32, Vec<u8>, String)> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut b);
        b
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    Some((
        status.code().unwrap_or(-1),
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    ))
}

/// {duration, tags, bit_rate, sample_rate, channels, audio_codec, cover_codec} – like meta.probe.
pub fn probe(path: &Path) -> Value {
    let args: Vec<String> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        AUDIO_DEMUXERS,
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([path.display().to_string()])
    .collect();
    let data: Value = command(&ffprobe_bin(), &args, &[path], &[], "")
        .ok()
        .and_then(|argv| run_output(&argv, Duration::from_secs(60)))
        .and_then(|(_, out, _)| serde_json::from_slice(&out).ok())
        .unwrap_or(json!({}));
    let fmt = &data["format"];
    let mut tags: Map<String, Value> = fmt["tags"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect())
        .unwrap_or_default();
    let num = |v: &Value| {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| v.as_f64())
            .unwrap_or(0.0)
    };
    let mut info = json!({"duration": num(&fmt["duration"]), "bit_rate": num(&fmt["bit_rate"]) as i64,
                          "sample_rate": null, "channels": null, "cover_codec": null, "audio_codec": null});
    for s in data["streams"].as_array().cloned().unwrap_or_default() {
        if s["codec_type"] == "audio" && info["sample_rate"].is_null() {
            let rate = num(&s["sample_rate"]) as i64;
            info["sample_rate"] = if rate > 0 { json!(rate) } else { Value::Null };
            info["channels"] = s["channels"].clone();
            info["audio_codec"] = s["codec_name"].clone();
            for (k, v) in s["tags"].as_object().cloned().unwrap_or_default() {
                tags.entry(k.to_lowercase()).or_insert(v);
            }
        } else if s["codec_type"] == "video"
            && info["cover_codec"].is_null()
            && (s["disposition"]["attached_pic"].as_i64().unwrap_or(0) != 0
                || s["codec_name"] == "mjpeg"
                || s["codec_name"] == "png")
        {
            info["cover_codec"] = s["codec_name"].clone();
        }
    }
    info["tags"] = Value::Object(tags);
    info
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, (f64, Value)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn mtime(path: &Path) -> Option<f64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64())
}

/// probe() cached by modification time (meta.cache.probe); {} when the file is gone.
pub fn probe_cached(path: &Path) -> Value {
    let Some(mt) = mtime(path) else { return json!({}) };
    if let Some((t, v)) = CACHE.lock().unwrap_or_else(|p| p.into_inner()).get(path)
        && *t == mt
    {
        return v.clone();
    }
    let v = probe(path);
    let mut c = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if c.len() > 20000 {
        c.clear();
    }
    c.insert(path.to_path_buf(), (mt, v.clone()));
    v
}

/// The tags of an M4B as a book (tagger.read_book).
pub fn read_book(path: &Path) -> Value {
    hw_tags::read_book(path)
}

/// The picture inside an audio file: M4B/M4A from the tags, MP3/FLAC through the jailed ffmpeg.
pub fn embedded_cover(path: &Path) -> Option<Vec<u8>> {
    match ext_of(&path.display().to_string()).as_str() {
        "m4b" | "m4a" | "mp4" => hw_tags::embedded_cover(path),
        "mp3" | "flac" => {
            let args: Vec<String> = [
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                AUDIO_DEMUXERS,
                "-i",
            ]
            .iter()
            .map(|s| s.to_string())
            .chain([path.display().to_string()])
            .chain(
                [
                    "-map",
                    "0:v:0",
                    "-c",
                    "copy",
                    "-frames:v",
                    "1",
                    "-f",
                    "image2pipe",
                    "pipe:1",
                ]
                .iter()
                .map(|s| s.to_string()),
            )
            .collect();
            let argv = command(&ffmpeg_bin(), &args, &[path], &[], "").ok()?;
            let (code, out, _) = run_output(&argv, Duration::from_secs(30))?;
            (code == 0 && !out.is_empty()).then_some(out)
        }
        _ => None,
    }
}

pub fn image_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_files() {
        assert!(
            is_audio("01.MP3") && is_audio("a.b.m4b") && !is_audio(".x.mp3") && !is_audio("run.sh") && !is_audio("mp3")
        );
        let dir = std::env::temp_dir().join(format!("hw-media-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in [
            "CD2/01.mp3",
            "CD1/10.mp3",
            "CD1/2.mp3",
            ".hidden/x.mp3",
            "_skip/y.mp3",
            "cover.jpg",
        ] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        std::os::unix::fs::symlink("/etc/hostname", dir.join("link.mp3")).unwrap();
        assert_eq!(collect_audio(&dir), ["CD1/2.mp3", "CD1/10.mp3", "CD2/01.mp3"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
