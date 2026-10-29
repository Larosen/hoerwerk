import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import db as dbm  # noqa: E402
import library_index  # noqa: E402
import pipeline  # noqa: E402
import tagger  # noqa: E402
from media import tone  # noqa: E402
from meta import cache as meta_cache  # noqa: E402
from test_pipeline import FakeClient, product, tree  # noqa: E402

HAVE_FFMPEG = bool(shutil.which("ffmpeg"))


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg fehlt")
class IndexTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="lib-")
        self.dbfile = os.path.join(self.root, "t.db")
        self.db = dbm.Database(self.dbfile)
        self.lib = os.path.join(self.root, "lib")
        # filed by Better Renamer: M4B with tags
        m4b = tone(os.path.join(self.lib, "Sebastian Fitzek", "Der Insasse", "Der Insasse.m4b"), 1.0)
        tagger.write_tags(m4b, {"title": "Der Insasse", "authors": ["Sebastian Fitzek"], "asin": "B07HFT61QM"}, {})
        # older collection: MP3 folder with tags …
        tone(os.path.join(self.lib, "Tolkien", "Herr der Ringe", "01 - Die Gefährten", "01.mp3"), 0.5,
             tags={"album": "Die Gefährten", "artist": "J. R. R. Tolkien"})
        # … and one without any tags: folder names only
        tone(os.path.join(self.lib, "Eschbach, Andreas", "Das Jesus-Video", "a.mp3"), 0.5)
        self.idx = library_index.LibraryIndex(self.lib, self.db,
                                              read_m4b=tagger.read_book, probe=meta_cache.probe)
        self.assertEqual(self.idx.scan(), 3)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_find(self):
        self.assertEqual(self.idx.find({"asin": "b07hft61qm", "title": "x"})[0]["why"], "asin")
        self.assertEqual(self.idx.find({"title": "Der Insasse", "authors": ["Fitzek, Sebastian"]})[0]["path"],
                         "Sebastian Fitzek/Der Insasse")
        self.assertEqual(self.idx.find({"title": "Die Gefaehrten", "authors": ["J.R.R. Tolkien"]})[0]["path"],
                         "Tolkien/Herr der Ringe/01 - Die Gefährten")
        self.assertEqual(self.idx.find({"title": "Das Jesus Video", "authors": ["Andreas Eschbach"]})[0]["why"], "title")
        self.assertEqual(self.idx.find({"title": "Der Passagier 23", "authors": ["Sebastian Fitzek"]}), [])
        self.assertEqual(self.idx.find({"title": "Der Insasse", "authors": ["Jemand Anderes"]}), [])

    def test_release_names(self):
        hit = self.idx.match_release("Sebastian_Fitzek-Der_Insasse-Ungekuerzt-DE-2018-AUDIOBOOK-GRP")
        self.assertEqual([h["path"] for h in hit], ["Sebastian Fitzek/Der Insasse"])
        self.assertEqual(self.idx.match_release("J.R.R.Tolkien-Die.Gefaehrten.Hoerbuch"), self.idx.match_release(
            "Tolkien - Die Gefährten (Lesung)"))
        self.assertEqual(len(self.idx.match_release("Tolkien - Die Gefährten")), 1)
        self.assertEqual(self.idx.match_release("Sebastian Fitzek - Passagier 23"), [])

    def test_cache_and_incremental(self):
        again = library_index.LibraryIndex(self.lib, dbm.Database(self.dbfile))
        self.assertEqual(again.stats()["books"], 3)           # loaded from the cache without scanning
        self.assertEqual(again.stats()["m4b"], 1)
        f = os.path.join(self.lib, "Neu", "Buch", "Buch.m4b")
        tone(f, 0.5)
        self.idx.add(f, {"title": "Buch", "authors": ["Neu"]})
        self.assertEqual(len(self.idx.find({"title": "Buch", "authors": ["Neu"]})), 1)
        self.idx.remove(f)
        self.assertEqual(self.idx.find({"title": "Buch", "authors": ["Neu"]}), [])


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg fehlt")
class DuplicatePipelineTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="dup-")
        self.dbfile = os.path.join(self.root, "t.db")
        self.db = dbm.Database(self.dbfile)
        self.inbox = os.path.join(self.root, "inbox")
        self.lib = os.path.join(self.root, "lib")
        os.makedirs(self.inbox)
        old = tone(os.path.join(self.lib, "Sebastian Fitzek", "Der Insasse", "Der Insasse.m4b"), 1.0)
        tagger.write_tags(old, {"title": "Der Insasse", "authors": ["Sebastian Fitzek"], "asin": "B0INSASSE1"}, {})
        self.settings = {"ab_region": "de", "ab_match": "auto", "ab_threshold": 0.8, "ab_chapters": "files",
                         "ab_originals": "archive", "ab_duplicates": "ask"}
        self.idx = library_index.LibraryIndex(self.lib, self.db,
                                              read_m4b=tagger.read_book, probe=meta_cache.probe)
        self.idx.scan()
        self.p = pipeline.Pipeline(lambda: self.settings, self.db, self.inbox,
                                   self.lib, os.path.join(self.inbox, ".work"), start=False, index=self.idx,
                                   client=FakeClient([product("B0INSASSE1", "Der Insasse", ["Sebastian Fitzek"], 4.0)]))

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def new_download(self):
        tone(os.path.join(self.inbox, "Sebastian Fitzek - Der Insasse", "01.mp3"), 4.0)
        jobs = self.p.scan_once(settle=0)
        self.p.process(jobs[0])
        return jobs[0]

    def test_asks_instead_of_adding_a_copy(self):
        j = self.new_download()
        self.assertEqual(j.status, "review", j.log)
        self.assertEqual(j.duplicate["path"], "Sebastian Fitzek/Der Insasse")
        self.assertEqual(j.duplicate["why"], "asin")
        self.assertEqual(tree(self.lib), ["Sebastian Fitzek/Der Insasse/Der Insasse.m4b"])

    def test_keep_both(self):
        j = self.new_download()
        self.p.resolve_duplicate(j.id, "keep")
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)
        self.assertIn("Sebastian Fitzek/Der Insasse (2)/Der Insasse.m4b", tree(self.lib))

    def test_replace_moves_old_out_of_the_library(self):
        j = self.new_download()
        self.p.resolve_duplicate(j.id, "replace")
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)
        self.assertIn("Sebastian Fitzek/Der Insasse/Der Insasse.m4b", tree(self.lib))
        self.assertNotIn("Sebastian Fitzek/Der Insasse (2)/Der Insasse.m4b", tree(self.lib))
        self.assertIn("_ersetzt/Sebastian Fitzek/Der Insasse/Der Insasse.m4b", tree(self.inbox))

    def test_discard(self):
        j = self.new_download()
        self.p.resolve_duplicate(j.id, "discard")
        self.assertEqual(j.status, "cancelled")
        self.assertEqual(tree(self.lib), ["Sebastian Fitzek/Der Insasse/Der Insasse.m4b"])
        self.assertIn("_fertig/Sebastian Fitzek - Der Insasse/01.mp3", tree(self.inbox))
        self.assertEqual(self.p.scan_once(settle=0), [])       # not picked up again

    def test_skip_setting(self):
        self.settings["ab_duplicates"] = "skip"
        j = self.new_download()
        self.assertEqual(j.status, "cancelled")
        self.assertIn("Übersprungen", j.message)

    def test_other_book_is_filed_and_indexed(self):
        self.p.client.books.append(product("B0RAETSEL1", "Das Rätsel", ["Max Muster"], 3.0))
        tone(os.path.join(self.inbox, "Max Muster - Das Rätsel", "01.mp3"), 3.0)
        j = self.p.scan_once(settle=0)[0]
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(len(self.idx.find({"asin": "B0RAETSEL1"})), 1)


