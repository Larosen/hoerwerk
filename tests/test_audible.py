import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import audible  # noqa: E402

PRODUCT = {
    "asin": "B0054JTDOG",
    "title": "Die Gefährten",
    "subtitle": "Der Herr der Ringe 1",
    "authors": [{"asin": "B000ARC6KA", "name": "J.R.R. Tolkien"}, {"name": "Wolfgang Krege — Translator"}],
    "narrators": [{"name": "Achim Höppner"}],
    "category_ladders": [
        {"ladder": [{"id": "1", "name": "Science Fiction & Fantasy"}, {"id": "2", "name": "Fantasy"}], "root": "Genres"},
        {"ladder": [{"id": "3", "name": "Literatur & Belletristik"}, {"id": "4", "name": "Belletristik"}], "root": "Genres"},
    ],
    "format_type": "unabridged",
    "language": "german",
    "product_images": {"1024": "https://m.media-amazon.com/images/I/91TFseitgwL._SL1024_.jpg",
                       "500": "https://m.media-amazon.com/images/I/51AoExrmwrL._SL500_.jpg"},
    "publisher_name": "der Hörverlag",
    "publisher_summary": "Neun Gefährten machen sich auf. <p>Achim Höppner liest.</p><p><b>(Laufzeit: 22h 54)</b></p>",
    "release_date": "2011-06-03",
    "runtime_length_min": 1373,
    "series": [{"asin": "B00FYVHU6W", "sequence": "Band 1", "title": "Herr der Ringe"}],
}


def book(title, authors, runtime, asin="B000000000", **kw):
    return {"asin": asin, "title": title, "authors": authors, "runtime_min": runtime, "narrators": [], **kw}


class FakeClient:
    def __init__(self, results=None, products=None):
        self.results = results or []
        self.products = products or {}
        self.calls = []

    def search(self, region, keywords="", title="", author="", limit=10):
        self.calls.append(("search", keywords, title, author))
        return list(self.results)

    def product(self, asin, region):
        self.calls.append(("product", asin))
        if asin not in self.products:
            raise audible.AudibleError("nope")
        return self.products[asin]


class NormalizeTest(unittest.TestCase):
    def test_product(self):
        b = audible.normalize_product(PRODUCT, "de")
        self.assertEqual(b["authors"], ["J.R.R. Tolkien"])
        self.assertEqual(b["translators"], ["Wolfgang Krege"])
        self.assertEqual(b["narrators"], ["Achim Höppner"])
        self.assertEqual(b["series"], "Herr der Ringe")
        self.assertEqual(b["series_position"], "1")
        self.assertEqual(b["genres"], ["Science Fiction & Fantasy", "Literatur & Belletristik"])
        self.assertEqual(b["tags"], ["Fantasy", "Belletristik"])
        self.assertEqual(b["language"], "German")
        self.assertEqual(b["cover_url"], "https://m.media-amazon.com/images/I/91TFseitgwL.jpg")
        self.assertEqual(b["url"], "https://www.audible.de/pd/B0054JTDOG")
        self.assertEqual(b["description"], "Neun Gefährten machen sich auf.\n\nAchim Höppner liest.\n\n(Laufzeit: 22h 54)")

    def test_series_position(self):
        self.assertEqual(audible.series_position("Book 8.5"), "8.5")
        self.assertEqual(audible.series_position("1-3"), "1-3")
        self.assertEqual(audible.series_position("Folge 12,5"), "12.5")
        self.assertIsNone(audible.series_position(""))
        self.assertIsNone(audible.series_position("Sonderband"))


