"""Fallback metadata sources and Audiobookshelf integration."""
import http.server
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import abs as absm  # noqa: E402
import audible  # noqa: E402
import db as dbm  # noqa: E402
import providers  # noqa: E402
import tagger  # noqa: E402
from test_pipeline import FakeClient, product  # noqa: E402

# trimmed real answers (October 2026)
ITUNES = {"resultCount": 1, "results": [{
    "wrapperType": "audiobook", "collectionId": 1436685726, "artistName": "Theodor Storm",
    "collectionName": "Der Schimmelreiter (Ungekürzt)", "releaseDate": "2018-10-24T07:00:00Z",
    "primaryGenreName": "Klassiker", "description": "Ein <b>Deichgraf</b> an der Nordsee …",
    "artworkUrl100": "https://is1-ssl.mzstatic.com/image/thumb/Music211/v4/bd/rm_image.jpg/100x100bb.jpg",
    "collectionViewUrl": "https://books.apple.com/de/audiobook/der-insasse/id1436685726?uo=4"}]}
OPENLIBRARY = {"numFound": 1, "docs": [{"author_name": ["Theodor Storm"], "cover_i": 10249865,
                                       "first_publish_year": 2018, "key": "/works/OL20368308W",
                                       "title": "Der Schimmelreiter"}]}
AUDNEXUS = {"asin": "B0TEST0001", "authors": [{"asin": "B0TEST0003", "name": "Theodor Storm"}],
            "narrators": [{"name": "Max Muster"}], "formatType": "unabridged",
            "genres": [{"name": "Krimis & Thriller", "type": "genre"}, {"name": "Psychothriller", "type": "tag"}],
            "image": "https://m.media-amazon.com/images/I/71iXCQYKQ2L.jpg", "language": "german",
            "publisherName": "Audible Studios", "releaseDate": "2018-10-24T00:00:00.000Z",
            "runtimeLengthMin": 512, "seriesPrimary": {"name": "Thriller", "position": "3"},
            "summary": "<p>Ein Deichgraf an der Nordsee …</p>", "title": "Der Schimmelreiter"}


class FakeGet:
    def __init__(self, fail=()):
        self.calls, self.fail = [], fail

    def __call__(self, url):
        self.calls.append(url)
        host = url.split("/")[2]
        if host in self.fail:
            raise audible.AudibleError(f"{host} kaputt")
        if "itunes" in url:
            return ITUNES
        if "openlibrary" in url:
            return OPENLIBRARY
        if "audnex" in url:
            return AUDNEXUS
        raise AssertionError(url)


class BrokenClient(FakeClient):
    def search(self, *a, **kw):
        raise audible.AudibleError("Audible nicht erreichbar: timeout")

    def product(self, asin, region):
        raise audible.AudibleError("Audible nicht erreichbar: timeout")


class NormaliseTest(unittest.TestCase):
    def test_itunes(self):
        b = providers.from_itunes(ITUNES["results"][0])
        self.assertEqual((b["title"], b["authors"], b["source"]), ("Der Schimmelreiter", ["Theodor Storm"], "apple"))
        self.assertTrue(b["cover_url"].endswith("/1400x1400bb.jpg"))
        self.assertEqual(b["url"], "https://books.apple.com/de/audiobook/der-insasse/id1436685726")
        self.assertEqual(b["release_date"], "2018-10-24")
        self.assertNotIn("<b>", b["description"])

    def test_openlibrary(self):
        b = providers.from_openlibrary(OPENLIBRARY["docs"][0])
        self.assertEqual(b["cover_url"], "https://covers.openlibrary.org/b/id/10249865-L.jpg")
        self.assertEqual(b["release_date"], "2018")

    def test_audnexus(self):
        b = providers.from_audnexus(AUDNEXUS, "de")
        self.assertEqual((b["narrators"], b["series"], b["series_position"], b["runtime_min"]),
                         (["Max Muster"], "Thriller", "3", 512))
        self.assertEqual((b["genres"], b["tags"], b["language"]), (["Krimis & Thriller"], ["Psychothriller"], "German"))


class LookupTest(unittest.TestCase):
    hints = {"query": "Storm Der Schimmelreiter", "title": "Der Schimmelreiter", "author": "Theodor Storm", "duration": 512 * 60}

    def test_audible_first_no_other_calls(self):
        get = FakeGet()
        p = providers.Providers(FakeClient([product("B0TEST0001", "Der Schimmelreiter", ["Theodor Storm"], 512 * 60)]), get)
        ranked, notes = p.lookup(self.hints, "de")
        self.assertEqual(ranked[0]["source"], "audible")
        self.assertEqual(get.calls, [])

    def test_audible_down_asin_from_audnexus_is_accepted(self):
        p = providers.Providers(BrokenClient([]), FakeGet())
        ranked, notes = p.lookup({**self.hints, "asin": "B0TEST0001"}, "de")
        self.assertEqual(ranked[0]["source"], "audnexus")
        self.assertIn("Audnexus", " ".join(notes))
        self.assertEqual(providers.auto_match(ranked, 0.8)["asin"], "B0TEST0001")

    def test_other_shops_only_for_review(self):
        get = FakeGet(fail=("openlibrary.org",))
        p = providers.Providers(FakeClient([]), get)
        ranked, notes = p.lookup(self.hints, "de")
        self.assertEqual([b["source"] for b in ranked], ["apple"])
        self.assertGreater(ranked[0]["match"]["score"], 0.8)
        self.assertIsNone(providers.auto_match(ranked, 0.8))   # no runtime, no narrator: a person decides
        self.assertTrue(any("Open Library" in n for n in notes))
        self.assertIn("country=de", get.calls[0])

    def test_switched_off(self):
        get = FakeGet()
        p = providers.Providers(FakeClient([]), get, enabled=lambda: False)
        self.assertEqual(p.lookup(self.hints, "de")[0], [])
        self.assertEqual(get.calls, [])

    def test_manual_search_reports_audible_errors_when_nothing_else_helps(self):
        p = providers.Providers(BrokenClient([]), FakeGet(), fallbacks=())
        with self.assertRaises(audible.AudibleError):
            p.search(self.hints, "de")
        self.assertEqual(providers.Providers(BrokenClient([]), FakeGet()).search(self.hints, "de")[0]["source"], "apple")

    def test_only_known_hosts(self):
        with self.assertRaises(audible.AudibleError):
            providers.get_json("https://evil.example/search.json")
        with self.assertRaises(audible.AudibleError):
            providers.get_json("http://itunes.apple.com/search")      # https only


