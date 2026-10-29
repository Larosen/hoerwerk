import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import db as dbm  # noqa: E402
import audible  # noqa: E402
import pipeline  # noqa: E402
from media import tone  # noqa: E402


def tree(base):
    out = []
    for root, dirs, files in os.walk(base):
        dirs[:] = [d for d in dirs if d != ".work"]
        for f in files:
            out.append(os.path.relpath(os.path.join(root, f), base))
    return sorted(out)


def product(asin, title, authors, runtime_s, **kw):
    return {"asin": asin, "title": title, "subtitle": None, "authors": authors, "narrators": ["Sprecher A"],
            "series": None, "series_position": None, "publisher": "Verlag", "release_date": "2020-01-02",
            "language": "German", "runtime_min": runtime_s / 60, "format_type": "unabridged", "genres": ["Krimi"],
            "tags": [], "description": "Beschreibung", "cover_url": None, "region": "de",
            "url": f"https://www.audible.de/pd/{asin}", "source": "audible", **kw}


class FakeClient:
    def __init__(self, books, chapters=None):
        self.books = books
        self.chapter_data = chapters or {}

    def search(self, region, keywords="", title="", author="", limit=10):
        q = audible._tokens(" ".join([keywords, title, author]))
        return [dict(b) for b in self.books if audible._tokens(b["title"]) & q]

    def product(self, asin, region):
        for b in self.books:
            if b["asin"] == asin:
                return dict(b)
        raise audible.AudibleError("unbekannt")

    def chapters(self, asin, region):
        return self.chapter_data.get(asin)


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg fehlt")
class PipelineTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pipe-")
        self.dbfile = os.path.join(self.root, "t.db")
        self.db = dbm.Database(self.dbfile)
        self.inbox = os.path.join(self.root, "inbox")
        self.lib = os.path.join(self.root, "lib")
        os.makedirs(self.inbox)
        os.makedirs(self.lib)
        self.settings = {"ab_region": "de", "ab_match": "auto", "ab_threshold": 0.8, "bitrate": "auto",
                         "ab_chapters": "auto", "ab_originals": "archive", "ab_cover_source": "audible"}
        self.client = FakeClient([
            product("B0INSASSE1", "Der Insasse", ["Sebastian Fitzek"], 4.0),
            product("B0RAETSEL1", "Das Rätsel", ["Max Muster"], 3.0, series="Rätselreihe", series_position="2"),
            product("B0RAETSEL2", "Das Rätsel", ["Eva Andere"], 3.0),
        ], chapters={"B0INSASSE1": {"accurate": True, "runtime_ms": 4000, "intro_ms": 0, "outro_ms": 0,
                                    "chapters": [{"start": 0, "title": "Prolog"}, {"start": 1.0, "title": "Eins"},
                                                 {"start": 2.5, "title": "Zwei"}]}})
        self.p = self.make()

    def make(self):
        return pipeline.Pipeline(lambda: self.settings, self.db, self.inbox,
                                 self.lib, os.path.join(self.inbox, ".work"), client=self.client, start=False)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def run_all(self):
        jobs = self.p.scan_once(settle=0)
        for j in jobs:
            self.p.process(j)
        return jobs

    def test_auto_match_cd_folders(self):
        book = os.path.join(self.inbox, "Sebastian Fitzek - Der Insasse (Ungekürzt)")
        tone(os.path.join(book, "CD1", "01.mp3"), 1.0, tags={"title": "Track 1"})
        tone(os.path.join(book, "CD1", "02.mp3"), 1.0, tags={"title": "Track 2"})
        tone(os.path.join(book, "CD2", "01.mp3"), 2.0, tags={"title": "Track 3"})
        jobs = self.run_all()
        self.assertEqual(len(jobs), 1)
        j = jobs[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(tree(self.lib), ["Sebastian Fitzek/Der Insasse/Der Insasse.m4b",
                                          "Sebastian Fitzek/Der Insasse/desc.txt",
                                          "Sebastian Fitzek/Der Insasse/metadata.json",
                                          "Sebastian Fitzek/Der Insasse/reader.txt"])
        self.assertEqual(tree(self.inbox), ["_fertig/Sebastian Fitzek - Der Insasse (Ungekürzt)/CD1/01.mp3",
                                            "_fertig/Sebastian Fitzek - Der Insasse (Ungekürzt)/CD1/02.mp3",
                                            "_fertig/Sebastian Fitzek - Der Insasse (Ungekürzt)/CD2/01.mp3"])
        # generic "Track n" chapters were replaced by Audible's (runtime matches)
        import encoder
        chs = encoder.file_chapters(j.dest)
        self.assertEqual([c["title"] for c in chs], ["Prolog", "Eins", "Zwei"])
        from mutagen.mp4 import MP4
        t = MP4(j.dest).tags
        self.assertEqual(t["©alb"], ["Der Insasse"])
        self.assertEqual(t["stik"], [2])
        self.assertFalse(os.listdir(os.path.join(self.inbox, ".work")))

    def test_review_then_choose(self):
        tone(os.path.join(self.inbox, "Das Rätsel.mp3"), 3.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "review")
        self.assertEqual({c["asin"] for c in j.candidates}, {"B0RAETSEL1", "B0RAETSEL2"})
        self.assertTrue(os.path.exists(j.m4b))
        # survives a restart
        self.p = self.make()
        j = self.p.get(j.id)
        self.assertEqual(j.status, "review")
        self.p.choose(j.id, "B0RAETSEL1", "de")
        self.assertEqual(j.status, "queued")
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(tree(self.lib), ["Max Muster/Rätselreihe/02 - Das Rätsel/Das Rätsel.m4b",
                                          "Max Muster/Rätselreihe/02 - Das Rätsel/desc.txt",
                                          "Max Muster/Rätselreihe/02 - Das Rätsel/metadata.json",
                                          "Max Muster/Rätselreihe/02 - Das Rätsel/reader.txt"])

    def test_manual_without_audible(self):
        tone(os.path.join(self.inbox, "Völlig Unbekannt", "a.mp3"), 1.0,
             tags={"album": "Völlig Unbekannt", "artist": "Jemand"})
        j = self.run_all()[0]
        self.assertEqual(j.status, "review")
        self.p.manual(j.id, {"title": "Mein Titel", "authors": "Erika Muster, Hans Muster", "series": ""})
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(tree(self.lib), ["Erika Muster/Mein Titel/Mein Titel.m4b",
                                          "Erika Muster/Mein Titel/metadata.json"])

    def test_metadata_yml(self):
        folder = os.path.join(self.inbox, "Hörspiel")
        tone(os.path.join(folder, "a.mp3"), 1.0)
        with open(os.path.join(folder, "metadata.yml"), "w", encoding="utf-8") as fh:
            fh.write("title: Eigenes Hörspiel\nauthors: [Autorin]\nnarrators: [Ensemble]\n"
                     "description: Text\ngenres: [Hörspiel]\nreleaseDate: 2001-02-03\npublisher: Selbst\n")
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertIn("Autorin/Eigenes Hörspiel/Eigenes Hörspiel.m4b", tree(self.lib))

    def test_failure_moves_to_failed_and_retry(self):
        folder = os.path.join(self.inbox, "Sebastian Fitzek - Der Insasse")
        os.makedirs(folder)
        with open(os.path.join(folder, "x.mp3"), "wb") as fh:
            fh.write(b"\x00" * 4096)
        j = self.run_all()[0]
        self.assertEqual(j.status, "failed")
        failed = "_failed/Sebastian Fitzek - Der Insasse/"
        self.assertIn(failed + "x.mp3", tree(self.inbox))
        self.assertIn(failed + "FEHLER.txt", tree(self.inbox))
        # not picked up again
        self.assertEqual(self.p.scan_once(settle=0), [])
        # repair and retry: back into the inbox, then done
        os.remove(os.path.join(self.inbox, failed, "x.mp3"))
        tone(os.path.join(self.inbox, failed, "01.mp3"), 4.0)
        self.p.retry(j.id)
        self.assertTrue(os.path.isdir(folder))
        self.assertFalse(os.path.exists(os.path.join(self.inbox, "_failed")))
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)

    def test_settle_and_partial_downloads(self):
        tone(os.path.join(self.inbox, "Neu", "a.mp3"), 1.0)
        open(os.path.join(self.inbox, "Neu", "b.mp3.part"), "w").close()
        self.assertEqual(self.p.scan_once(settle=0), [])          # still downloading
        os.remove(os.path.join(self.inbox, "Neu", "b.mp3.part"))
        self.assertEqual(self.p.scan_once(settle=60), [])         # first sighting: wait
        self.assertEqual(len(self.p.status()["waiting"]), 1)
        self.assertEqual(len(self.p.scan_once(settle=0)), 1)

    def test_book_units(self):
        tone(os.path.join(self.inbox, "Reihe", "Band 1", "a.mp3"), 0.5)
        tone(os.path.join(self.inbox, "Reihe", "Band 2", "a.mp3"), 0.5)
        tone(os.path.join(self.inbox, "Doppel", "CD 1", "a.mp3"), 0.5)
        tone(os.path.join(self.inbox, "Doppel", "CD 2", "a.mp3"), 0.5)
        units = pipeline.book_units(os.path.join(self.inbox, "Reihe"))
        self.assertEqual([os.path.basename(u) for u in units], ["Band 1", "Band 2"])
        self.assertEqual(pipeline.book_units(os.path.join(self.inbox, "Doppel")), [os.path.join(self.inbox, "Doppel")])

    def test_single_m4b_is_not_reencoded(self):
        tone(os.path.join(self.inbox, "Sebastian Fitzek - Der Insasse.m4b"), 4.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertTrue(j.converted_info.get("copied"))

    def test_chapters_txt(self):
        folder = os.path.join(self.inbox, "Fitzek - Der Insasse")
        tone(os.path.join(folder, "Der Insasse.m4b"), 4.0)
        with open(os.path.join(folder, "Der Insasse.chapters.txt"), "w") as fh:
            fh.write("00:00:00.000 Anfang\n00:00:02.000 Schluss\n")
        self.settings["ab_chapters"] = "files"
        j = self.run_all()[0]
        import encoder
        self.assertEqual([c["title"] for c in encoder.file_chapters(j.dest)], ["Anfang", "Schluss"])

    def test_link_candidate_edit_and_correct(self):
        from unittest import mock
        import links
        tone(os.path.join(self.inbox, "Unbekannt", "a.mp3"), 2.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "review")
        linked = {**product(None, "Eragon", ["Christopher Paolini"], 2.0, series="Eragon", series_position="1"),
                  "asin": None, "id": "bookbeat:11925", "source": "bookbeat"}
        with mock.patch.object(links, "fetch", return_value=linked):
            self.p.search(j.id, "https://www.bookbeat.com/de/book/eragon-11925", "de")
        self.assertEqual(j.candidates[0]["id"], "bookbeat:11925")
        self.assertEqual(j.to_dict(full=False)["candidates"][0]["source"], "bookbeat")
        # edit the candidate: drop the series, change the title
        self.p.manual(j.id, {"title": "Eragon – Das Vermächtnis", "authors": "Christopher Paolini",
                             "series": "", "series_position": ""}, base_id="bookbeat:11925")
        self.assertEqual(j.match["source"], "bookbeat")
        self.assertIsNone(j.match["series"])
        self.p.process(j)
        self.assertEqual(j.status, "done", j.log)
        self.assertIn("Christopher Paolini/Eragon – Das Vermächtnis/Eragon – Das Vermächtnis.m4b", tree(self.lib))
        # wrong after all: take it back out of the library and choose differently
        self.p.reopen(j.id)
        self.assertEqual(j.status, "review")
        self.assertEqual(tree(self.lib), [])
        self.p.choose(j.id, "bookbeat:11925")
        self.p.process(j)
        self.assertIn("Christopher Paolini/Eragon/01 - Eragon/Eragon.m4b", tree(self.lib))

    def test_existing_author_folder_is_reused(self):
        os.makedirs(os.path.join(self.lib, "Fitzek, Sebastian"))
        tone(os.path.join(self.inbox, "Sebastian Fitzek - Der Insasse", "a.mp3"), 4.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(os.listdir(self.lib), ["Fitzek, Sebastian"])
        self.assertTrue(j.dest.startswith(os.path.join(self.lib, "Fitzek, Sebastian", "Der Insasse")))

    def test_download_hint_from_search(self):
        """A release name the matcher can't read, but the search terms from the search tab help."""
        name = "SF-DI-GRP-2018"
        tone(os.path.join(self.inbox, name, "a.mp3"), 4.0)
        self.p.hint_provider = lambda folder: {"query": "Sebastian Fitzek Der Insasse"} if folder == name else None
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(j.hints["query"], "Sebastian Fitzek Der Insasse")

    def test_download_hint_with_asin(self):
        """Book picked at Audible in the search tab: the ASIN decides, even against a useless name."""
        name = "x1-GRP"
        tone(os.path.join(self.inbox, name, "a.mp3"), 3.0)
        self.p.hint_provider = lambda folder: {"query": "Max Muster Das Rätsel", "asin": "B0RAETSEL1",
                                               "title": "Das Rätsel", "author": "Max Muster"}
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(j.match["asin"], "B0RAETSEL1")       # not the other "Das Rätsel" (B0RAETSEL2)

    def test_match_off(self):
        self.settings["ab_match"] = "off"
        tone(os.path.join(self.inbox, "Irgendwas", "a.mp3"), 1.0, tags={"album": "Album X", "artist": "Künstler Y"})
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(tree(self.lib), ["Künstler Y/Album X/Album X.m4b", "Künstler Y/Album X/metadata.json"])


if __name__ == "__main__":
    unittest.main()
