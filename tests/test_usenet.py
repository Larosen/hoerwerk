import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
from urllib import parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import db as dbm  # noqa: E402
import usenet  # noqa: E402

FEED = """<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/">
<channel><title>{name}</title>
{items}
</channel></rss>"""
ITEM = """<item><title>{title}</title><guid>https://{host}/details/{n}</guid>
<link>https://{host}/getnzb/{n}.nzb&amp;r=SECRETKEY123</link>
<comments>https://{host}/details/{n}#comments</comments>
<pubDate>Mon, 26 Oct 2026 10:00:00 +0000</pubDate><category>Audio &gt; Audiobook</category>
<enclosure url="https://{host}/getnzb/{n}.nzb&amp;r=SECRETKEY123" length="{size}" type="application/x-nzb"/>
<newznab:attr name="category" value="3030"/><newznab:attr name="size" value="{size}"/>
<newznab:attr name="grabs" value="{grabs}"/></item>"""


class Fake(BaseHTTPRequestHandler):
    calls = []

    def log_message(self, *a):
        pass

    def do_GET(self):
        url = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        Fake.calls.append((url.path, q))
        if url.path.startswith("/idx"):
            name = url.path.split("/")[1]
            if q.get("apikey") != "goodkey":
                body = b'<?xml version="1.0"?><error code="100" description="Incorrect user credentials"/>'
            elif q.get("t") == "caps":
                body = f'<caps><server title="{name}"/></caps>'.encode()
            else:
                items = [
                    ("Theodor_Storm-Der_Schimmelreiter-Ungekuerzt-DE-2018-AUDIOBOOK-GRP", 400 << 20, 50),
                    ("Theodor Storm - Der Schimmelreiter (gekuerzt) MP3 German", 200 << 20, 500),
                    ("Theodor.Storm.The.Dykemaster.Unabridged.English.M4B", 300 << 20, 5),
                ]
                body = FEED.format(name=name, items="".join(
                    ITEM.format(title=t, host=name, n=i, size=s, grabs=g) for i, (t, s, g) in enumerate(items))).encode()
            ctype = "application/xml"
        elif url.path == "/sab/api":
            if q.get("apikey") != "sabkey":
                body = json.dumps({"status": False, "error": "API Key Incorrect"}).encode()
            elif q["mode"] == "version":
                body = b'{"version": "4.4.1"}'
            elif q["mode"] == "get_cats":
                body = b'{"categories": ["*", "audiobooks", "tv"]}'
            elif q["mode"] == "addurl":
                body = b'{"status": true, "nzo_ids": ["SABnzbd_nzo_abc"]}'
            elif q["mode"] == "queue":
                body = json.dumps({"queue": {"paused": False, "speed": "12.3 M", "slots": [
                    {"nzo_id": "a", "filename": "Buch A", "cat": "audiobooks", "percentage": "42", "status": "Downloading",
                     "size": "400 MB", "timeleft": "0:01:10"},
                    {"nzo_id": "b", "filename": "Serie S01", "cat": "tv", "percentage": "10", "status": "Downloading"}]}}).encode()
            else:
                body = json.dumps({"history": {"slots": [
                    {"nzo_id": "c", "name": "Buch C", "category": "audiobooks", "status": "Completed",
                     "storage": "/downloads/audiobooks/Buch C", "size": "300 MB", "completed": 1761000000},
                    {"nzo_id": "d", "name": "Buch D", "category": "audiobooks", "status": "Failed",
                     "fail_message": "Out of retention", "size": "1 GB"}]}}).encode()
            ctype = "application/json"
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class UsenetTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Fake)
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def setUp(self):
        Fake.calls.clear()
        self.cfg = tempfile.mkdtemp(prefix="usenet-")
        self.db = dbm.Database(os.path.join(self.cfg, "t.db"))
        self.u = usenet.Usenet(self.db)
        self.u.update_config({
            "indexers": [
                {"name": "Eins", "url": f"{self.base}/idx1", "apikey": "goodkey"},
                {"name": "Zwei", "url": f"{self.base}/idx2/api", "apikey": "goodkey", "categories": "3030,7020"},
                {"name": "Kaputt", "url": f"{self.base}/idx3", "apikey": "wrong"},
            ],
            "sab": {"url": f"{self.base}/sab", "apikey": "sabkey", "category": "audiobooks"},
        })

    def tearDown(self):
        shutil.rmtree(self.cfg, ignore_errors=True)

    def test_keys_are_never_returned_and_kept_when_empty(self):
        pub = self.u.public_config()
        self.assertNotIn("goodkey", json.dumps(pub))
        self.assertNotIn("sabkey", json.dumps(pub))
        self.assertTrue(pub["sab"]["apikey_set"])
        self.assertEqual(pub["configured"], {"indexers": 3, "sab": True})
        # resave from the UI (keys empty) keeps them
        pub["indexers"][0]["apikey"] = ""
        pub["sab"]["apikey"] = ""
        self.u.update_config(pub)
        saved = self.db.get("sources")
        self.assertEqual(saved["indexers"][0]["apikey"], "goodkey")
        self.assertEqual(saved["sab"]["apikey"], "sabkey")

    def test_tests(self):
        idx = {i["name"]: i["id"] for i in self.u.public_config()["indexers"]}
        self.assertEqual(self.u.test("indexer", idx["Eins"]), "idx1 · Hörbuch-Kategorien: 3030")
        with self.assertRaises(usenet.UsenetError) as cm:
            self.u.test("indexer", idx["Kaputt"])
        self.assertIn("Incorrect user credentials", str(cm.exception))
        self.assertEqual(self.u.test("sab"), "SABnzbd 4.4.1")

    def test_search_merges_ranks_and_hides_links(self):
        res = self.u.search("Storm Schimmelreiter", language="de")
        self.assertEqual(len(res["errors"]), 1)
        self.assertIn("Kaputt", res["errors"][0])
        titles = [r["title"] for r in res["results"]]
        self.assertEqual(len(titles), 3)                     # duplicates from both indexers merged
        self.assertTrue(titles[0].startswith("Theodor_Storm-Der_Schimmelreiter-Ungekuerzt"))   # German, unabridged
        self.assertEqual(titles[-1], "Theodor.Storm.The.Dykemaster.Unabridged.English.M4B")
        top = res["results"][0]
        self.assertEqual(top["also"], ["Zwei"])
        self.assertEqual(top["language"], "de")
        self.assertNotIn("link", top)
        self.assertNotIn("SECRETKEY", json.dumps(res))
        cats = [q.get("cat") for p, q in Fake.calls if q.get("t") == "search"]
        self.assertIn("3030,7020,3000", cats)          # own categories kept, the whole audio area (3000) added

    def test_send_to_sab_and_hint(self):
        res = self.u.search("Storm Schimmelreiter", language="de")
        out = self.u.send(res["results"][0]["id"])
        self.assertEqual(out["nzo_id"], "SABnzbd_nzo_abc")
        add = [q for p, q in Fake.calls if q.get("mode") == "addurl"][0]
        self.assertTrue(add["name"].startswith("https://idx1/getnzb/0.nzb"))
        self.assertEqual(add["cat"], "audiobooks")
        self.assertEqual(add["nzbname"], res["results"][0]["title"])
        # SABnzbd names the folder after the release -> the pipeline gets the search terms back
        hint = self.u.hint_for("Theodor Storm-Der Schimmelreiter-Ungekuerzt-DE-2018-AUDIOBOOK-GRP")
        self.assertEqual(hint["query"], "Storm Schimmelreiter")
        self.assertIsNone(self.u.hint_for("Etwas anderes"))
        with self.assertRaises(usenet.UsenetError):
            self.u.send("abgelaufen")

    def test_activity_only_own_category(self):
        a = self.u.activity()
        names = [i["name"] for i in a["items"]]
        self.assertEqual(names, ["Buch A", "Buch C", "Buch D"])
        self.assertAlmostEqual(a["items"][0]["progress"], 0.42)
        self.assertEqual(a["items"][2]["error"], "Out of retention")

    def test_wrong_sab_key(self):
        self.u.update_config({"indexers": [], "sab": {"url": f"{self.base}/sab", "apikey": "nope"}})
        with self.assertRaises(usenet.UsenetError) as cm:
            self.u.test("sab")
        self.assertIn("API Key Incorrect", str(cm.exception))

    def test_release_info(self):
        i = usenet.release_info("Autor_-_Titel-Ungekuerzt-DE-2020-128kbps-MP3")
        self.assertEqual((i["format"], i["language"], i["bitrate"], i["abridged"]), ("MP3", "de", 128, False))
        i = usenet.release_info("Author - Title (Abridged) English M4B 64k")
        self.assertEqual((i["format"], i["language"], i["bitrate"], i["abridged"]), ("M4B", "en", 64, True))
        self.assertTrue(usenet.release_info("Grimms Maerchen Hoerspiel Folge 1")["drama"])

    def test_umlaut_spellings_are_all_searched(self):
        res = self.u.search("Grimm Märchen")
        sent = sorted({q["q"] for p, q in Fake.calls if q.get("t") == "search" and p.startswith("/idx1")})
        self.assertEqual(sent, ["Grimm Maerchen", "Grimm Marchen", "Grimm Märchen"])
        self.assertEqual(res["queries"], ["Grimm Märchen", "Grimm Maerchen", "Grimm Marchen"])
        self.assertEqual(len(res["results"]), 3)           # same releases from 3 queries x 2 indexers: merged
        self.assertEqual(len(res["errors"]), 1)            # one message per broken indexer, not per query

    def test_extra_queries_from_audible_book(self):
        self.u.search("Storm Der Schimmelreiter", extra=["Der Schimmelreiter"])
        sent = {q["q"] for p, q in Fake.calls if q.get("t") == "search" and p.startswith("/idx1")}
        self.assertEqual(sent, {"Storm Der Schimmelreiter", "Der Schimmelreiter"})

    def test_rank_ignores_umlaut_spelling(self):
        def r(title, **kw):
            return {"title": title, "grabs": 0, "published": 0, **usenet.release_info(title), **kw}
        good = r("Hoffmann-Das_Fraeulein-Ungekuerzt-DE-AUDIOBOOK")
        other = r("Hoffmann-Der_Sandmann-DE-AUDIOBOOK", grabs=999)
        key = lambda x: usenet.Usenet._rank(x, "Hoffmann Das Fräulein", "de")  # noqa: E731
        self.assertGreater(key(good), key(other))
        self.assertEqual(usenet.match_words("Das Fräulein"), usenet.match_words("DAS.FRAULEIN"))
        self.assertEqual(usenet.match_words("Großvater"), usenet.match_words("Grossvater"))

    def test_429_pauses_that_indexer(self):
        asked = []

        def get(url):
            asked.append(url)
            if "busy" in url:
                raise usenet.RateLimited("HTTP 429", 600)
            return b'<rss><channel></channel></rss>'
        u = usenet.Usenet(self.db, get=get)
        u.update_config({"indexers": [{"name": "Busy", "url": "http://busy/api", "apikey": "k"},
                                      {"name": "Calm", "url": "http://calm/api", "apikey": "k"}]})
        res = u.search("Grimm Märchen")                     # three spellings
        busy = [a for a in asked if "busy" in a and "t=search" in a]
        self.assertEqual(len(busy), 1)                      # after the 429 no further query to it
        self.assertEqual(len([a for a in asked if "calm" in a and "t=search" in a]), 6)   # 3 x (audio + all)
        self.assertIn("Busy: zu viele Anfragen – Pause bis", res["errors"][0])
        self.assertTrue(u.paused("Busy"))
        asked.clear()
        u.search("Storm")                                   # still paused: not asked at all
        self.assertFalse([a for a in asked if "busy" in a and "t=search" in a])
        self.assertEqual(usenet._retry_after("120"), 120)
        self.assertEqual(usenet._retry_after(None), usenet.RATE_PAUSE)

    def test_api_url(self):
        self.assertEqual(usenet.api_url("api.indexer.example"), "http://api.indexer.example/api")
        self.assertEqual(usenet.api_url("http://prowlarr:9696/1/"), "http://prowlarr:9696/1/api")
        self.assertEqual(usenet.api_url("https://x/api"), "https://x/api")


