"""Metadata for rename templates: audio/video tags via ffprobe, photo EXIF in pure Python."""
from __future__ import annotations

import json
import os
import re
import struct
import subprocess
import threading
from datetime import datetime
from typing import Dict, Optional

import sandbox

AUDIO_EXT = {"mp3", "m4a", "m4b", "aac", "flac", "ogg", "oga", "opus", "wav", "aif", "aiff",
             "wma", "alac", "ape", "mka", "dsf"}
VIDEO_EXT = {"mp4", "mkv", "mov", "avi", "m4v", "wmv", "webm", "ts", "mts", "m2ts", "mpg",
             "mpeg", "flv", "3gp", "vob", "ogv"}
IMAGE_EXT = {"jpg", "jpeg", "heic", "heif", "tif", "tiff", "png", "webp", "cr2", "cr3", "nef",
             "arw", "dng", "orf", "rw2", "raf", "srw", "pef"}


# Containers ffprobe/ffmpeg may open. Downloads (audiobook pipeline) only get the audio ones;
# the renamer also reads video containers for {resolution}, {codec}, {fps}.
AUDIO_DEMUXERS = "mp3,mov,aac,flac,ogg,wav,w64,aiff,asf,matroska,ape,wv,caf,mpc,mpc8,tta,au,amr,dsf"
VIDEO_DEMUXERS = "avi,mpegts,mpeg,flv"


def kind_of(filename: str) -> str:
    ext = os.path.splitext(filename)[1][1:].lower()
    if ext in AUDIO_EXT:
        return "audio"
    if ext in VIDEO_EXT:
        return "video"
    if ext in IMAGE_EXT:
        return "image"
    return "other"


# --------------------------------------------------------------------------- #
# ffprobe
# --------------------------------------------------------------------------- #

def probe(path: str) -> dict:
    """Return {duration, tags, sample_rate, channels, cover_codec, video: {...}}."""
    try:
        out = subprocess.run(
            sandbox.ffprobe(["-v", "error", "-protocol_whitelist", "file",
                             "-format_whitelist", f"{AUDIO_DEMUXERS},{VIDEO_DEMUXERS}", "-print_format", "json",
                             "-show_format", "-show_streams", path], read=[path]),
            capture_output=True, text=True, timeout=60,
        ).stdout
        data = json.loads(out or "{}")
    except (subprocess.SubprocessError, json.JSONDecodeError, OSError):
        data = {}
    fmt = data.get("format", {})
    tags = {k.lower(): v for k, v in (fmt.get("tags") or {}).items()}
    info = {"duration": float(fmt.get("duration") or 0), "tags": tags,
            "bit_rate": int(fmt.get("bit_rate") or 0),
            "sample_rate": None, "channels": None, "cover_codec": None,
            "audio_codec": None, "video": None}
    for s in data.get("streams", []):
        disp = s.get("disposition") or {}
        if s.get("codec_type") == "audio" and info["sample_rate"] is None:
            info["sample_rate"] = int(s.get("sample_rate") or 0) or None
            info["channels"] = s.get("channels")
            info["audio_codec"] = s.get("codec_name")
            for k, v in (s.get("tags") or {}).items():
                tags.setdefault(k.lower(), v)
        elif s.get("codec_type") == "video":
            is_cover = disp.get("attached_pic") or (
                kind_of(path) == "audio" and s.get("codec_name") in ("mjpeg", "png"))
            if is_cover:
                info["cover_codec"] = info["cover_codec"] or s.get("codec_name")
            elif info["video"] is None:
                fps = 0.0
                try:
                    a, b = (s.get("avg_frame_rate") or "0/1").split("/")
                    fps = float(a) / float(b) if float(b) else 0.0
                except ValueError:
                    pass
                info["video"] = {"width": s.get("width"), "height": s.get("height"),
                                 "codec": s.get("codec_name"), "fps": fps}
                for k, v in (s.get("tags") or {}).items():
                    tags.setdefault(k.lower(), v)
    return info


# --------------------------------------------------------------------------- #
# EXIF (JPEG, HEIC, TIFF-based RAW)
# --------------------------------------------------------------------------- #