if __name__ == "__main__":
    unittest.main()


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg fehlt")
class ConvertLibraryTest(unittest.TestCase):
    """Older MP3 books in the library become one M4B that replaces them."""

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="conv-")
        self.dbfile = os.path.join(self.root, "t.db")
        self.db = dbm.Database(self.dbfile)
        self.inbox = os.path.join(self.root, "inbox")
        self.lib = os.path.join(self.root, "lib")
        os.makedirs(self.inbox)
        self.book = os.path.join(self.lib, "Sebastian Fitzek", "Der Insasse")
        tone(os.path.join(self.book, "CD1", "Track 01.mp3"), 2.0)
        tone(os.path.join(self.book, "CD1", "Track 02.mp3"), 1.0)
        tone(os.path.join(self.book, "CD2", "Track 01.mp3"), 1.0)
        with open(os.path.join(self.book, "folder.jpg"), "wb") as fh:
            fh.write(b"\xff\xd8\xff" + b"0" * 100)
        self.settings = {"ab_region": "de", "ab_match": "auto", "ab_threshold": 0.8, "ab_chapters": "files",
                         "ab_duplicates": "ask", "ab_library_originals": "archive"}
        self.idx = library_index.LibraryIndex(self.lib, self.db,
                                              read_m4b=tagger.read_book, probe=meta_cache.probe)
        self.idx.scan()
        # CD1/CD2 are one book, not two
        self.assertEqual([(e["path"], e["files"], e["format"]) for e in self.idx.entries.values()],
                         [("Sebastian Fitzek/Der Insasse", 3, "mp3")])
        self.p = pipeline.Pipeline(lambda: self.settings, self.db, self.inbox,
                                   self.lib, os.path.join(self.inbox, ".work"), start=False, index=self.idx,
                                   client=FakeClient([product("B0INSASSE1", "Der Insasse", ["Sebastian Fitzek"], 4.0)]))

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_convert_with_own_chapter_names_and_renamed_tracks(self):
        names = ["Prolog", "Die Klinik", "Epilog"]
        j = self.p.add(self.book, origin="library", chapter_names=names, rename_files=True)
        self.assertEqual(j.rel, os.path.join("Sebastian Fitzek", "Der Insasse"))
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)       # not flagged as a duplicate of itself
        self.assertEqual(tree(self.lib), ["Sebastian Fitzek/Der Insasse/Der Insasse.m4b",
                                          "Sebastian Fitzek/Der Insasse/cover.jpg",   # the old folder.jpg
                                          "Sebastian Fitzek/Der Insasse/desc.txt",
                                          "Sebastian Fitzek/Der Insasse/metadata.json",
                                          "Sebastian Fitzek/Der Insasse/reader.txt"])
        import encoder
        self.assertEqual([c["title"] for c in encoder.file_chapters(j.dest)], names)
        old = sorted(f for f in tree(self.inbox) if f.startswith("_ersetzt/"))
        self.assertEqual(old, ["_ersetzt/Sebastian Fitzek/Der Insasse/CD1/01 - Prolog.mp3",
                               "_ersetzt/Sebastian Fitzek/Der Insasse/CD1/02 - Die Klinik.mp3",
                               "_ersetzt/Sebastian Fitzek/Der Insasse/CD2/03 - Epilog.mp3",
                               "_ersetzt/Sebastian Fitzek/Der Insasse/folder.jpg"])
        entries = list(self.idx.entries.values())
        self.assertEqual([(e["path"], e["format"]) for e in entries], [("Sebastian Fitzek/Der Insasse", "m4b")])

    def test_query_from_library_path_and_delete_mode(self):
        self.settings["ab_library_originals"] = "delete"
        j = self.p.add(self.book, origin="library")
        self.p.process(j)
        self.assertEqual(j.hints["query"], "Sebastian Fitzek Der Insasse")
        self.assertEqual(j.status, "done", j.log)
        self.assertFalse(any(f.startswith("_ersetzt") for f in tree(self.inbox)))

    def test_rename_tracks_swaps_safely(self):
        d = os.path.join(self.root, "swap")
        for n in ("a.mp3", "b.mp3"):
            tone(os.path.join(d, n), 0.2)
        out = pipeline.rename_tracks(d, ["a.mp3", "b.mp3"], ["B/x: y", "A"])
        self.assertEqual(out, ["01 - B-x- y.mp3", "02 - A.mp3"])
        self.assertEqual(sorted(os.listdir(d)), out)
