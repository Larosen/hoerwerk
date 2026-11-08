"""Describe the tags of an M4B as plain JSON – the neutral yardstick for every tag writer (Python or Rust).

    python3 contract/read_tags.py book.m4b

Text atoms as lists of strings, numbers as lists, freeform atoms (----:com.apple.iTunes:ASIN …) decoded as UTF-8,
the cover as format + SHA-256, chapters as (start, title) from ffprobe. Atom order and padding do not matter."""
import hashlib
import json
import subprocess
import sys

from mutagen.mp4 import MP4, MP4Cover


def describe(path: str) -> dict:
    tags = MP4(path).tags or {}
    out: dict = {}
    for key in sorted(tags.keys()):
        values = tags[key]
        if key == "covr":
            out[key] = [{"format": "png" if c.imageformat == MP4Cover.FORMAT_PNG else "jpeg",
                         "sha256": hashlib.sha256(bytes(c)).hexdigest()} for c in values]
        elif key.startswith("----"):
            out[key] = [bytes(v).decode("utf-8") for v in values]
        elif key in ("trkn", "disk"):
            out[key] = [list(v) for v in values]
        else:
            out[key] = [v if isinstance(v, (int, float, str, bool)) else str(v) for v in values]
    probe = json.loads(subprocess.run(["ffprobe", "-v", "error", "-print_format", "json", "-show_chapters", path],
                                      capture_output=True, text=True, check=True).stdout)
    out["chapters"] = [[round(float(c["start_time"]), 2), c.get("tags", {}).get("title", "")] for c in probe["chapters"]]
    return out


if __name__ == "__main__":
    print(json.dumps(describe(sys.argv[1]), ensure_ascii=False, indent=1, sort_keys=True))