_EXIF_TAGS = {
    0x010F: "make", 0x0110: "model", 0x0132: "datetime", 0x8769: "_exif_ifd",
    0x9003: "datetime_original", 0x9004: "datetime_digitized", 0xA002: "width",
    0xA003: "height", 0x8827: "iso", 0xA434: "lens", 0x829A: "exposure",
    0x829D: "fnumber", 0x920A: "focal", 0x0112: "orientation",
}
_TYPE_SIZE = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 9: 4, 10: 8}


def _parse_tiff(buf: bytes) -> Dict[str, object]:
    if len(buf) < 8:
        return {}
    bo = {b"II": "<", b"MM": ">"}.get(buf[:2])
    if not bo:
        return {}
    out: Dict[str, object] = {}

    def read_ifd(off: int, depth: int = 0) -> None:
        if depth > 2 or off <= 0 or off + 2 > len(buf):
            return
        (count,) = struct.unpack_from(bo + "H", buf, off)
        for i in range(min(count, 500)):
            e = off + 2 + i * 12
            if e + 12 > len(buf):
                return
            tag, typ, n = struct.unpack_from(bo + "HHI", buf, e)
            name = _EXIF_TAGS.get(tag)
            if not name or typ not in _TYPE_SIZE:
                continue
            size = _TYPE_SIZE[typ] * n
            voff = e + 8 if size <= 4 else struct.unpack_from(bo + "I", buf, e + 8)[0]
            if voff + size > len(buf):
                continue
            if typ == 2:
                val: object = buf[voff:voff + n].split(b"\x00", 1)[0].decode("utf-8", "replace").strip()
            elif typ == 3:
                val = struct.unpack_from(bo + "H", buf, voff)[0]
            elif typ in (4, 9):
                val = struct.unpack_from(bo + ("I" if typ == 4 else "i"), buf, voff)[0]
            elif typ in (5, 10):
                a, b = struct.unpack_from(bo + ("II" if typ == 5 else "ii"), buf, voff)
                val = (a, b)
            else:
                continue
            if name == "_exif_ifd":
                read_ifd(int(val), depth + 1)  # type: ignore[arg-type]
            else:
                out.setdefault(name, val)

    (first,) = struct.unpack_from(bo + "I", buf, 4)
    read_ifd(first)
    return out


def read_exif(path: str) -> Dict[str, object]:
    try:
        with open(path, "rb") as fh:
            head = fh.read(1024 * 1024)
    except OSError:
        return {}
    if head[:4] in (b"II*\x00", b"MM\x00*"):
        raw = _parse_tiff(head)
    else:
        i = head.find(b"Exif\x00\x00")
        if i < 0:
            return {}
        raw = _parse_tiff(head[i + 6:])
    out: Dict[str, object] = {}
    for k in ("make", "model", "lens"):
        if raw.get(k):
            out[k] = str(raw[k]).strip()
    if out.get("model") and out.get("make"):
        # "Canon EOS R6" instead of "Canon Canon EOS R6"
        if str(out["model"]).lower().startswith(str(out["make"]).split()[0].lower()):
            out["camera"] = out["model"]
        else:
            out["camera"] = f"{out['make']} {out['model']}"
    for k in ("width", "height", "iso", "orientation"):
        if isinstance(raw.get(k), int):
            out[k] = raw[k]
    if isinstance(raw.get("fnumber"), tuple) and raw["fnumber"][1]:
        out["fnumber"] = f"f{raw['fnumber'][0] / raw['fnumber'][1]:.1f}".replace(".0", "")
    if isinstance(raw.get("focal"), tuple) and raw["focal"][1]:
        out["focal"] = f"{raw['focal'][0] / raw['focal'][1]:.0f}mm"
    if isinstance(raw.get("exposure"), tuple) and raw["exposure"][1]:
        a, b = raw["exposure"]
        out["exposure"] = f"1-{round(b / a)}s" if a and a < b else f"{a / b:g}s"
    for k in ("datetime_original", "datetime_digitized", "datetime"):
        d = _parse_date(str(raw.get(k) or ""))
        if d:
            out["_taken"] = d
            break
    return out


# --------------------------------------------------------------------------- #
# Combined metadata
# --------------------------------------------------------------------------- #

def _parse_date(s: str) -> Optional[datetime]:
    s = s.strip()
    if not s or s.startswith("0000"):
        return None
    m = re.match(r"(\d{4})[:\-](\d{2})[:\-](\d{2})(?:[T\s](\d{2}):(\d{2})(?::(\d{2}))?)?", s)
    if m:
        try:
            return datetime(*(int(x) if x else 0 for x in m.groups()))
        except ValueError:
            return None
    m = re.fullmatch(r"(\d{4})", s)
    if m:
        return datetime(int(s), 1, 1)
    return None


