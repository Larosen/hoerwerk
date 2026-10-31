"""Aufräumen: only desc.txt/reader.txt, only next to metadata.json, never through symlinks."""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import cleanup  # noqa: E402


def touch(path, text="x"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


class CleanupTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        r = lambda *p: os.path.join(self.root, *p)  # noqa: E731
        for name in ("metadata.json", "desc.txt", "reader.txt", "cover.jpg", "Buch.m4b"):
            touch(r("Autorin", "Buch", name))
        touch(r("Alt", "MP3-Buch", "desc.txt"))                     # no metadata.json: stays
        touch(r("Alt", "MP3-Buch", "01.mp3"))
        touch(r(".br-alt-1-Buch", "metadata.json"))                 # hidden parked copy: untouched
        touch(r(".br-alt-1-Buch", "desc.txt"))
        outside = tempfile.mkdtemp()
        touch(os.path.join(outside, "metadata.json"))
        touch(os.path.join(outside, "desc.txt"))
        os.symlink(outside, r("Link"))                              # symlinked folder: not followed
        os.makedirs(r("Sym"))
        touch(r("Sym", "metadata.json"))
        os.symlink(os.path.join(outside, "desc.txt"), r("Sym", "desc.txt"))   # symlinked file: not deleted
        self.outside = outside

    def test_finds_only_redundant_files(self):
        self.assertEqual(sorted(cleanup.leftovers(self.root)),
                         [os.path.join("Autorin", "Buch", "desc.txt"), os.path.join("Autorin", "Buch", "reader.txt")])

    def test_remove(self):
        self.assertEqual(cleanup.remove(self.root), {"removed": 2, "errors": []})
        self.assertEqual(sorted(os.listdir(os.path.join(self.root, "Autorin", "Buch"))), ["Buch.m4b", "cover.jpg", "metadata.json"])
        self.assertTrue(os.path.exists(os.path.join(self.root, "Alt", "MP3-Buch", "desc.txt")))
        self.assertTrue(os.path.exists(os.path.join(self.outside, "desc.txt")))
        self.assertEqual(cleanup.leftovers(self.root), [])

    def test_missing_library(self):
        self.assertEqual(cleanup.leftovers(""), [])
        self.assertEqual(cleanup.leftovers(os.path.join(self.root, "nope")), [])


if __name__ == "__main__":
    unittest.main()
