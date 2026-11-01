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
        self.asked = []

    def similar(self, asin, region, limit=20):
        return self.sims.get(asin, [])

    def genre_products(self, region, genre_id, sort="BestSellers", limit=50, page=1):
        self.asked.append((genre_id, sort, page))
        return self.lists.get(sort, []) if page == 1 else self.lists.get(f"{sort}:{page}", [])

    def genres(self, region):
        return [{"id": "1", "name": "Krimis & Thriller", "children": [{"id": "11", "name": "Krimis"}, {"id": "12", "name": "Cricket"}]},
                {"id": "2", "name": "Geschichte", "children": []},
                {"id": "3", "name": "Erotik", "children": [{"id": "31", "name": "Sexpraktiken"}]},
                {"id": "4", "name": "Literatur & Belletristik", "children": [{"id": "41", "name": "Erotik"}]}]

    def search(self, region, title="", author="", limit=5, **kw):
        return [h for h in self.hits if h["title"].lower().startswith(title.lower()[:5])]


PT = """<?xml version="1.0"?><rss version="2.0"><channel>
<item><category>BÜCHERBRIEF</category><title>BÜCHERBRIEF: Oktober</title>
<link>https://www.perlentaucher.de/buecherbrief/oktober.html</link><pubDate>Mon, 12 Oct 2026 08:53:37 +0000</pubDate>
<description><![CDATA[<a href="x"><img src="y"/></a><b>Dora Kapralovas</b> kleines Juwel "Die Mariborhypnose" und
<b>Jachym Topols</b> barocker Roman "Die Hölle gibt es nicht" vertreten den Schwerpunkt. Die <i>FAZ</i> denkt
über <b>Schuld und Sühne </b>nach. Und eine Biografie über <b>Rudolf Augstein</b>.]]></description></item>
<item><category>IM KINO</category><title>Film</title><link>https://www.perlentaucher.de/k.html</link>
<description><![CDATA[<b>Anna Regie</b> dreht "Ein Film"]]></description></item>
</channel></rss>"""

