"""Audio -> chapterised M4B, the modern replacement for auto-m4b / m4b-tool.

Every source file is encoded on its own (in parallel) to a raw AAC (ADTS)
segment.  The segments are joined byte-wise and muxed into one MP4 without
re-encoding.  That avoids the classic m4b-tool problems:

* no "Non-monotonic DTS" warnings (raw ADTS has no edit lists to collide),
* exact chapter positions (packet count x 1024 samples, not mp3 duration
  guesses),
* mixed inputs (mp3 + m4a, 22 kHz + 44.1 kHz, mono + stereo) just work,
* AAC sources with matching parameters are copied, not re-encoded.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from typing import Callable, Dict, List, Optional, Sequence

import sandbox
from meta import AUDIO_DEMUXERS, cache as meta_cache
from rules import natural_key

AUDIO_EXTS = {"mp3", "m4a", "m4b", "mp4", "aac", "flac", "ogg", "oga", "opus", "wav", "wma",
              "aif", "aiff", "mka", "ape", "alac"}
AAC_RATES = (8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000)
STD_BITRATES = (32, 48, 64, 80, 96, 128, 160, 192, 256)
COVER_NAMES = ("cover", "folder", "front", "albumart", "poster")

# Files from downloads are untrusted. ffmpeg may only read local files, and only with real
# audio demuxers – no playlists (hls), concat scripts, image sequences, network protocols …
# That keeps tricks like a ".mp3" that is really a playlist pointing at /config away from it.
SAFE_INPUT = ["-protocol_whitelist", "file", "-format_whitelist", AUDIO_DEMUXERS]
OWN_METADATA = ["-protocol_whitelist", "file", "-f", "ffmetadata"]
MAX_TEXT_FILE = 512 * 1024


def is_regular(path: str) -> bool:
    """A real file – symlinks in downloads could point at /config or /etc and are ignored."""
    return os.path.isfile(path) and not os.path.islink(path)
IMAGE_EXTS = (".jpg", ".jpeg", ".png")


class Cancelled(Exception):
    pass


class EncodeError(Exception):
    pass


def is_audio(name: str) -> bool:
    return os.path.splitext(name)[1][1:].lower() in AUDIO_EXTS and not name.startswith(".")


def collect_audio(folder: str) -> List[str]:
    """All audio files below ``folder`` (relative paths, natural order, CD1 before CD2)."""
    out = []
    for root, dirs, files in os.walk(folder):     # does not follow symlinked folders
        dirs[:] = [d for d in dirs if not d.startswith((".", "_"))
                   and not os.path.islink(os.path.join(root, d))]
        for f in files:
            if is_audio(f) and is_regular(os.path.join(root, f)):
                out.append(os.path.relpath(os.path.join(root, f), folder))
    return sorted(out, key=lambda p: [natural_key(part) for part in p.split(os.sep)])


def find_cover(folder: str) -> Optional[str]:
    """cover.jpg/folder.jpg/... or, failing that, the only image in the folder."""
    try:
        entries = [e for e in os.listdir(folder) if not e.startswith(".")]
    except OSError:
        return None
    images = [e for e in entries if e.lower().endswith(IMAGE_EXTS)
              and is_regular(os.path.join(folder, e))
              and os.path.getsize(os.path.join(folder, e)) < 20 * 1024 * 1024]
    lower = {e.lower(): e for e in images}
    for base in COVER_NAMES:
        for ext in IMAGE_EXTS:
            if base + ext in lower:
                return os.path.join(folder, lower[base + ext])
    if len(images) == 1:
        return os.path.join(folder, images[0])
    return None


def ffmpeg_encoder() -> str:
    """libfdk_aac if this ffmpeg has it, otherwise the (good) native encoder."""
    try:
        out = subprocess.run(sandbox.ffmpeg(["-hide_banner", "-encoders"]), capture_output=True,
                             text=True, timeout=20).stdout
    except (OSError, subprocess.SubprocessError):
        return "aac"
    return "libfdk_aac" if " libfdk_aac " in out else "aac"


# --------------------------------------------------------------------------- #
# Chapters
# --------------------------------------------------------------------------- #

_TIME_RX = re.compile(r"^\s*(?:(\d+):)?(\d{1,2}):(\d{1,2}(?:[.,]\d+)?)\s+(.*\S)?\s*$")


def _seconds(h: Optional[str], m: str, s: str) -> float:
    return int(h or 0) * 3600 + int(m) * 60 + float(s.replace(",", "."))


def parse_chapters_txt(text: str) -> List[dict]:
    """mp4chaps format (``00:01:02.500 Title``) or OGM (``CHAPTER01=…`` / ``CHAPTER01NAME=…``)."""
    chapters: List[dict] = []
    ogm: Dict[str, dict] = {}
    for line in text.splitlines():
        line = line.strip().lstrip("﻿")
        if not line or line.startswith("#"):
            continue
        m = re.match(r"(?i)^CHAPTER(\d+)(NAME)?\s*=\s*(.*)$", line)
        if m:
            entry = ogm.setdefault(m.group(1), {})
            if m.group(2):
                entry["title"] = m.group(3).strip()
            else:
                t = _TIME_RX.match(m.group(3) + " x")
                if t:
                    entry["start"] = _seconds(t.group(1), t.group(2), t.group(3))
            continue
        t = _TIME_RX.match(line)
        if t:
            chapters.append({"start": _seconds(t.group(1), t.group(2), t.group(3)),
                             "title": (t.group(4) or "").strip()})
    if ogm:
        chapters = [{"start": v["start"], "title": v.get("title", "")}
                    for _, v in sorted(ogm.items(), key=lambda kv: int(kv[0])) if "start" in v]
    chapters.sort(key=lambda c: c["start"])
    for i, c in enumerate(chapters):
        c["title"] = c["title"] or f"Kapitel {i + 1}"
    return chapters


def format_chapters_txt(chapters: Sequence[dict]) -> str:
    lines = []
    for c in chapters:
        s = c["start"]
        lines.append(f"{int(s // 3600):02d}:{int(s % 3600 // 60):02d}:{s % 60:06.3f} {c['title']}")
    return "\n".join(lines) + "\n"


_GENERIC_RX = re.compile(r"(?i)^\s*(?:track|kapitel|chapter|teil|part|cd|disc|disk|titel)?[\s_.-]*\d+\s*$")


def chapter_title(rel_path: str, tags: dict, mode: str) -> str:
    """Name for the chapter of one source file.

    mode ``tag`` prefers the ID3 title, ``filename`` uses the file name;
    ``auto`` uses the tag when there is one, otherwise the cleaned file name.
    """
    stem = os.path.splitext(os.path.basename(rel_path))[0]
    tag = (tags.get("title") or "").strip()
    if mode == "tag" and tag:
        return tag
    if mode == "auto" and tag and not _GENERIC_RX.match(tag):
        return tag
    if mode == "filename":
        return stem
    clean = re.sub(r"^\s*\d{1,4}(?:\s*[-_.)]\s*|\s+)", "", stem).replace("_", " ").strip()
    if not clean or re.fullmatch(r"(?i)track|kapitel|chapter|teil|part|titel|cd|disc", clean):
        return stem.replace("_", " ").strip()   # "01 Kapitel": the number is the information
    return clean


def looks_generic(chapters: Sequence[dict]) -> bool:
    """True if chapter names carry no information ("Track 01", "1", "Kapitel 3")."""
    return len(chapters) <= 1 or all(_GENERIC_RX.match(c.get("title") or "") for c in chapters)


def ffmetadata(chapters: Sequence[dict], total: float, tags: Optional[dict] = None) -> str:
    def esc(s) -> str:
        return re.sub(r"([=;#\\\n])", r"\\\1", str(s))

    lines = [";FFMETADATA1"]
    for k, v in (tags or {}).items():
        if v:
            lines.append(f"{k}={esc(v)}")
    for i, c in enumerate(chapters):
        start = int(round(c["start"] * 1000))
        end = int(round((chapters[i + 1]["start"] if i + 1 < len(chapters) else total) * 1000))
        if end <= start:
            continue
        lines += ["[CHAPTER]", "TIMEBASE=1/1000", f"START={start}", f"END={end}",
                  f"title={esc(c['title'])}"]
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------- #
# Encoding
# --------------------------------------------------------------------------- #

def pick_bitrate(setting: str, infos: List[dict], channels: int) -> str:
    """``auto`` follows the source (never above ``max``), otherwise the fixed value."""
    setting = (setting or "auto").strip().lower()
    if setting != "auto":
        return setting
    rates = [i.get("bit_rate") or 0 for i in infos if i.get("bit_rate")]
    src = sorted(rates)[len(rates) // 2] // 1000 if rates else 128
    cap = 128 if channels > 1 else 64
    target = min(src, cap)
    best = min(STD_BITRATES, key=lambda b: (abs(b - target), b))
    return f"{max(best, 32)}k"


def pick_rate(infos: List[dict]) -> int:
    rates = Counter(i["sample_rate"] for i in infos if i.get("sample_rate"))
    rate = rates.most_common(1)[0][0] if rates else 44100
    return min(AAC_RATES, key=lambda r: (abs(r - min(rate, 48000)), -r))


class Runner:
    """Starts ffmpeg processes and kills them all on cancel."""

    def __init__(self, cancelled: Callable[[], bool], timeout: int = 6 * 3600) -> None:
        self.cancelled = cancelled
        self.timeout = timeout          # a broken/malicious file must not block the queue forever
        self._procs: set = set()
        self._lock = threading.Lock()

    def run(self, cmd: List[str]) -> subprocess.CompletedProcess:
        """``cmd`` comes from sandbox.ffmpeg() – jailed, with its own CPU priority."""
        if self.cancelled():
            raise Cancelled()
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        with self._lock:
            self._procs.add(proc)
        try:
            try:
                out, err = proc.communicate(timeout=self.timeout)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.communicate()
                raise EncodeError(f"ffmpeg hat nach {self.timeout // 3600} h nicht geantwortet – abgebrochen")
        finally:
            with self._lock:
                self._procs.discard(proc)
        if self.cancelled():
            raise Cancelled()
        return subprocess.CompletedProcess(cmd, proc.returncode, out, err)

    def kill(self) -> None:
        with self._lock:
            for p in list(self._procs):
                try:
                    p.kill()
                except OSError:
                    pass


def _adts_seconds(path: str, rate: int) -> float:
    """Exact length of a raw AAC segment: packets x 1024 samples."""
    out = subprocess.run(
        sandbox.ffprobe(["-v", "error", "-protocol_whitelist", "file", "-format_whitelist", "aac", "-count_packets",
                         "-select_streams", "a:0", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0", path],
                        read=[path]),
        capture_output=True, text=True, timeout=600).stdout.strip()
    try:
        return int(out.split(",")[0]) * 1024 / rate
    except ValueError:
        return meta_cache.probe(path).get("duration", 0.0)


def _err_tail(stderr: str) -> str:
    lines = [ln.strip() for ln in (stderr or "").splitlines() if ln.strip()]
    return lines[-1] if lines else "unbekannter Fehler"


def merge_to_m4b(folder: str, files: List[str], out_path: str, *,
                 bitrate: str = "auto", mono: bool = False, jobs: int = 2,
                 chapter_mode: str = "auto", tags: Optional[dict] = None,
                 titles: Optional[List[str]] = None,
                 progress: Callable[[float, str], None] = lambda p, m: None,
                 cancelled: Callable[[], bool] = lambda: False,
                 runner: Optional[Runner] = None, nice: str = "") -> dict:
    """Encode ``files`` (relative to ``folder``) into one chapterised M4B.

    Returns ``{"duration", "chapters", "bitrate", "copied"}``.  ``out_path``
    only appears once everything succeeded.
    """
    if not files:
        raise EncodeError("Keine Audiodateien gefunden")
    runner = runner or Runner(cancelled)
    paths = [os.path.join(folder, f) for f in files]
    infos = [meta_cache.probe(p) for p in paths]
    bad = [f for f, i in zip(files, infos) if not i.get("sample_rate")]
    if bad:
        raise EncodeError(f"Keine Tonspur in: {', '.join(bad[:3])}")

    channels = 1 if mono else min(max((i.get("channels") or 2) for i in infos), 2)
    rate = pick_rate(infos)
    br = pick_bitrate(bitrate, infos, channels)
    # AAC sources that already match: copy instead of transcoding (no quality loss)
    copy = all(i.get("audio_codec") == "aac" and i.get("sample_rate") == rate
               and (i.get("channels") or 2) == channels for i in infos)
    codec = ffmpeg_encoder()
    total_src = sum(i.get("duration") or 0 for i in infos) or 1.0
    done_src = [0.0]
    lock = threading.Lock()

    work = tempfile.mkdtemp(prefix=".m4b-", dir=os.path.dirname(out_path) or None)
    try:
        segs = [os.path.join(work, f"{n:05d}.aac") for n in range(len(paths))]

        def encode(n: int) -> None:
            args = ["-nostdin", "-hide_banner", "-v", "error", "-y",
                    *SAFE_INPUT, "-i", paths[n], "-map", "0:a:0", "-vn", "-sn", "-dn", "-map_metadata", "-1"]
            if copy:
                args += ["-c:a", "copy"]
            else:
                args += ["-c:a", codec, "-b:a", br, "-ar", str(rate), "-ac", str(channels)]
            args += ["-f", "adts", segs[n]]
            res = runner.run(sandbox.ffmpeg(args, read=[paths[n]], write=[work], nice=nice))
            ok = res.returncode == 0 and os.path.exists(segs[n]) and os.path.getsize(segs[n]) > 0
            if not ok:
                raise EncodeError(f"{files[n]}: {_err_tail(res.stderr)}")
            with lock:
                done_src[0] += infos[n].get("duration") or 0
                progress(0.95 * done_src[0] / total_src,
                         f"{'Kopiere' if copy else 'Kodiere'} Datei {n + 1}/{len(paths)}")

        progress(0.0, f"{len(paths)} Datei(en), {'AAC kopieren' if copy else f'AAC {br}'}, "
                      f"{rate} Hz, {'Mono' if channels == 1 else 'Stereo'}")
        with ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
            futures = [pool.submit(encode, n) for n in range(len(paths))]
            try:
                for f in futures:
                    f.result()
            except BaseException:
                runner.kill()
                for f in futures:
                    f.cancel()
                raise

        # Chapters: one per file – or the file's own chapters if it has several
        chapters: List[dict] = []
        pos = 0.0
        for n, rel in enumerate(files):
            length = _adts_seconds(segs[n], rate)
            own = titles[n].strip() if titles and len(titles) == len(files) and titles[n] else ""
            inner = [] if own else file_chapters(paths[n])
            if len(inner) > 1:
                scale = length / (infos[n].get("duration") or length)
                chapters += [{"start": pos + c["start"] * scale, "title": c["title"]} for c in inner]
            else:
                chapters.append({"start": pos, "title": own or chapter_title(rel, infos[n]["tags"], chapter_mode)})
            pos += length
        total = pos

        meta_path = os.path.join(work, "meta.txt")
        with open(meta_path, "w", encoding="utf-8") as fh:
            fh.write(ffmetadata(chapters, total, {"genre": "Audiobook", **(tags or {})}))
        concat = "concat:" + "|".join(segs)
        part = os.path.join(work, "out.m4b")
        progress(0.96, "Erzeuge M4B …")
        res = runner.run(sandbox.ffmpeg(["-nostdin", "-hide_banner", "-v", "error", "-y",
                                         "-protocol_whitelist", "file,concat", "-f", "aac", "-i", concat,
                                         *OWN_METADATA, "-i", meta_path,
                                         "-map", "0:a", "-map_metadata", "1", "-map_chapters", "1",
                                         "-c", "copy", "-movflags", "+faststart", "-f", "mp4", part],
                                        read=[work], write=[work], nice=nice))
        if res.returncode != 0:
            raise EncodeError("Zusammenfügen fehlgeschlagen: " + _err_tail(res.stderr))
        shutil.move(part, out_path)
        progress(1.0, "Fertig")
        return {"duration": total, "chapters": chapters, "bitrate": "copy" if copy else br,
                "copied": copy}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def file_chapters(path: str) -> List[dict]:
    """Chapters embedded in a file (start in seconds, title)."""
    try:
        out = subprocess.run(sandbox.ffprobe(["-v", "error", *SAFE_INPUT, "-print_format", "json", "-show_chapters",
                                              path], read=[path]),
                             capture_output=True, text=True, timeout=60).stdout
        data = json.loads(out or "{}")
    except (OSError, subprocess.SubprocessError, ValueError):
        return []
    res = []
    for i, c in enumerate(data.get("chapters", [])):
        title = ((c.get("tags") or {}).get("title") or "").strip() or f"Kapitel {i + 1}"
        res.append({"start": float(c.get("start_time") or 0), "title": title})
    return res


def remux(src: str, dst: str, chapters: Optional[Sequence[dict]] = None,
          runner: Optional[Runner] = None, nice: str = "") -> None:
    """Copy the audio of ``src`` into a fresh M4B, optionally with new chapters.

    Tags and cover are transferred separately by the tagger (mutagen), so only
    the audio stream and chapters are handled here.
    """
    runner = runner or Runner(lambda: False)
    work = tempfile.mkdtemp(prefix=".remux-", dir=os.path.dirname(dst) or None)
    try:
        cmd = ["-nostdin", "-hide_banner", "-v", "error", "-y", *SAFE_INPUT, "-i", src]
        if chapters is not None:
            total = meta_cache.probe(src).get("duration") or (chapters[-1]["start"] + 1 if chapters else 0)
            meta = os.path.join(work, "meta.txt")
            with open(meta, "w", encoding="utf-8") as fh:
                fh.write(ffmetadata(chapters, total))
            cmd += [*OWN_METADATA, "-i", meta, "-map", "0:a:0", "-map_metadata", "0", "-map_chapters", "1"]
        else:
            cmd += ["-map", "0:a:0", "-map_metadata", "0", "-map_chapters", "0"]
        part = os.path.join(work, "out.m4b")
        cmd += ["-c", "copy", "-movflags", "+faststart", "-f", "mp4", part]
        res = runner.run(sandbox.ffmpeg(cmd, read=[src], write=[work], nice=nice))
        if res.returncode != 0:
            raise EncodeError("Remux fehlgeschlagen: " + _err_tail(res.stderr))
        shutil.move(part, dst)
    finally:
        shutil.rmtree(work, ignore_errors=True)


def extract_cover(path: str, dest_dir: str) -> Optional[str]:
    """Embedded picture of an audio file -> image file, or None."""
    info = meta_cache.probe(path)
    codec = info.get("cover_codec")
    if codec not in ("mjpeg", "png"):
        return None
    out = os.path.join(dest_dir, "embedded-cover." + ("png" if codec == "png" else "jpg"))
    subprocess.run(sandbox.ffmpeg(["-nostdin", "-v", "error", "-y", *SAFE_INPUT, "-i", path, "-map", "0:v:0",
                                   "-c:v", "copy", "-frames:v", "1", out], read=[path], write=[dest_dir]),
                   capture_output=True, timeout=60)
    return out if os.path.exists(out) and os.path.getsize(out) > 0 else None
