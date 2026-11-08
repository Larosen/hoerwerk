"""Every tag writer produces contract/golden/tags.json for contract/golden/book.json.

The Python writer is held to it by tests/test_golden.py; this file checks the Rust writer (hw-tags), found via
HW_TAGS or rust/target/release/hw-tags – skipped when it is not built."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import make_golden
from make_golden import _load

HERE = os.path.dirname(os.path.abspath(__file__))
BINARY = os.environ.get("HW_TAGS") or os.path.join(HERE, "..", "rust", "target", "release", "hw-tags")


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
        self.assertEqual(make_golden.golden(rust_writer), want)

    def test_retagging_replaces_and_keeps_the_cover(self):
        """A second tagging without a cover keeps the old one and leaves nothing of the first tags behind."""
        spec = _load(os.path.join(HERE, "golden", "book.json"))
        tmp = tempfile.mkdtemp()
        path = os.path.join(tmp, "b.m4b")
        make_golden.sample_m4b(path)
        rust_writer(path, spec["book"], spec["settings"], make_golden.cover_jpeg())
        book = {"title": "Anderes Buch", "authors": ["Max Muster"]}
        files = os.path.join(tmp, "book2.json")
        with open(files, "w") as fh:
            json.dump(book, fh)
        subprocess.run([BINARY, "write", path, "--book", files], check=True)
        from read_tags import describe
        tags = describe(path)
        self.assertEqual(tags["©nam"], ["Anderes Buch"])
        self.assertNotIn("----:com.apple.iTunes:ASIN", tags)              # nothing of the first book survives
        self.assertEqual(len(tags["covr"]), 1)                            # the cover stays
        self.assertEqual(len(tags["chapters"]), 2)                        # chapters untouched
        shutil.rmtree(tmp)

    def test_reads_what_python_reads(self):
        """hw-tags read gives the same book as tagger.read_book (what the library index uses)."""
        import sys
        sys.path.insert(0, os.path.join(HERE, "..", "app"))
        import tagger
        spec = _load(os.path.join(HERE, "golden", "book.json"))
        tmp = tempfile.mkdtemp()
        try:
            path = os.path.join(tmp, "b.m4b")
            make_golden.sample_m4b(path)
            tagger.write_tags(path, spec["book"], spec["settings"], make_golden.cover_jpeg())
            out = subprocess.run([BINARY, "read", path], check=True, capture_output=True, text=True).stdout
            self.assertEqual(json.loads(out), tagger.read_book(path))
        finally:
            shutil.rmtree(tmp)


if __name__ == "__main__":
    unittest.main()