class QueryTest(unittest.TestCase):
    def test_clean_query(self):
        self.assertEqual(
            audible.clean_query("J.R.R. Tolkien - Der Herr der Ringe 01 - Die Gefährten (Ungekürzt) [2011] 64kbps"),
            "J.R.R. Tolkien Der Herr der Ringe 01 Die Gefährten")
        self.assertEqual(audible.clean_query("Sebastian_Fitzek_-_Der_Insasse_CD1"), "Sebastian Fitzek Der Insasse")
        self.assertEqual(audible.clean_query("Die Säulen der Erde (Hörbuch) [MP3]"), "Die Säulen der Erde")
        self.assertEqual(audible.clean_query("Harry Potter (Band 1)"), "Harry Potter Band 1")
        self.assertEqual(audible.clean_query("Buch [B0054JTDOG]"), "Buch")

    def test_find_asin(self):
        self.assertEqual(audible.find_asin("Tolkien - Gefährten [B0054JTDOG]"), "B0054JTDOG")
        self.assertEqual(audible.find_asin("", "3899034677"), "3899034677")
        self.assertIsNone(audible.find_asin("Telefon 0123456789 Buch"))
        self.assertIsNone(audible.find_asin("Kein Code hier"))


class MatchTest(unittest.TestCase):
    def test_duration_decides_between_editions(self):
        cands = [book("Der Insasse", ["Sebastian Fitzek"], 553, "A1"),
                 book("Der Insasse", ["Sebastian Fitzek", "Josef Ulbig"], 399, "A2")]
        ranked = audible.rank(cands, {"query": "Sebastian Fitzek Der Insasse", "duration": 399 * 60 + 30})
        self.assertEqual(ranked[0]["asin"], "A2")
        self.assertEqual(audible.best_match(ranked, 0.8)["asin"], "A2")

    def test_runtime_far_off_needs_review(self):
        ranked = audible.rank([book("Die Gefährten", ["J.R.R. Tolkien"], 1373)],
                              {"query": "Tolkien Die Gefährten", "duration": 600})
        self.assertGreaterEqual(ranked[0]["match"]["score"], 0.75)
        self.assertIsNone(audible.best_match(ranked, 0.75))

    def test_ambiguous_without_duration(self):
        cands = [book("Der Insasse", ["Sebastian Fitzek"], 553, "A1"),
                 book("Der Insasse", ["Sebastian Fitzek", "Josef Ulbig"], 399, "A2")]
        ranked = audible.rank(cands, {"query": "Sebastian Fitzek Der Insasse"})
        self.assertIsNone(audible.best_match(ranked, 0.8))

    def test_same_book_tie_prefers_unabridged(self):
        cands = [book("Das Jesus-Video", ["Andreas Eschbach"], 434, "AB", format_type="abridged"),
                 book("Das Jesus-Video", ["Andreas Eschbach"], 1220, "UN", format_type="unabridged")]
        ranked = audible.rank(cands, {"query": "Andreas Eschbach Das Jesus Video"})
        self.assertEqual(audible.best_match(ranked, 0.8)["asin"], "UN")

    def test_wrong_title_scores_low(self):
        ranked = audible.rank([book("Der Seelenbrecher", ["Sebastian Fitzek"], 334)],
                              {"query": "Sebastian Fitzek Der Insasse"})
        self.assertLess(ranked[0]["match"]["score"], 0.8)

    def test_lookup_prefers_asin(self):
        b = book("Die Gefährten", ["J.R.R. Tolkien"], 1373, "B0054JTDOG")
        c = FakeClient(products={"B0054JTDOG": b})
        ranked = audible.lookup(c, {"asin": "B0054JTDOG", "query": "egal"}, "de")
        self.assertEqual(ranked[0]["asin"], "B0054JTDOG")
        self.assertEqual(c.calls, [("product", "B0054JTDOG")])

    def test_lookup_title_author_then_keywords(self):
        c = FakeClient(results=[book("Die Gefährten", ["J.R.R. Tolkien"], 1373, "X1")])
        audible.lookup(c, {"title": "Die Gefährten", "author": "Tolkien", "query": "Die Gefährten Tolkien"}, "de")
        self.assertEqual(c.calls[0], ("search", "", "Die Gefährten", "Tolkien"))
        self.assertEqual(c.calls[1][1], "Die Gefährten Tolkien")

    def test_html_to_text(self):
        self.assertEqual(audible.html_to_text("<p>Eins &amp; zwei</p><p></p><p>Drei<br>Vier</p>"),
                         "Eins & zwei\n\nDrei\nVier")


if __name__ == "__main__":
    unittest.main()