PT_LIST = """<html><body>
<article title="Radio Nacht. Roman - 7 Notizen" class="book teaser-block"><h2><a href="/buch/juri-andruchowytsch/radio-nacht.html">Juri Andruchowytsch: <span>Radio Nacht. Roman</span> </a></h2>
<div class="paragraph">Aus dem Ukrainischen.</div><a href="/buch/juri-andruchowytsch/radio-nacht.html" class="gold"><img src="x.svg"><span> 7 Notizen</span></a></article>
<article title="Ein Leben" class="book teaser-block"><h2><a href="/buch/anna-autorin/ein-leben.html">Anna Autorin: <span>Ein Leben</span> </a></h2></article>
</body></html>"""

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
        self.pt = PT.encode()
        self.pt_list = b"<html></html>"
        self.busy = set()
        self.d = discover.Discover(self.db, self.client, self.index, lambda: self.settings, fetch=self.fetch,
                                   pending=lambda: self.busy, prepare=False)

    def fetch(self, url):
        if "buchKSL" in url:
            if self.pt_list is None:
                raise OSError("down")
            return self.pt_list
        if "perlentaucher" in url:
            if self.pt is None:
                raise OSError("down")
            return self.pt
        return self.feed

    def items(self, section, genre=""):
        self.d.refresh(section, genre, wait=True)
        return self.d.get(section, genre)["items"]

    def test_review_titles(self):
        p = discover.parse_review_title
        self.assertEqual(p('Buchkritik: "Machtspiele" von Alice Zeniter'), {"title": "Machtspiele", "author": "Alice Zeniter"})
        self.assertEqual(p('Gaia Vince: "Das nomadische Jahrhundert"'), {"title": "Das nomadische Jahrhundert", "author": "Gaia Vince"})
        self.assertEqual(p('„Zwei Leben“ von A. Autorin und B. Berger')["author"], "A. Autorin")
        self.assertIsNone(p("Ein Gespräch über Bücher"))

    def test_perlentaucher_mentions(self):
        got = discover.parse_book_mentions(PT.split("CDATA[")[1].split("]]>")[0])
        self.assertEqual(got, [{"title": "Die Mariborhypnose", "author": "Dora Kapralovas"},
                               {"title": "Die Hölle gibt es nicht", "author": "Jachym Topols"}])
        self.assertEqual(discover._surnames("Dora Kapralovas"), ["kapralovas", "kapralova"])

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
                             "-ReleaseDate": [book("G2", "Zwei"), book("G3", "Drei", rating=2.0)],
                             "AvgRating": [book("G4", "Vier", rating=4.9, ratings=12),       # few ratings: fill-up, at the end
                                           book("G5", "Fünf", rating=4.5, ratings=2)],       # too few even for that
                             "BestSellers:2": [book("G6", "Sechs")]}
        self.assertEqual([i["asin"] for i in self.items("genre", "1")], ["G1", "G2", "G6", "G4"])
        self.assertTrue(self.d.get("genre", "1")["items"][-1]["loose"])
        self.assertEqual(self.d.get("genre", "")["items"], [])

    def test_genre_shows_nonfiction_when_picked(self):
        self.client.lists = {"BestSellers": [book("H1", "Rom", genres=["Geschichte"])]}
        self.assertEqual([i["asin"] for i in self.items("genre", "2")], ["H1"])
        self.client.sims = {"A1": [book("H1", "Rom", genres=["Geschichte"])]}
        self.index.entries = {"x": {"asin": "A1", "title": "Seed"}}
        self.assertEqual(self.items("foryou"), [])                 # not mixed into "Für dich"

    def test_big_genre_needs_no_second_page(self):
        self.client.lists = {"BestSellers": [book(f"B{n}", f"Titel {n}") for n in range(40)]}
        self.items("genre", "1")
        self.assertNotIn(("1", "BestSellers", 2), self.client.asked)

    def test_unknown_genre_is_not_looked_up(self):
        self.d.refresh("genre", "999", wait=True)
        self.assertEqual(self.client.asked, [])
        self.assertFalse(self.d.known_genre("999"))
        self.assertTrue(self.d.known_genre("11"))

    def test_critics_only_real_audiobooks(self):
        self.pt = None                                                    # one feed down: the other still counts
        self.client.hits = [book("R1", "Das stille Haus", rating=0, ratings=0),          # new: no ratings needed
                            book("R2", "Die Geschichte der Wolken", author="Bert Berger", genres=["Geschichte"]),
                            book("R3", "Das stille Haus", author="Jemand Anderes")]
        got = self.items("critics")
        self.assertEqual(sorted(i["asin"] for i in got), ["R1", "R2"])     # non-fiction allowed from the editorial source
        got = sorted(got, key=lambda i: i["asin"])
        self.assertEqual(got[0]["review"]["link"], "https://www.deutschlandfunkkultur.de/a-100.html")
        self.assertEqual(got[0]["review"]["source"], "Deutschlandfunk Kultur")

    def test_perlentaucher_and_memory(self):
        self.feed = b'<rss><channel></channel></rss>'
        self.client.hits = [book("P1", "Die Mariborhypnose", author="Dora Kapralová", rating=0, ratings=0),
                            book("P2", "Ein Film", author="Anna Regie")]
        got = self.items("critics")
        self.assertEqual([i["asin"] for i in got], ["P1"])                # genitive s and accent tolerated, no film
        self.assertEqual(got[0]["review"]["source"], "Perlentaucher")
        self.pt = b'<rss><channel></channel></rss>'                       # feed moved on: still remembered
        self.client.hits = []
        self.assertEqual([i["asin"] for i in self.items("critics")], ["P1"])
        self.busy = {"P1"}                                                # being downloaded: hidden
        self.assertEqual(self.d.get("critics")["items"], [])

    def test_perlentaucher_audiobook_list(self):
        self.assertEqual(discover.parse_pt_list(PT_LIST), [
            {"title": "Radio Nacht", "author": "Juri Andruchowytsch", "notes": 7,
             "link": "https://www.perlentaucher.de/buch/juri-andruchowytsch/radio-nacht.html"},
            {"title": "Ein Leben", "author": "Anna Autorin", "notes": 0,
             "link": "https://www.perlentaucher.de/buch/anna-autorin/ein-leben.html"}])
        self.feed = self.pt = b"<rss><channel></channel></rss>"
        self.pt_list = PT_LIST.encode()
        self.client.hits = [book("L1", "Radio Nacht", author="Juri Andruchowytsch", rating=0, ratings=0)]
        got = self.items("critics")
        self.assertEqual([i["asin"] for i in got], ["L1"])
        self.assertEqual(got[0]["review"]["notes"], 7)

    def test_feed_with_dtd_is_rejected(self):
        self.feed = b'<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><rss/>'
        self.pt = self.feed
        self.pt_list = None
        self.d.refresh("critics", wait=True)
        res = self.d.get("critics")
        self.assertEqual(res["items"], [])
        self.assertTrue(res["error"])

    def test_owned_later_disappears_from_cache(self):
        self.client.lists = {"BestSellers": [book("G1", "Eins")]}
        self.assertEqual(len(self.items("genre", "1")), 1)
        self.index.owned_titles = {"Eins"}
        self.assertEqual(self.d.get("genre", "1")["items"], [])

    def test_genres_like_the_shop(self):
        gs = self.d.genres()
        self.assertEqual([g["name"] for g in gs], ["Krimis & Thriller", "Geschichte", "Literatur & Belletristik"])
        self.assertTrue(gs[1]["nonfiction"])
        self.assertEqual([c["name"] for c in gs[0]["children"]], ["Krimis", "Cricket"])
        self.assertEqual(gs[2]["children"], [])                         # no adult subcategory either
        self.items("genre", "12")                                       # nothing German there at all …
        self.assertEqual([c["name"] for c in self.d.genres()[0]["children"]], ["Krimis"])   # … so it is left out

    def test_critics_sources_take_turns(self):
        reviews = [{"source": "DLF", "date": f"Fri, {d} Oct 2026", "n": d} for d in (23, 30, 16)] + \
                  [{"source": "PT", "date": "", "seen": 1, "n": 1}]
        self.assertEqual([r["n"] for r in discover._mix_sources(reviews)], [30, 1, 23, 16])

    def test_review_title_with_subline(self):
        p = discover.parse_review_title
        self.assertEqual(p("Manuel Bittorf: „Goldener Retriever“ – Weltschmerz und Übermut (Buchkritik)"),
                         {"title": "Goldener Retriever", "author": "Manuel Bittorf"})
        self.assertEqual(p('Pernille Radeid, Jannicke Hansen (Ill.): "Wenn der Nebel flüstert"')["author"], "Pernille Radeid")
        self.assertIsNone(p("„Das glückliche Schicksal“: Ach, diese wunderbaren Lücken!"))


if __name__ == "__main__":
    unittest.main()
