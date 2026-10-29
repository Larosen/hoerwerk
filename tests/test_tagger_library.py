import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import library  # noqa: E402

BOOK = {
    "asin": "B0054JTDOG", "title": "Die Gefährten, Band 1", "subtitle": "Der Herr der Ringe 1",
    "authors": ["J.R.R. Tolkien"], "narrators": ["Achim Höppner"], "series": "Der Herr der Ringe",
    "series_position": "1", "publisher": "der Hörverlag", "release_date": "2011-06-03",
    "language": "German", "genres": ["Fantasy", "Klassiker"], "description": "Neun Gefährten …",
    "region": "de", "url": "https://www.audible.de/pd/B0054JTDOG", "source": "audible",
}


class LibraryTest(unittest.TestCase):
    def test_optional_sections(self):
        tpl = "{author}/[{series}/][{series_position:2} - ]{title}"
        self.assertEqual(library.render(tpl, library.fields(BOOK)),
                         "J.R.R. Tolkien/Der Herr der Ringe/01 - Die Gefährten, Band 1")
        solo = {"title": "Der Insasse", "authors": ["Sebastian Fitzek"]}
        self.assertEqual(library.render(tpl, library.fields(solo)), "Sebastian Fitzek/Der Insasse")

    def test_sanitize(self):
        self.assertEqual(library.sanitize('Was: ist? "das"/*'), "Was - ist das-")
        self.assertEqual(library.sanitize("Titel: Untertitel"), "Titel - Untertitel")
        self.assertEqual(library.sanitize("  Ende...  "), "Ende")
        self.assertEqual(library.sanitize(""), "_")
        self.assertLessEqual(len(library.sanitize("ä" * 300).encode()), 180)

    def test_slash_in_value_is_no_folder(self):
        b = {"title": "AC/DC Story", "authors": ["Max"]}
        self.assertEqual(library.render("{author}/{title}", library.fields(b)), "Max/AC-DC Story")

    def test_padding(self):
        self.assertEqual(library._pad("1.5", "2"), "01.5")
        self.assertEqual(library._pad("1-3", "2"), "01-03")

    def test_unique_folder(self):
        root = tempfile.mkdtemp()
        try:
            p = os.path.join(root, "Buch")
            self.assertEqual(library.unique_folder(p), p)
            os.makedirs(p)
            open(os.path.join(p, "cover.jpg"), "w").close()
            self.assertEqual(library.unique_folder(p), p)       # only a cover: reuse
            open(os.path.join(p, "Buch.m4b"), "w").close()
            self.assertEqual(library.unique_folder(p), p + " (2)")
        finally:
            shutil.rmtree(root)

    def test_reuse_existing_spelling(self):
        root = tempfile.mkdtemp()
        try:
            os.makedirs(os.path.join(root, "J. R. R. Tolkien", "der herr der ringe"))
            os.makedirs(os.path.join(root, "Fitzek, Sebastian"))
            os.makedirs(os.path.join(root, "Müller"))
            got = library.reuse_existing(root, os.path.join(root, "J.R.R. Tolkien", "Der Herr der Ringe", "01 - Die Gefährten"))
            self.assertEqual(got, os.path.join(root, "J. R. R. Tolkien", "der herr der ringe", "01 - Die Gefährten"))
            got = library.reuse_existing(root, os.path.join(root, "Sebastian Fitzek", "Der Insasse"))
            self.assertEqual(got, os.path.join(root, "Fitzek, Sebastian", "Der Insasse"))
            got = library.reuse_existing(root, os.path.join(root, "Mueller", "X"))
            self.assertEqual(got, os.path.join(root, "Müller", "X"))
            # a different author is not merged
            got = library.reuse_existing(root, os.path.join(root, "Andreas Eschbach", "X"))
            self.assertEqual(got, os.path.join(root, "Andreas Eschbach", "X"))
        finally:
            shutil.rmtree(root)

    def test_example(self):
        ex = library.example(library.DEFAULT_FOLDER, library.DEFAULT_FILE)
        self.assertEqual(ex, ["J.R.R. Tolkien/Herr der Ringe/01 - Die Gefährten/Die Gefährten.m4b",
                              "Sebastian Fitzek/Der Insasse/Der Insasse.m4b"])


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg fehlt")
class TaggerTest(unittest.TestCase):
    def setUp(self):
        from media import jpeg, tone
        import tagger
        from mutagen.mp4 import MP4
        self.tagger, self.MP4 = tagger, MP4
        self.dir = tempfile.mkdtemp(prefix="tag-")
        self.m4b = tone(os.path.join(self.dir, "b.m4b"), 1.0, tags={"title": "alt", "comment": "weg"})
        self.cover = jpeg(os.path.join(self.dir, "c.jpg"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_write_and_read(self):
        settings = {"keep_series_in_title": False, "narrator_in_artist": True}
        self.tagger.write_tags(self.m4b, BOOK, settings, self.cover)
        t = self.MP4(self.m4b).tags
        self.assertEqual(t["©nam"], ["Die Gefährten"])           # ", Band 1" removed
        self.assertEqual(t["©ART"], ["J.R.R. Tolkien, Achim Höppner"])
        self.assertEqual(t["aART"], ["J.R.R. Tolkien"])
        self.assertEqual(t["©wrt"], ["Achim Höppner"])
        self.assertEqual(t["stik"], [2])
        self.assertEqual(t["©mvn"], ["Der Herr der Ringe"])
        self.assertEqual(t["©mvi"], [1])
        self.assertEqual(t["shwm"], [1])
        self.assertEqual(t["©grp"], ["Der Herr der Ringe, Book #1"])
        self.assertEqual(t["soal"], ["Der Herr der Ringe 1 - Die Gefährten"])
        self.assertEqual(t["©gen"], ["Fantasy/Klassiker"])
        self.assertEqual(bytes(t["----:com.apple.iTunes:ASIN"][0]), b"B0054JTDOG")
        self.assertEqual(bytes(t["covr"][0]), self.cover)
        self.assertNotIn("weg", str(t.get("©cmt")))

        back = self.tagger.read_book(self.m4b)
        self.assertEqual(back["asin"], "B0054JTDOG")
        self.assertEqual(back["authors"], ["J.R.R. Tolkien"])
        self.assertEqual(back["narrators"], ["Achim Höppner"])
        self.assertEqual(back["series_position"], "1")

        # rewriting without a cover keeps the existing one
        self.tagger.write_tags(self.m4b, {**BOOK, "series": None}, {})
        t = self.MP4(self.m4b).tags
        self.assertEqual(bytes(t["covr"][0]), self.cover)
        self.assertNotIn("©mvn", t)

    def test_sidecars(self):
        out = self.tagger.write_sidecars(self.dir, BOOK, {}, self.cover)
        self.assertEqual(out, ["desc.txt", "reader.txt", "cover.jpg"])
        with open(os.path.join(self.dir, "reader.txt"), encoding="utf-8") as fh:
            self.assertEqual(fh.read(), "Achim Höppner\n")
        self.assertEqual(self.tagger.write_sidecars(self.dir, BOOK, {"write_desc": False, "write_reader": False,
                                                                      "write_cover": False}, self.cover), [])

    def test_manual_yaml(self):
        with open(os.path.join(self.dir, "metadata.yml"), "w", encoding="utf-8") as fh:
            fh.write("title: Der Hobbit (Hörspiel)\nauthors: [J. R. R. Tolkien]\nnarrators:\n  - Rufus Beck\n"
                     "description: |\n  Bilbo zieht los.\ngenres: [Fantasy]\nreleaseDate: 2008-08-19\n"
                     "publisher: Hörverlag\nseries: Mittelerde\nseriesPosition: 0\n")
        b = self.tagger.load_manual(self.dir)
        self.assertEqual(b["title"], "Der Hobbit (Hörspiel)")
        self.assertEqual(b["narrators"], ["Rufus Beck"])
        self.assertEqual(b["release_date"], "2008-08-19")
        self.assertEqual(b["series_position"], "0")
        self.assertEqual(b["description"], "Bilbo zieht los.")


if __name__ == "__main__":
    unittest.main()
