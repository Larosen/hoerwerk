import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
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
                    ("Sebastian_Fitzek-Der_Insasse-Ungekuerzt-DE-2018-AUDIOBOOK-GRP", 400 << 20, 50),
                    ("Sebastian Fitzek - Der Insasse (gekuerzt) MP3 German", 200 << 20, 500),
                    ("Sebastian.Fitzek.The.Inmate.Unabridged.English.M4B", 300 << 20, 5),
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
        self.assertEqual(self.u.test("indexer", idx["Eins"]), "idx1")
        with self.assertRaises(usenet.UsenetError) as cm:
            self.u.test("indexer", idx["Kaputt"])
        self.assertIn("Incorrect user credentials", str(cm.exception))
        self.assertEqual(self.u.test("sab"), "SABnzbd 4.4.1")

    def test_search_merges_ranks_and_hides_links(self):
        res = self.u.search("Fitzek Insasse", language="de")
        self.assertEqual(len(res["errors"]), 1)
        self.assertIn("Kaputt", res["errors"][0])
        titles = [r["title"] for r in res["results"]]
        self.assertEqual(len(titles), 3)                     # duplicates from both indexers merged
        self.assertTrue(titles[0].startswith("Sebastian_Fitzek-Der_Insasse-Ungekuerzt"))   # German, unabridged
        self.assertEqual(titles[-1], "Sebastian.Fitzek.The.Inmate.Unabridged.English.M4B")
        top = res["results"][0]
        self.assertEqual(top["also"], ["Zwei"])
        self.assertEqual(top["language"], "de")
        self.assertNotIn("link", top)
        self.assertNotIn("SECRETKEY", json.dumps(res))
        cats = [q.get("cat") for p, q in Fake.calls if q.get("t") == "search"]
        self.assertIn("3030,7020", cats)

    def test_send_to_sab_and_hint(self):
        res = self.u.search("Fitzek Insasse", language="de")
        out = self.u.send(res["results"][0]["id"])
        self.assertEqual(out["nzo_id"], "SABnzbd_nzo_abc")
        add = [q for p, q in Fake.calls if q.get("mode") == "addurl"][0]
        self.assertTrue(add["name"].startswith("https://idx1/getnzb/0.nzb"))
        self.assertEqual(add["cat"], "audiobooks")
        self.assertEqual(add["nzbname"], res["results"][0]["title"])
        # SABnzbd names the folder after the release -> the pipeline gets the search terms back
        hint = self.u.hint_for("Sebastian Fitzek-Der Insasse-Ungekuerzt-DE-2018-AUDIOBOOK-GRP")
        self.assertEqual(hint["query"], "Fitzek Insasse")
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
        self.assertTrue(usenet.release_info("Die drei Fragezeichen Hoerspiel Folge 1")["drama"])

    def test_umlaut_spellings_are_all_searched(self):
        res = self.u.search("Fitzek Insässe")
        sent = sorted({q["q"] for p, q in Fake.calls if q.get("t") == "search" and p.startswith("/idx1")})
        self.assertEqual(sent, ["Fitzek Insaesse", "Fitzek Insasse", "Fitzek Insässe"])
        self.assertEqual(res["queries"], ["Fitzek Insässe", "Fitzek Insaesse", "Fitzek Insasse"])
        self.assertEqual(len(res["results"]), 3)           # same releases from 3 queries x 2 indexers: merged
        self.assertEqual(len(res["errors"]), 1)            # one message per broken indexer, not per query

    def test_extra_queries_from_audible_book(self):
        self.u.search("Fitzek Der Insasse", extra=["Der Insasse"])
        sent = {q["q"] for p, q in Fake.calls if q.get("t") == "search" and p.startswith("/idx1")}
        self.assertEqual(sent, {"Fitzek Der Insasse", "Der Insasse"})

    def test_rank_ignores_umlaut_spelling(self):
        def r(title, **kw):
            return {"title": title, "grabs": 0, "published": 0, **usenet.release_info(title), **kw}
        good = r("Tolkien-Die_Gefaehrten-Ungekuerzt-DE-AUDIOBOOK")
        other = r("Tolkien-Der_Hobbit-DE-AUDIOBOOK", grabs=999)
        key = lambda x: usenet.Usenet._rank(x, "Tolkien Die Gefährten", "de")  # noqa: E731
        self.assertGreater(key(good), key(other))
        self.assertEqual(usenet.match_words("Die Gefährten"), usenet.match_words("DIE.GEFAHRTEN"))
        self.assertEqual(usenet.match_words("Großvater"), usenet.match_words("Grossvater"))

    def test_api_url(self):
        self.assertEqual(usenet.api_url("api.nzbgeek.info"), "http://api.nzbgeek.info/api")
        self.assertEqual(usenet.api_url("http://prowlarr:9696/1/"), "http://prowlarr:9696/1/api")
        self.assertEqual(usenet.api_url("https://x/api"), "https://x/api")


if __name__ == "__main__":
    unittest.main()