if __name__ == "__main__":
    unittest.main()


class AudiobookCategoryTest(unittest.TestCase):
    """Indexers with their own audiobook categories (like many German ones): found and searched."""

    CAPS = b"""<?xml version="1.0"?><caps><server title="TM"/><categories>
      <category id="3000" name="Audio"><subcat id="3010" name="MP3"/><subcat id="3030" name="Audiobook"/>
        <subcat id="3040" name="Lossless"/></category>
      <category id="100000" name="Deutsch"><subcat id="100030" name="H&#246;rbuch DE"/><subcat id="100040" name="H&#246;rspiele"/>
        <subcat id="100050" name="Filme"/></category>
      <category id="7000" name="Books"><subcat id="7020" name="eBook"/></category></categories></caps>"""

    def item(self, title, cat, cat_name=""):
        return (f'<item><title>{title}</title><link>https://idx/get/1</link><category>{cat_name}</category>'
                f'<enclosure url="https://idx/get/{abs(hash(title))}" length="1000"/>'
                f'<newznab:attr name="category" value="{cat}"/></item>')

    def feed(self, *items):
        return ('<?xml version="1.0"?><rss xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/">'
                f'<channel>{"".join(items)}</channel></rss>').encode()

    def setUp(self):
        usenet._CAPS.clear()
        self.calls = []

    def test_own_categories_from_caps(self):
        # the whole audio area (Goethe readings often sit under "Audio > Lossless"), plus own audiobook categories
        self.assertEqual(usenet.audiobook_categories(self.CAPS), ["3000", "3010", "3030", "3040", "100030", "100040"])

    def test_every_audio_subcategory_counts(self):
        """The categories of a real German indexer: audiobooks can be in any of them."""
        caps = b"""<caps><categories><category id="3000" name="Audio">
          <subcat id="3010" name="MP3"/><subcat id="3020" name="Video"/><subcat id="3030" name="Audiobook"/>
          <subcat id="3040" name="Lossless"/><subcat id="3060" name="Audiobook - DE"/><subcat id="3070" name="Audiobook - ES"/>
          <subcat id="3080" name="Audiobook - FR"/><subcat id="3999" name="Other"/></category>
          <category id="2000" name="Movies"><subcat id="2040" name="HD"/></category></categories></caps>"""
        self.assertEqual(usenet.audiobook_categories(caps),
                         ["3000", "3010", "3020", "3030", "3040", "3060", "3070", "3080", "3999"])

    def test_searches_own_categories_then_falls_back_to_all(self):
        def get(url):
            q = dict(parse.parse_qsl(parse.urlparse(url).query))
            self.calls.append(q)
            if q["t"] == "caps":
                return self.CAPS
            if "cat" in q:
                return self.feed()                                   # nothing in the audiobook categories
            return self.feed(self.item("Shida Bazyar-Die Luecken (Ungekuerzt) DE MP3-ABook", "100031", "Audio > Audiobook DE"),
                             self.item("Shida Bazyar Die Luecken German EPUB", "7020", "Books > eBook"),
                             self.item("Die Luecken 2026 German 1080p", "2040", "Movies > HD"))
        n = usenet.Newznab("TM", "https://idx", "key", "3030", get)
        found = n.search("Shida Bazyar")
        self.assertEqual([r["title"] for r in found], ["Shida Bazyar-Die Luecken (Ungekuerzt) DE MP3-ABook"])
        searches = [c for c in self.calls if c["t"] == "search"]
        self.assertEqual(searches[0]["cat"], "3030,3000,3010,3040,100030,100040")       # 3000 is in already
        self.assertNotIn("cat", searches[1])
        n.search("noch mal")
        self.assertEqual(sum(1 for c in self.calls if c["t"] == "caps"), 1)    # caps only once a day
