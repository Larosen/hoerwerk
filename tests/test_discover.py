"""Entdecken: sources, quality rules, review headlines."""
import os
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import db as dbm  # noqa: E402
import discover  # noqa: E402
from config import default_settings  # noqa: E402


def book(asin, title, author="Anna Autorin", rating=4.6, ratings=300, genres=("Krimis & Thriller",), **kw):
    return {"asin": asin, "title": title, "authors": [author], "narrators": ["Sam Sprecher"], "rating": rating,
            "ratings": ratings, "genres": list(genres), "format_type": "unabridged", "language": "german",
            "is_adult": False, "url": f"https://www.audible.de/pd/{asin}", **kw}


class FakeIndex:
    def __init__(self, entries=None, owned_titles=()):
        self.lock = threading.Lock()
        self.entries = entries or {}
        self.owned_titles = set(owned_titles)

    def find(self, b):
        return [{"path": "x"}] if b.get("title") in self.owned_titles else []


class FakeClient:
    def __init__(self):
        self.sims = {}
        self.lists = {}
        self.hits = []

    def similar(self, asin, region, limit=20):
        return self.sims.get(asin, [])

    def genre_products(self, region, genre_id, sort="BestSellers", limit=50):
        return self.lists.get(sort, [])

    def genres(self, region):
        return [{"id": "1", "name": "Krimis & Thriller"}, {"id": "2", "name": "Geschichte"}, {"id": "3", "name": "Erotik"}]

    def search(self, region, title="", author="", limit=5, **kw):
        return [h for h in self.hits if h["title"].lower().startswith(title.lower()[:5])]


RSS = """<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel>
<item><title>Buchkritik: "Das stille Haus" von Anna Autorin</title><link>https://www.deutschlandfunkkultur.de/a-100.html</link>
<pubDate>Fri, 30 Oct 2026 10:00:00 +0100</pubDate></item>
<item><title>Bert Berger: „Die Geschichte der Wolken“</title><link>https://www.deutschlandfunkkultur.de/b-100.html</link></item>
<item><title>Ein Gespräch über Bücher</title><link>https://www.deutschlandfunkkultur.de/c-100.html</link></item>
</channel></rss>"""


class DiscoverTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.db = dbm.Database(os.path.join(self.tmp, "t.db"))
        self.settings = default_settings({})
        self.client = FakeClient()
        self.index = FakeIndex()
        self.feed = RSS.encode()
        self.d = discover.Discover(self.db, self.client, self.index, lambda: self.settings, fetch=lambda url: self.feed)

    def items(self, section, genre=""):
        self.d.refresh(section, genre, wait=True)
        return self.d.get(section, genre)["items"]

    def test_review_titles(self):
        p = discover.parse_review_title
        self.assertEqual(p('Buchkritik: "Machtspiele" von Alice Zeniter'), {"title": "Machtspiele", "author": "Alice Zeniter"})
        self.assertEqual(p('Gaia Vince: "Das nomadische Jahrhundert"'), {"title": "Das nomadische Jahrhundert", "author": "Gaia Vince"})
        self.assertEqual(p('„Zwei Leben“ von A. Autorin und B. Berger')["author"], "A. Autorin")
        self.assertIsNone(p("Ein Gespräch über Bücher"))

    def test_for_you_counts_and_filters(self):
        self.index.entries = {"a": {"asin": "S1", "title": "Seed Eins", "mtime": 2},
                              "b": {"asin": "S2", "title": "Seed Zwei", "mtime": 1}}
        self.index.owned_titles = {"Schon da"}
        shared = book("B1", "Beide mögen das")
        self.client.sims = {
            "S1": [shared, book("B2", "Nur eins"), book("B3", "Schlecht", rating=3.2),
                   book("B4", "Kaum bewertet", ratings=4), book("B5", "Gekürzt", format_type="abridged"),
                   book("B6", "Englisch", language="english"), book("B7", "Schon da")],
            "S2": [shared, book("B8", "Sachbuch", genres=["Geschichte"]), book("B9", "Erotisch", genres=["Erotik"])],
        }
        got = self.items("foryou")
        self.assertEqual([i["asin"] for i in got], ["B1", "B2"])
        self.assertEqual(got[0]["because"], ["Seed Eins", "Seed Zwei"])
        self.settings["discover_nonfiction_store"] = True
        self.assertIn("B8", [i["asin"] for i in self.items("foryou")])

    def test_genre_merges_lists(self):
        self.client.lists = {"BestSellers": [book("G1", "Eins"), book("G2", "Zwei")],
                             "-ReleaseDate": [book("G2", "Zwei"), book("G3", "Drei", rating=2.0)]}
        self.assertEqual([i["asin"] for i in self.items("genre", "1")], ["G1", "G2"])
        self.assertEqual(self.d.get("genre", "")["items"], [])

    def test_critics_only_real_audiobooks(self):
        self.client.hits = [book("R1", "Das stille Haus", rating=0, ratings=0),          # new: no ratings needed
                            book("R2", "Die Geschichte der Wolken", author="Bert Berger", genres=["Geschichte"]),
                            book("R3", "Das stille Haus", author="Jemand Anderes")]
        got = self.items("critics")
        self.assertEqual([i["asin"] for i in got], ["R1", "R2"])     # non-fiction allowed from the editorial source
        self.assertEqual(got[0]["review"]["link"], "https://www.deutschlandfunkkultur.de/a-100.html")
        self.assertEqual(got[0]["review"]["source"], "Deutschlandfunk Kultur")

    def test_feed_with_dtd_is_rejected(self):
        self.feed = b'<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><rss/>'
        self.d.refresh("critics", wait=True)
        res = self.d.get("critics")
        self.assertEqual(res["items"], [])
        self.assertTrue(res["error"])

    def test_owned_later_disappears_from_cache(self):
        self.client.lists = {"BestSellers": [book("G1", "Eins")]}
        self.assertEqual(len(self.items("genre", "1")), 1)
        self.index.owned_titles = {"Eins"}
        self.assertEqual(self.d.get("genre", "1")["items"], [])

    def test_genres_hide_adult_and_nonfiction(self):
        self.assertEqual([g["name"] for g in self.d.genres()], ["Krimis & Thriller"])
        self.settings["discover_nonfiction_store"] = True
        gs = self.d.genres()
        self.assertEqual([g["name"] for g in gs], ["Krimis & Thriller", "Geschichte"])
        self.assertTrue(gs[1]["nonfiction"])


if __name__ == "__main__":
    unittest.main()