def _duration(sec: float) -> str:
    sec = int(round(sec))
    h, rem = divmod(sec, 3600)
    m, s = divmod(rem, 60)
    return f"{h}.{m:02d}.{s:02d}" if h else f"{m:02d}.{s:02d}"


def _resolution(w: Optional[int], h: Optional[int]) -> str:
    if not w or not h:
        return ""
    short = min(w, h) if w >= h else w  # portrait videos: use width as reference
    long_side = max(w, h)
    if long_side >= 3800 or short >= 2100:
        return "2160p"
    if long_side >= 2500 or short >= 1400:
        return "1440p"
    if long_side >= 1900 or short >= 1000:
        return "1080p"
    if long_side >= 1200 or short >= 700:
        return "720p"
    return f"{min(w, h)}p"


def load_meta(path: str) -> dict:
    """Flat dict of template keys for one file (empty for unsupported types)."""
    kind = kind_of(path)
    meta: dict = {}
    if kind in ("audio", "video"):
        info = probe(path)
        tags = info["tags"]
        for key in ("title", "artist", "album", "album_artist", "track", "disc", "genre",
                    "composer", "comment", "date", "year", "show", "season_number",
                    "episode_id", "episode_sort", "description", "language", "publisher"):
            if tags.get(key):
                meta[key] = str(tags[key]).strip()
        for alias, src in (("albumartist", "album_artist"), ("tracknumber", "track"),
                           ("discnumber", "disc")):
            if alias in tags and src not in meta:
                meta[src] = str(tags[alias]).strip()
        # TV episodes: {show} {season} {episode}
        season = tags.get("season_number") or tags.get("season")
        episode = tags.get("episode_sort") or tags.get("episode")
        if season:
            meta["season"] = str(season).strip()
        if episode:
            meta["episode"] = str(episode).strip()
        if info["duration"]:
            meta["duration"] = _duration(info["duration"])
            meta["seconds"] = int(round(info["duration"]))
        if info["bit_rate"]:
            meta["bitrate"] = f"{round(info['bit_rate'] / 1000)}kbps"
        if info["audio_codec"]:
            meta["audio_codec"] = info["audio_codec"]
        if info["sample_rate"]:
            meta["samplerate"] = f"{info['sample_rate'] / 1000:g}kHz"
        v = info.get("video")
        if v and kind == "video":
            meta["width"], meta["height"] = v.get("width") or "", v.get("height") or ""
            meta["codec"] = v.get("codec") or ""
            meta["resolution"] = _resolution(v.get("width"), v.get("height"))
            if v.get("fps"):
                meta["fps"] = f"{v['fps']:.3g}"
        d = _parse_date(str(tags.get("creation_time") or tags.get("date") or tags.get("year") or ""))
        if d:
            meta["_taken"] = d
    elif kind == "image":
        meta.update(read_exif(path))
        if "width" not in meta and not path.lower().endswith((".cr2", ".cr3", ".nef", ".arw", ".dng", ".orf", ".rw2", ".raf")):
            info = probe(path)
            v = info.get("video") or {}
            if v.get("width"):
                meta["width"], meta["height"] = v["width"], v["height"]
    if meta.get("width") and meta.get("height"):
        meta["dimensions"] = f"{meta['width']}x{meta['height']}"
    return meta


class MetaCache:
    def __init__(self) -> None:
        self._cache: Dict[str, tuple] = {}
        self._lock = threading.Lock()

    def _get(self, path: str, loader) -> dict:
        try:
            mtime = os.path.getmtime(path)
        except OSError:
            return {}
        key = f"{loader.__name__}:{path}"
        with self._lock:
            hit = self._cache.get(key)
        if hit and hit[0] == mtime:
            return hit[1]
        value = loader(path)
        with self._lock:
            if len(self._cache) > 20000:
                self._cache.clear()
            self._cache[key] = (mtime, value)
        return value

    def meta(self, path: str) -> dict:
        return self._get(path, load_meta)

    def probe(self, path: str) -> dict:
        return self._get(path, probe)


cache = MetaCache()
