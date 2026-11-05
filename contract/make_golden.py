"""Builds contract/golden/tags.json with the Python tag writer (run once after a deliberate change).

Input: contract/golden/book.json + a short two-chapter M4B made with ffmpeg + a tiny JPEG cover."""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))
sys.path.insert(0, HERE)

import config  # noqa: E402
import tagger  # noqa: E402
from read_tags import describe  # noqa: E402


def sample_m4b(path: str) -> None:
    meta = os.path.join(os.path.dirname(path), "chapters.txt")
    with open(meta, "w") as fh:
        fh.write(";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=Erster Teil\n"
                 "[CHAPTER]\nTIMEBASE=1/1000\nSTART=1000\nEND=2000\ntitle=Zweiter Teil\n")
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=2", "-i", meta,
                    "-map_metadata", "1", "-map_chapters", "1", "-c:a", "aac", "-b:a", "64k", "-f", "mp4", path], check=True)


def cover_jpeg() -> bytes:
    return open(os.path.join(HERE, "golden", "cover.jpg"), "rb").read()


def golden(writer) -> dict:
    spec = json.load(open(os.path.join(HERE, "golden", "book.json"), encoding="utf-8"))
    tmp = tempfile.mkdtemp()
    path = os.path.join(tmp, "book.m4b")
    sample_m4b(path)
    writer(path, spec["book"], {**config.default_settings({}), **spec["settings"]}, cover_jpeg())
    return describe(path)


if __name__ == "__main__":
    out = golden(tagger.write_tags)
    with open(os.path.join(HERE, "golden", "tags.json"), "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
        fh.write("\n")
    print("written")
