"""Small helpers to create real audio files for the tests (needs ffmpeg)."""
import os
import subprocess


def tone(path, seconds=2.0, rate=44100, channels=2, freq=440, codec_args=None, tags=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    cmd = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi",
           "-i", f"sine=f={freq}:d={seconds}", "-ar", str(rate), "-ac", str(channels)]
    for k, v in (tags or {}).items():
        cmd += ["-metadata", f"{k}={v}"]
    if path.endswith((".m4a", ".m4b")):
        cmd += ["-c:a", "aac", "-b:a", "64k", "-f", "mp4"]
    cmd += list(codec_args or []) + [path]
    subprocess.run(cmd, check=True)
    return path


def jpeg(path):
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i",
                    "color=c=red:s=64x64", "-frames:v", "1", path], check=True)
    with open(path, "rb") as fh:
        return fh.read()
