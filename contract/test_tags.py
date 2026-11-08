"""The tag writer (hw-tags) produces contract/golden/tags.json for contract/golden/book.json, and reads the book
back as contract/golden/read.json – both recorded from Hörwerk 7.13 and kept as the yardstick.

hw-tags is found via HW_TAGS or rust/target/release/hw-tags – skipped when it is not built."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

from read_tags import describe

HERE = os.path.dirname(os.path.abspath(__file__))
BINARY = os.environ.get("HW_TAGS") or os.path.join(HERE, "..", "rust", "target", "release", "hw-tags")


def _load(path: str):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def sample_m4b(path: str) -> None:
    """A short two-chapter M4B made with ffmpeg."""
    meta = os.path.join(os.path.dirname(path), "chapters.txt")
    with open(meta, "w") as fh:
        fh.write(";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=Erster Teil\n"
                 "[CHAPTER]\nTIMEBASE=1/1000\nSTART=1000\nEND=2000\ntitle=Zweiter Teil\n")
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=2", "-i", meta,
                    "-map_metadata", "1", "-map_chapters", "1", "-c:a", "aac", "-b:a", "64k", "-f", "mp4", path], check=True)


def cover_jpeg() -> bytes:
    with open(os.path.join(HERE, "golden", "cover.jpg"), "rb") as fh:
        return fh.read()


def golden(writer) -> dict:
    """The tags a writer leaves in the sample M4B (default settings + those of book.json)."""
    spec = _load(os.path.join(HERE, "golden", "book.json"))
    defaults = _load(os.path.join(HERE, "settings.json"))["defaults"]
    tmp = tempfile.mkdtemp()
    try:
        path = os.path.join(tmp, "book.m4b")
        sample_m4b(path)
        writer(path, spec["book"], {**defaults, **spec["settings"]}, cover_jpeg())
        return describe(path)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def rust_writer(path, book, settings, cover):
    tmp = tempfile.mkdtemp()
    try:
        files = {}
        for name, data in (("book.json", json.dumps(book)), ("settings.json", json.dumps(settings))):
            files[name] = os.path.join(tmp, name)
            with open(files[name], "w", encoding="utf-8") as fh:
                fh.write(data)
        cover_path = os.path.join(tmp, "cover.jpg")
        with open(cover_path, "wb") as fh:
            fh.write(cover)
        subprocess.run([BINARY, "write", path, "--book", files["book.json"], "--settings", files["settings.json"],
                        "--cover", cover_path], check=True, capture_output=True, text=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


@unittest.skipUnless(os.path.exists(BINARY) and shutil.which("ffmpeg"), "hw-tags not built or ffmpeg missing")
class RustTagWriter(unittest.TestCase):
    def test_matches_golden(self):
        want = _load(os.path.join(HERE, "golden", "tags.json"))
        self.assertEqual(golden(rust_writer), want)

    def test_retagging_replaces_and_keeps_the_cover(self):
        """A second tagging without a cover keeps the old one and leaves nothing of the first tags behind."""
        spec = _load(os.path.join(HERE, "golden", "book.json"))
        tmp = tempfile.mkdtemp()
        path = os.path.join(tmp, "b.m4b")
        sample_m4b(path)
        rust_writer(path, spec["book"], spec["settings"], cover_jpeg())
        book = {"title": "Anderes Buch", "authors": ["Max Muster"]}
        files = os.path.join(tmp, "book2.json")
        with open(files, "w") as fh:
            json.dump(book, fh)
        subprocess.run([BINARY, "write", path, "--book", files], check=True)
        tags = describe(path)
        self.assertEqual(tags["©nam"], ["Anderes Buch"])
        self.assertNotIn("----:com.apple.iTunes:ASIN", tags)              # nothing of the first book survives
        self.assertEqual(len(tags["covr"]), 1)                            # the cover stays
        self.assertEqual(len(tags["chapters"]), 2)                        # chapters untouched
        shutil.rmtree(tmp)

    def test_reads_like_7_13(self):
        """hw-tags read gives the book the library index uses – as Hörwerk 7.13 read it (golden/read.json)."""
        spec = _load(os.path.join(HERE, "golden", "book.json"))
        tmp = tempfile.mkdtemp()
        try:
            path = os.path.join(tmp, "b.m4b")
            sample_m4b(path)
            rust_writer(path, spec["book"], spec["settings"], cover_jpeg())
            out = subprocess.run([BINARY, "read", path], check=True, capture_output=True, text=True).stdout
            self.assertEqual(json.loads(out), _load(os.path.join(HERE, "golden", "read.json")))
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
