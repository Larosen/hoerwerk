"""Audio file details via ffprobe (duration, tags, stream info), cached by modification time."""
from __future__ import annotations

import json
import os
import subprocess
import threading
from typing import Dict

import sandbox

# Containers ffprobe/ffmpeg may open: audio only – playlists, image sequences, video and
# network formats stay closed (downloads are foreign files).
AUDIO_DEMUXERS = "mp3,mov,aac,flac,ogg,wav,w64,aiff,asf,matroska,ape,wv,caf,mpc,mpc8,tta,au,amr,dsf"


def probe(path: str) -> dict:
    """Return {duration, tags, bit_rate, sample_rate, channels, audio_codec, cover_codec}."""
    try:
        out = subprocess.run(
            sandbox.ffprobe(["-v", "error", "-protocol_whitelist", "file",
                             "-format_whitelist", AUDIO_DEMUXERS, "-print_format", "json",
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
            "sample_rate": None, "channels": None, "cover_codec": None, "audio_codec": None}
    for s in data.get("streams", []):
        if s.get("codec_type") == "audio" and info["sample_rate"] is None:
            info["sample_rate"] = int(s.get("sample_rate") or 0) or None
            info["channels"] = s.get("channels")
            info["audio_codec"] = s.get("codec_name")
            for k, v in (s.get("tags") or {}).items():
                tags.setdefault(k.lower(), v)
        elif (s.get("codec_type") == "video" and info["cover_codec"] is None
              and ((s.get("disposition") or {}).get("attached_pic") or s.get("codec_name") in ("mjpeg", "png"))):
            info["cover_codec"] = s.get("codec_name")      # the embedded cover picture
    return info


class MetaCache:
    def __init__(self) -> None:
        self._cache: Dict[str, tuple] = {}
        self._lock = threading.Lock()

    def probe(self, path: str) -> dict:
        try:
            mtime = os.path.getmtime(path)
        except OSError:
            return {}
        with self._lock:
            hit = self._cache.get(path)
        if hit and hit[0] == mtime:
            return hit[1]
        value = probe(path)
        with self._lock:
            if len(self._cache) > 20000:
                self._cache.clear()
            self._cache[path] = (mtime, value)
        return value


cache = MetaCache()
