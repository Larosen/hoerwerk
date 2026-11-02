import os
import re
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
            product("B0STORM001", "Der Schimmelreiter", ["Theodor Storm"], 4.0),
            product("B0RAETSEL1", "Das Rätsel", ["Max Muster"], 3.0, series="Rätselreihe", series_position="2"),
            product("B0RAETSEL2", "Das Rätsel", ["Eva Andere"], 3.0),
        ], chapters={"B0STORM001": {"accurate": True, "runtime_ms": 4000, "intro_ms": 0, "outro_ms": 0,
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
        book = os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter (Ungekürzt)")
        tone(os.path.join(book, "CD1", "01.mp3"), 1.0, tags={"title": "Track 1"})
        tone(os.path.join(book, "CD1", "02.mp3"), 1.0, tags={"title": "Track 2"})
        tone(os.path.join(book, "CD2", "01.mp3"), 2.0, tags={"title": "Track 3"})
        jobs = self.run_all()
        self.assertEqual(len(jobs), 1)
        j = jobs[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(tree(self.lib), ["Theodor Storm/Der Schimmelreiter/Der Schimmelreiter.m4b",
                                          "Theodor Storm/Der Schimmelreiter/metadata.json"])
        self.assertEqual(tree(self.inbox), ["_fertig/Theodor Storm - Der Schimmelreiter (Ungekürzt)/CD1/01.mp3",
                                            "_fertig/Theodor Storm - Der Schimmelreiter (Ungekürzt)/CD1/02.mp3",
                                            "_fertig/Theodor Storm - Der Schimmelreiter (Ungekürzt)/CD2/01.mp3"])
        # generic "Track n" chapters were replaced by Audible's (runtime matches)
        import encoder
        chs = encoder.file_chapters(j.dest)
        self.assertEqual([c["title"] for c in chs], ["Prolog", "Eins", "Zwei"])
        from mutagen.mp4 import MP4
        t = MP4(j.dest).tags
        self.assertEqual(t["©alb"], ["Der Schimmelreiter"])
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
                                          "Max Muster/Rätselreihe/02 - Das Rätsel/metadata.json"])

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
        folder = os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter")
        os.makedirs(folder)
        with open(os.path.join(folder, "x.mp3"), "wb") as fh:
            fh.write(b"\x00" * 4096)
        j = self.run_all()[0]
        self.assertEqual(j.status, "failed")
        failed = "_failed/Theodor Storm - Der Schimmelreiter/"
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
        tone(os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter.m4b"), 4.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertTrue(j.converted_info.get("copied"))

    def test_chapters_txt(self):
        folder = os.path.join(self.inbox, "Storm - Der Schimmelreiter")
        tone(os.path.join(folder, "Der Schimmelreiter.m4b"), 4.0)
        with open(os.path.join(folder, "Der Schimmelreiter.chapters.txt"), "w") as fh:
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

    def test_inbox_book_is_linked_to_an_open_request(self):
        """Something in the inbox that clearly is a requested book: linked and filed without asking."""
        linked = []
        self.p.open_requests = lambda: [{"id": 7, "user": "Eva", "key": "asin:B0STORM001",
                                         "book": {"asin": "B0STORM001", "title": "Der Schimmelreiter",
                                                  "authors": ["Theodor Storm"]}}]
        self.p.on_request = linked.append
        tone(os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter", "a.mp3"), 4.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(j.hints["request_id"], 7)
        self.assertEqual(linked, [j])
        self.assertTrue(any("Anfrage von Eva" in line for line in j.log))

    def test_unsure_request_match_waits_for_an_admin(self):
        """Below the minimum certainty: the requested book is offered first, an admin decides."""
        linked = []
        self.p.open_requests = lambda: [{"id": 9, "user": "Tom", "key": "book:das ratsel|muster",
                                         "book": {"title": "Das Rätsel", "authors": ["Max Muster"], "source": "bookbeat",
                                                  "id": "bookbeat:42"}}]
        self.p.on_request = linked.append
        tone(os.path.join(self.inbox, "Raetsel", "a.mp3"), 3.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "review", j.log)
        self.assertEqual(linked, [])
        mine = [c for c in j.candidates if c.get("request_id") == 9]
        self.assertTrue(mine)
        self.p.choose(j.id, pipeline.candidate_id(mine[0]))
        self.assertEqual((j.hints["request_id"], linked), (9, [j]))
        self.assertNotIn("request_id", j.match)

    def test_wrong_download_for_a_request_is_not_taken_as_the_requested_book(self):
        """Real case: English "Twilight" (book 1) was requested, "Twilight-Saga 5" in German came. The ASIN of
        the request must not decide – the request blocks the release, the files are recognised as what they are."""
        self.client.books += [
            product("B004V0T0VK", "Twilight", ["Stephenie Meyer"], 12.0, language="English",
                    series="The Twilight Saga", series_position="1"),
            product("B0BISS0005", "Biss zum ersten Sonnenstrahl", ["Stephenie Meyer"], 4.0,
                    series="Twilight-Saga", series_position="5"),
        ]
        name = "Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010"
        tone(os.path.join(self.inbox, name, "a.mp3"), 4.0)
        self.p.hint_provider = lambda folder: ({"asin": "B004V0T0VK", "title": "Twilight", "author": "Stephenie Meyer",
                                                "request_id": 7} if folder == name else None)
        wrong = []
        self.p.on_wrong_download = lambda rid, rel, why: wrong.append((rid, rel, why))
        j = self.run_all()[0]
        self.assertEqual(wrong, [(7, name, "anderer Band (5 statt 1)")])
        self.assertNotIn("request_id", j.hints)                       # not filed as Theresa's request
        self.assertNotEqual((j.match or {}).get("asin"), "B004V0T0VK")
        self.assertEqual(j.candidates[0]["asin"], "B0BISS0005", j.log)
        self.assertTrue(any("Passt nicht" in line for line in j.log))

    def test_only_wrong_volumes_found_then_search_by_content(self):
        """Real case (7.9.1, after "Entfernen"): the shop search found only other Twilight volumes and editions.
        They are marked as not fitting (also in the browser), and the shorter search finds the right book."""
        twilight = [product("B004V0T0VK", "Twilight", ["Stephenie Meyer"], 12.0, language="English",
                            series="The Twilight Saga", series_position="1"),
                    product("B0058HTSN0", "Bis(s) zum Morgengrauen", ["Stephenie Meyer"], 13.0,
                            series="Twilight", series_position="1")]
        right = product("B0BISS0005", "Bis(s) zum ersten Sonnenstrahl", ["Stephenie Meyer"], 4.0,
                        series="Twilight", series_position="5")

        class ShopSearch(FakeClient):
            def search(self, region, keywords="", title="", author="", limit=10):
                text = " ".join([keywords, title])
                return [dict(b) for b in (twilight if "Twilight" in text or "Biss" in text else [right])]
        self.client = ShopSearch(twilight + [right])
        self.p = self.make()
        name = "Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010"
        tone(os.path.join(self.inbox, name, "a.mp3"), 4.0)
        j = self.run_all()[0]
        self.assertEqual(j.candidates[0]["asin"], "B0BISS0005", j.log)
        self.assertTrue(any("zum ersten Sonnenstrahl" in line for line in j.log))
        shown = j.to_dict(full=False)["candidates"]                     # what the browser gets
        wrong = [c for c in shown if c["asin"] == "B004V0T0VK"][0]
        self.assertIn("anderer Band", wrong["mismatch"])
        self.assertIsNone(shown[0]["mismatch"])

    def test_content_search_many_folders(self):
        """The search by content, not just for one case: German and English, dotted and dashed names, a volume
        without a title, written-out umlauts, label words. The shop here is strict (every word must be in title,
        series or author; "Bis(s)" is not "Biss") – real stores are more forgiving."""
        de, us = [], []

        def p(asin, title, author, secs, store, lang="German", series=None, pos=None):
            book = product(asin, title, [author], secs, language=lang, series=series, series_position=pos, region=store)
            (de if store == "de" else us).append(book)
        p("B0BIS00001", "Bis(s) zum Morgengrauen", "Stephenie Meyer", 13.0, "de", series="Twilight", pos="1")
        p("B0BIS00002", "Bis(s) zur Mittagsstunde", "Stephenie Meyer", 14.0, "de", series="Twilight", pos="2")
        p("B0BIS00005", "Bis(s) zum ersten Sonnenstrahl", "Stephenie Meyer", 4.0, "de", series="Twilight", pos="5")
        p("B004V0T0VK", "Twilight", "Stephenie Meyer", 12.0, "de", lang="English", series="The Twilight Saga", pos="1")
        p("B0HPDE0001", "Harry Potter und der Stein der Weisen", "J.K. Rowling", 10.0, "de", series="Harry Potter", pos="1")
        p("B0HPDE0003", "Harry Potter und der Gefangene von Askaban", "J.K. Rowling", 12.0, "de", series="Harry Potter", pos="3")
        p("B0MAERCHEN", "Märchen aus dem Wald", "Anna Autorin", 5.0, "de")
        p("B0HPUS0003", "Harry Potter and the Prisoner of Azkaban", "J.K. Rowling", 12.0, "us", lang="English",
          series="Harry Potter", pos="3")
        p("B0TWUS0001", "Twilight", "Stephenie Meyer", 12.0, "us", lang="English", series="The Twilight Saga", pos="1")

        def words(text):
            return set(re.findall(r"[a-zäöüß0-9]+", text.lower()))

        class StrictShop(FakeClient):
            def search(self, region, keywords="", title="", author="", limit=10):
                want = words(" ".join([keywords, title, author]))
                store = de if region == "de" else us if region == "us" else []
                return [dict(b) for b in store
                        if want and want <= words(" ".join([b["title"], b.get("series") or "", *b["authors"]]))]

            def product(self, asin, region):
                for b in de + us:
                    if b["asin"] == asin and b["region"] == region:
                        return dict(b)
                raise audible.AudibleError("unbekannt")

        cases = [
            ("Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010", 4.0, "B0BIS00005"),
            ("Stephenie.Meyer.-.Twilight.1.-.Bis(s).zum.Morgengrauen.German.MP3", 13.0, "B0BIS00001"),
            ("J.K. Rowling - Harry Potter 3 - The Prisoner of Azkaban (Unabridged) EN", 12.0, "B0HPUS0003"),
            ("Rowling - Harry Potter 01 - German", 10.0, "B0HPDE0001"),
            ("Anna Autorin - Maerchen aus dem Wald - Hoerbuch", 5.0, "B0MAERCHEN"),
        ]
        cases.append(("Unbekannte Autorin - Gibt es nirgends 2 - Kein Shop kennt mich", 6.0, None))
        for name, secs, want in cases:
            with self.subTest(name):
                self.client = StrictShop(de + us)
                self.p = self.make()
                for leftover in os.listdir(self.inbox):
                    shutil.rmtree(os.path.join(self.inbox, leftover), ignore_errors=True)
                tone(os.path.join(self.inbox, name, "a.mp3"), secs)
                j = self.run_all()[0]
                got = (j.match or {}).get("asin") or (j.candidates[0]["asin"] if j.candidates else None)
                if want is None:                                # in no store: nothing taken, the admin decides
                    self.assertEqual(j.status, "review", "\n".join(j.log))
                    self.assertFalse(j.match and j.match.get("asin"))
                    continue
                self.assertEqual(got, want, "\n".join(j.log))
                first = next((c for c in j.candidates if c["asin"] == want), None)
                if first:
                    self.assertIsNone(first.get("mismatch"))
                for c in j.candidates:                          # nothing that clearly isn't it comes before it
                    if c["asin"] == want:
                        break
                    self.assertTrue(c.get("mismatch"), (name, c["title"]))

    def test_existing_author_folder_is_reused(self):
        os.makedirs(os.path.join(self.lib, "Storm, Theodor"))
        tone(os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter", "a.mp3"), 4.0)
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(os.listdir(self.lib), ["Storm, Theodor"])
        self.assertTrue(j.dest.startswith(os.path.join(self.lib, "Storm, Theodor", "Der Schimmelreiter")))

    def test_download_hint_from_search(self):
        """A release name the matcher can't read, but the search terms from the search tab help."""
        name = "SF-DI-GRP-2018"
        tone(os.path.join(self.inbox, name, "a.mp3"), 4.0)
        self.p.hint_provider = lambda folder: {"query": "Theodor Storm Der Schimmelreiter"} if folder == name else None
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(j.hints["alt_query"], "Theodor Storm Der Schimmelreiter")

    def test_waiting_is_visible_and_search_downloads_wait_less(self):
        import pipeline as pl
        tone(os.path.join(self.inbox, "Von der Platte", "a.mp3"), 1.0)
        tone(os.path.join(self.inbox, "Aus der Suche", "a.mp3"), 1.0)
        self.p.hint_provider = lambda folder: {"query": "x"} if folder == "Aus der Suche" else None
        self.assertEqual(self.p.scan_once(settle=120), [])
        waiting = {w["name"]: w["ready_in"] for w in self.p.status()["waiting"]}
        self.assertGreater(waiting["Von der Platte"], 100)
        self.assertLessEqual(waiting["Aus der Suche"], pl.GRAB_SETTLE)
        self.assertTrue(any("Neu im Eingang" in e["text"] for e in pl.applog.log.since(0)))
        real = pl._now
        pl._now = lambda: real() + pl.GRAB_SETTLE + 1
        try:
            jobs = self.p.scan_once(settle=120)
        finally:
            pl._now = real
        self.assertEqual([j.name for j in jobs], ["Aus der Suche"])

    def test_release_name_beats_a_vague_search_term(self):
        """Searched "Erzähl mir", downloaded a release with a clear name: the name decides."""
        name = "Theodor.Storm-Der.Schimmelreiter.(Ungekuerzt).ABOOK"
        tone(os.path.join(self.inbox, name, "a.mp3"), 4.0)
        self.p.hint_provider = lambda folder: {"query": "Erzähl mir"} if folder == name else None
        j = self.run_all()[0]
        self.assertEqual(j.status, "done", j.log)
        self.assertEqual(j.match["title"], "Der Schimmelreiter")

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


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg fehlt")
class PackTest(unittest.TestCase):
    """One download with several volumes: one book per volume, only the wanted ones."""
    NAME = "Theodor Storm-Novellen 01 - 03-AUDIOBOOK-DE-GRP"

    def setUp(self):
        import library_index
        import tagger
        from meta import cache as meta_cache
        self.root = tempfile.mkdtemp(prefix="pack-")
        self.db = dbm.Database(os.path.join(self.root, "t.db"))
        self.inbox = os.path.join(self.root, "inbox")
        self.lib = os.path.join(self.root, "lib")
        os.makedirs(self.inbox)
        # volume 2 is already in the library (folder layout Author/Series/NN - Title)
        tone(os.path.join(self.lib, "Theodor Storm", "Novellen", "02 - Pole Poppenspaeler", "a.mp3"), 0.5)
        self.index = library_index.LibraryIndex(self.lib, self.db, read_m4b=tagger.read_book, probe=meta_cache.probe)
        self.index.scan()
        pack = os.path.join(self.inbox, self.NAME)
        tone(os.path.join(pack, "01 - Immensee", "CD1", "a.mp3"), 2.0)        # CD folders: one book
        tone(os.path.join(pack, "01 - Immensee", "CD2", "b.mp3"), 2.0)
        tone(os.path.join(pack, "02 - Pole Poppenspaeler", "a.mp3"), 3.0)
        tone(os.path.join(pack, "03 - Der Schimmelreiter", "a.mp3"), 4.0)
        self.settings = {"ab_region": "de", "ab_match": "auto", "ab_threshold": 0.8, "bitrate": "auto",
                         "ab_chapters": "files", "ab_originals": "archive", "ab_cover_source": "local"}
        client = FakeClient([product("B0IMMENSEE", "Immensee", ["Theodor Storm"], 4.0),
                             product("B0STORM001", "Der Schimmelreiter", ["Theodor Storm"], 4.0)])
        self.p = pipeline.Pipeline(lambda: self.settings, self.db, self.inbox, self.lib,
                                   os.path.join(self.inbox, ".work"), client=client, start=False, index=self.index)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def run_all(self):
        jobs = self.p.scan_once(settle=0)
        for j in jobs:
            self.p.process(j)
        return jobs

    def test_each_volume_its_own_book_owned_ones_skipped(self):
        jobs = self.run_all()
        self.assertEqual([(j.pack["volume"], j.status) for j in jobs], [(1, "done"), (3, "done")], [j.log for j in jobs])
        self.assertEqual(jobs[0].hints["author"], "Theodor Storm")      # from the release name, not the tags
        self.assertEqual(jobs[0].hints["query"], "Immensee Theodor Storm")
        books = [p for p in tree(self.lib) if p.endswith(".m4b")]
        self.assertEqual(books, ["Theodor Storm/Der Schimmelreiter/Der Schimmelreiter.m4b",
                                 "Theodor Storm/Immensee/Immensee.m4b"])
        # volume 2 was not converted, only put aside with the other originals
        self.assertIn(f"_fertig/{self.NAME}/02 - Pole Poppenspaeler/a.mp3", tree(self.inbox))

    def test_only_the_volumes_chosen_in_the_search(self):
        self.p.hint_provider = lambda folder: {"query": "Storm", "volumes": [2, 3]} if folder == self.NAME else None
        jobs = self.run_all()
        # 2 is owned but was chosen on purpose: it is processed (the duplicate question comes at the end)
        self.assertEqual([j.pack["volume"] for j in jobs], [2, 3])
        self.assertIn(f"_fertig/{self.NAME}/01 - Immensee/CD1/a.mp3", tree(self.inbox))

    def test_owned_volume_recognised_by_audible_is_skipped_before_converting(self):
        tone(os.path.join(self.lib, "Theodor Storm", "Immensee", "a.mp3"), 0.5,
             tags={"album": "Immensee", "artist": "Theodor Storm"})
        self.index.scan()
        jobs = self.run_all()
        first = jobs[0]
        self.assertEqual((first.pack["volume"], first.status), (1, "cancelled"), first.log)
        self.assertIn("schon in der Bibliothek", first.message)
        self.assertIsNone(first.m4b)                                      # never converted

    def test_disc_and_volume_folders(self):
        self.assertTrue(pipeline.are_discs(["CD1", "CD2"]))
        self.assertTrue(pipeline.are_discs(["01", "02"]))
        self.assertTrue(pipeline.are_discs(["Teil 1", "Teil 2"]))
        self.assertTrue(pipeline.are_discs(["Immensee CD1", "Immensee CD2"]))
        self.assertFalse(pipeline.are_discs(["01 - Immensee", "02 - Pole Poppenspaeler"]))   # volumes, not discs
        self.assertFalse(pipeline.are_discs(["Band 1", "Band 2"]))
