import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import audible  # noqa: E402

PRODUCT = {
    "asin": "B0TEST0002",
    "title": "Das Fräulein",
    "subtitle": "Die Serapionsbrüder 1",
    "authors": [{"asin": "B0TEST0004", "name": "E.T.A. Hoffmann"}, {"name": "Hans Muster — Translator"}],
    "narrators": [{"name": "Erika Muster"}],
    "category_ladders": [
        {"ladder": [{"id": "1", "name": "Science Fiction & Fantasy"}, {"id": "2", "name": "Fantasy"}], "root": "Genres"},
        {"ladder": [{"id": "3", "name": "Literatur & Belletristik"}, {"id": "4", "name": "Belletristik"}], "root": "Genres"},
    ],
    "format_type": "unabridged",
    "language": "german",
    "product_images": {"1024": "https://m.media-amazon.com/images/I/91TFseitgwL._SL1024_.jpg",
                       "500": "https://m.media-amazon.com/images/I/51AoExrmwrL._SL500_.jpg"},
    "publisher_name": "der Hörverlag",
    "publisher_summary": "Ein Kriminalfall im Paris Ludwigs XIV. <p>Erika Muster liest.</p><p><b>(Laufzeit: 22h 54)</b></p>",
    "release_date": "2011-06-03",
    "runtime_length_min": 1373,
    "series": [{"asin": "B00FYVHU6W", "sequence": "Band 1", "title": "Serapionsbrüder"}],
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
        self.assertEqual(b["authors"], ["E.T.A. Hoffmann"])
        self.assertEqual(b["translators"], ["Hans Muster"])
        self.assertEqual(b["narrators"], ["Erika Muster"])
        self.assertEqual(b["series"], "Serapionsbrüder")
        self.assertEqual(b["series_position"], "1")
        self.assertEqual(b["genres"], ["Science Fiction & Fantasy", "Literatur & Belletristik"])
        self.assertEqual(b["tags"], ["Fantasy", "Belletristik"])
        self.assertEqual(b["language"], "German")
        self.assertEqual(b["cover_url"], "https://m.media-amazon.com/images/I/91TFseitgwL.jpg")
        self.assertEqual(b["url"], "https://www.audible.de/pd/B0TEST0002")
        self.assertEqual(b["description"], "Ein Kriminalfall im Paris Ludwigs XIV.\n\nErika Muster liest.\n\n(Laufzeit: 22h 54)")

    def test_series_position(self):
        self.assertEqual(audible.series_position("Book 8.5"), "8.5")
        self.assertEqual(audible.series_position("1-3"), "1-3")
        self.assertEqual(audible.series_position("Folge 12,5"), "12.5")
        self.assertIsNone(audible.series_position(""))
        self.assertIsNone(audible.series_position("Sonderband"))


class ContentQueryTest(unittest.TestCase):
    """Shorter searches from a folder's own words – for any name, not one case."""

    def test_cases(self):
        q = audible.content_queries
        cases = {
            ("Twilight Saga 5 Biss zum ersten Sonnenstrahl HörbucHHamburg Meyer, Stephenie", "Meyer, Stephenie"):
                ["Biss zum ersten Sonnenstrahl Meyer", "zum ersten Sonnenstrahl Meyer", "Twilight Saga Meyer"],
            ("J K Rowling Harry Potter 3 The Prisoner of Azkaban Unabridged EN", "J.K. Rowling"):
                ["The Prisoner of Azkaban Rowling", "Harry Potter Rowling"],
            ("Rowling Harry Potter 01 German", "Rowling"): ["Harry Potter Rowling"],
            ("Anna Autorin Maerchen aus dem Wald Hoerbuch", "Anna Autorin"): ["Märchen aus dem Wald Autorin"],
            ("Stephenie Meyer Twilight Saga 5 Biss zum ersten Sonnenstrahl 4CD DE 2010", "Stephenie Meyer"):
                ["Biss zum ersten Sonnenstrahl Meyer"],
        }
        for (query, author), expected in cases.items():
            got = q(query, author)
            for want in expected:
                self.assertIn(want, got, query)
            self.assertLessEqual(len(got), 8)
            for g in got:                        # never the label words, years or "4CD"
                self.assertNotRegex(g, r"HörbucHHamburg|\b(DE|EN|German|Unabridged|Hoerbuch|4CD|2010)\b")

    def test_no_volume_no_split_and_no_false_umlauts(self):
        got = audible.content_queries("Michael Ende Die unendliche Geschichte", "Michael Ende")
        self.assertEqual(got[0], "Die unendliche Geschichte Ende")
        self.assertFalse(any("Gesch" in g and "ä" in g for g in got))
        self.assertNotIn("Morgengraün", " ".join(audible.content_queries("Biss zum Morgengrauen 1 Kapitel eins", "")))
        self.assertEqual(audible.surname_of("Meyer, Stephenie"), "Meyer")
        self.assertEqual(audible.surname_of("J.K. Rowling"), "Rowling")


class QueryTest(unittest.TestCase):
    def test_clean_query(self):
        self.assertEqual(
            audible.clean_query("E.T.A. Hoffmann - Die Serapionsbrüder 01 - Das Fräulein (Ungekürzt) [2011] 64kbps"),
            "E.T.A. Hoffmann Die Serapionsbrüder 01 Das Fräulein")
        self.assertEqual(audible.clean_query("Theodor_Storm_-_Der_Schimmelreiter_CD1"), "Theodor Storm Der Schimmelreiter")
        self.assertEqual(audible.clean_query("Die Säulen der Erde (Hörbuch) [MP3]"), "Die Säulen der Erde")
        self.assertEqual(audible.clean_query("Max und Moritz (Band 1)"), "Max und Moritz Band 1")
        self.assertEqual(audible.clean_query("Buch [B0TEST0002]"), "Buch")

    def test_find_asin(self):
        self.assertEqual(audible.find_asin("Hoffmann - Fräulein [B0TEST0002]"), "B0TEST0002")
        self.assertEqual(audible.find_asin("", "3899034677"), "3899034677")
        self.assertIsNone(audible.find_asin("Telefon 0123456789 Buch"))
        self.assertIsNone(audible.find_asin("Kein Code hier"))


class MatchTest(unittest.TestCase):
    def test_duration_decides_between_editions(self):
        cands = [book("Der Schimmelreiter", ["Theodor Storm"], 553, "A1"),
                 book("Der Schimmelreiter", ["Theodor Storm", "Erika Muster"], 399, "A2")]
        ranked = audible.rank(cands, {"query": "Theodor Storm Der Schimmelreiter", "duration": 399 * 60 + 30})
        self.assertEqual(ranked[0]["asin"], "A2")
        self.assertEqual(audible.best_match(ranked, 0.8)["asin"], "A2")

    def test_runtime_far_off_needs_review(self):
        ranked = audible.rank([book("Das Fräulein", ["E.T.A. Hoffmann"], 1373)],
                              {"query": "Hoffmann Das Fräulein", "duration": 600})
        self.assertGreaterEqual(ranked[0]["match"]["score"], 0.75)
        self.assertIsNone(audible.best_match(ranked, 0.75))

    def test_ambiguous_without_duration(self):
        cands = [book("Der Schimmelreiter", ["Theodor Storm"], 553, "A1"),
                 book("Der Schimmelreiter", ["Theodor Storm", "Erika Muster"], 399, "A2")]
        ranked = audible.rank(cands, {"query": "Theodor Storm Der Schimmelreiter"})
        self.assertIsNone(audible.best_match(ranked, 0.8))

    def test_same_book_tie_prefers_unabridged(self):
        cands = [book("West-östlicher Divan", ["Johann Wolfgang Goethe"], 434, "AB", format_type="abridged"),
                 book("West-östlicher Divan", ["Johann Wolfgang Goethe"], 1220, "UN", format_type="unabridged")]
        ranked = audible.rank(cands, {"query": "Johann Wolfgang Goethe West östlicher Divan"})
        self.assertEqual(audible.best_match(ranked, 0.8)["asin"], "UN")

    def test_wrong_title_scores_low(self):
        ranked = audible.rank([book("Immensee", ["Theodor Storm"], 334)],
                              {"query": "Theodor Storm Der Schimmelreiter"})
        self.assertLess(ranked[0]["match"]["score"], 0.8)

    def test_lookup_prefers_asin(self):
        b = book("Das Fräulein", ["E.T.A. Hoffmann"], 1373, "B0TEST0002")
        c = FakeClient(products={"B0TEST0002": b})
        ranked = audible.lookup(c, {"asin": "B0TEST0002", "query": "egal"}, "de")
        self.assertEqual(ranked[0]["asin"], "B0TEST0002")
        self.assertEqual(c.calls, [("product", "B0TEST0002")])

    def test_lookup_title_author_then_keywords(self):
        c = FakeClient(results=[book("Das Fräulein", ["E.T.A. Hoffmann"], 1373, "X1")])
        audible.lookup(c, {"title": "Das Fräulein", "author": "Hoffmann", "query": "Das Fräulein Hoffmann"}, "de")
        self.assertEqual(c.calls[0], ("search", "", "Das Fräulein", "Hoffmann"))
        self.assertEqual(c.calls[1][1], "Das Fräulein Hoffmann")

    def test_html_to_text(self):
        self.assertEqual(audible.html_to_text("<p>Eins &amp; zwei</p><p></p><p>Drei<br>Vier</p>"),
                         "Eins & zwei\n\nDrei\nVier")


if __name__ == "__main__":
    unittest.main()