class AbsMetadataTest(unittest.TestCase):
    def test_metadata_json_and_roundtrip(self):
        book = {"title": "Der Schimmelreiter", "authors": ["Theodor Storm"], "narrators": ["Max Muster"],
                "series": "Thriller", "series_position": "3", "release_date": "2018-10-24", "asin": "B0TEST0001",
                "genres": ["Krimi"], "description": "Text", "language": "German", "format_type": "unabridged"}
        meta = absm.metadata(book, [{"start": 0, "title": "Eins"}, {"start": 10.5, "title": ""}], 20.0)
        self.assertEqual(meta["series"], ["Thriller #3"])
        self.assertEqual(meta["publishedYear"], "2018")
        self.assertEqual(meta["chapters"], [{"id": 0, "start": 0, "end": 10.5, "title": "Eins"},
                                            {"id": 1, "start": 10.5, "end": 20.0, "title": "Kapitel 2"}])
        self.assertFalse(meta["abridged"])
        d = tempfile.mkdtemp()
        try:
            absm.write_metadata(d, book, [], 0)
            back = tagger.load_manual(d)       # a book that already has one is recognised from it
            self.assertEqual((back["title"], back["series"], back["series_position"], back["narrators"]),
                             ("Der Schimmelreiter", "Thriller", "3", ["Max Muster"]))
            self.assertEqual(back["release_date"], "2018-10-24")
        finally:
            shutil.rmtree(d)


class FakeAbs(http.server.BaseHTTPRequestHandler):
    calls = []

    def log_message(self, *a):
        pass

    def _answer(self):
        FakeAbs.calls.append((self.command, self.path, self.headers.get("Authorization")))
        if self.path == "/redirect/api/libraries":
            self.send_response(302)
            self.send_header("Location", "http://evil.example/steal")
            self.end_headers()
            return
        if self.path.endswith("/slow/scan"):
            time.sleep(1.0)                     # Audiobookshelf answers when the scan is done
        if self.headers.get("Authorization") != "Bearer TOKEN":
            self.send_response(401)
            self.end_headers()
            return
        body = json.dumps({"libraries": [{"id": "lib1", "name": "Hörbücher", "mediaType": "book"},
                                         {"id": "lib2", "name": "Podcasts", "mediaType": "podcast"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = _answer


class AudiobookshelfTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeAbs)
        cls.url = f"http://127.0.0.1:{cls.srv.server_address[1]}"
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()

    def setUp(self):
        FakeAbs.calls.clear()
        self.dir = tempfile.mkdtemp()
        self.db = dbm.Database(os.path.join(self.dir, "t.db"))
        self.abs = absm.Audiobookshelf(self.db, delay=0.05)

    def tearDown(self):
        self.db.close()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_setup_picks_the_book_library_and_scans_once(self):
        self.abs.update_config({"url": self.url, "token": "TOKEN"})
        self.assertNotIn("TOKEN", json.dumps(self.abs.public_config()))
        self.assertIn("Hörbücher", self.abs.test())
        self.assertTrue(self.abs.configured())
        for _ in range(5):                     # five books in a row …
            self.abs.request_scan()
        time.sleep(0.4)
        scans = [c for c in FakeAbs.calls if c[0] == "POST"]
        self.assertEqual(scans, [("POST", "/api/libraries/lib1/scan", "Bearer TOKEN")])   # … one scan

    def test_long_scan_is_not_an_error(self):
        self.abs.update_config({"url": self.url, "token": "TOKEN", "library": "slow"})
        self.abs.scan_timeout = 0.3
        self.abs.scan_now()
        self.assertEqual(self.abs.last, "Scan läuft")

    def test_wrong_token(self):
        self.abs.update_config({"url": self.url, "token": "WRONG"})
        with self.assertRaises(absm.AbsError):
            self.abs.test()

    def test_token_does_not_follow_a_new_address_or_a_redirect(self):
        self.abs.update_config({"url": self.url, "token": "TOKEN", "library": "lib1"})
        self.abs.update_config({"url": "http://other.example:13378", "token": "", "library": "lib1"})
        self.assertFalse(self.abs.public_config()["token_set"])
        self.abs.update_config({"url": self.url + "/redirect", "token": "TOKEN"})
        with self.assertRaises(absm.AbsError):
            self.abs.libraries()
        self.assertEqual([c[1] for c in FakeAbs.calls], ["/redirect/api/libraries"])


if __name__ == "__main__":
    unittest.main()
