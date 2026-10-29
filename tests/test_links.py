import json
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import audible  # noqa: E402
import links  # noqa: E402

BOOKBEAT = {
    "id": 11925, "title": "Eragon", "subtitle": "Das Vermächtnis der Drachenreiter",
    "author": "Christopher Paolini", "narrator": "Andreas Fröhlich, Jane Doe", "translator": "Joannis Stefanidis",
    "summary": "<p>Als Eragon …</p>", "cover": "https://prod-bb-images.akamaized.net/book-covers/x.jpg?w=400",
    "language": "German", "published": "2004-01-01T00:00:00+01:00", "audiobooklength": 72960,
    "genres": [{"name": "Fantasy"}, {"name": "Jugendbuch"}],
    "editions": [{"format": "eBook", "isbn": "111", "publisher": "cbj", "published": "2010-01-01T00:00:00"},
                 {"format": "Audiobook", "isbn": "9783867421324", "publisher": "Der Hörverlag",
                  "published": "2004-09-01T00:00:00+02:00"}],
    "series": {"id": 1, "name": "Eragon", "partnumber": 1, "displaypartnumber": "1"},
}

THALIA_HTML = """<html><head>
<meta property="og:title" content="Der Insasse - Sebastian Fitzek | Thalia">
<script type="application/ld+json">{"@context":"https://schema.org","@graph":[
 {"@type":"BreadcrumbList","itemListElement":[]},
 {"@type":"Book","name":"Der Insasse","author":[{"@type":"Person","name":"Sebastian Fitzek"}],
  "image":["https://images.thalia.media/cover.jpg"],"description":"Zwei entsetzliche &amp; Kindermorde",
  "workExample":[{"@type":"Book","bookFormat":"https://schema.org/AudiobookFormat","isbn":"9783839816448",
   "datePublished":"2018-10-24","publisher":{"@type":"Organization","name":"Audible Studios"},
   "readBy":[{"@type":"Person","name":"Simon Jäger"}],"duration":"PT9H13M"}]}
]}</script></head><body></body></html>"""


class LinkTest(unittest.TestCase):
    def test_shops(self):
        self.assertEqual(links.shop_of("https://www.audible.de/pd/B0054JTDOG"), "audible")
        self.assertEqual(links.shop_of("https://www.audible.co.uk/pd/x/B0054JTDOG"), "audible")
        self.assertEqual(links.shop_of("https://www.bookbeat.com/de/book/eragon-11925"), "bookbeat")
        self.assertEqual(links.shop_of("https://www.bookbeat.de/buch/x"), "bookbeat")
        self.assertEqual(links.shop_of("https://www.thalia.de/shop/home/artikeldetails/A1053789155"), "thalia")
        self.assertIsNone(links.shop_of("https://audible.de.evil.com/pd/B0054JTDOG"))
        self.assertIsNone(links.shop_of("https://example.com/"))
        self.assertTrue(links.is_link(" www.bookbeat.com/de/book/1"))
        self.assertFalse(links.is_link("Fitzek Insasse"))

    def test_unsupported_link(self):
        with self.assertRaises(links.LinkError):
            links.fetch("https://example.com/book/1", None)

    def test_audible_link(self):
        client = mock.Mock()
        client.product.return_value = {"asin": "B0054JTDOG"}
        links.fetch("https://www.audible.co.uk/pd/The-Fellowship-Audiobook/B0054JTDOG?ref=a", client)
        client.product.assert_called_once_with("B0054JTDOG", "uk")

    def test_bookbeat_id(self):
        self.assertEqual(links.bookbeat_id("https://www.bookbeat.com/de/book/eragon-das-vermachtnis-11925"), "11925")
        self.assertEqual(links.bookbeat_id("https://www.bookbeat.com/de/book/11925?x=1"), "11925")
        self.assertIsNone(links.bookbeat_id("https://www.bookbeat.com/de/author/x"))

    def test_bookbeat_normalize(self):
        b = links.normalize_bookbeat(BOOKBEAT, "https://www.bookbeat.com/de/book/11925")
        self.assertEqual(b["title"], "Eragon")
        self.assertEqual(b["narrators"], ["Andreas Fröhlich", "Jane Doe"])
        self.assertEqual(b["series"], "Eragon")
        self.assertEqual(b["series_position"], "1")
        self.assertEqual(b["publisher"], "Der Hörverlag")
        self.assertEqual(b["release_date"], "2004-09-01")
        self.assertEqual(b["isbn"], "9783867421324")
        self.assertEqual(b["runtime_min"], 1216)
        self.assertEqual(b["genres"], ["Fantasy", "Jugendbuch"])
        self.assertEqual(b["description"], "Als Eragon …")
        self.assertTrue(b["cover_url"].endswith("w=1400"))
        self.assertEqual(b["id"], "bookbeat:11925")
        self.assertEqual(b["source"], "bookbeat")

    def test_bookbeat_ebook_only(self):
        ebook = {**BOOKBEAT, "audiobooklength": 0, "editions": [BOOKBEAT["editions"][0]]}
        with mock.patch.object(audible, "http_get", return_value=json.dumps(ebook).encode()):
            with self.assertRaises(links.LinkError):
                links.fetch("https://www.bookbeat.com/de/book/11925", None)
        with mock.patch.object(audible, "http_get", return_value=json.dumps(BOOKBEAT).encode()):
            self.assertEqual(links.fetch("https://www.bookbeat.com/de/book/11925", None)["title"], "Eragon")

    def test_product_page(self):
        b = links.parse_product_page(THALIA_HTML, "https://www.thalia.de/x", "thalia")
        self.assertEqual(b["title"], "Der Insasse")
        self.assertEqual(b["authors"], ["Sebastian Fitzek"])
        self.assertEqual(b["narrators"], ["Simon Jäger"])
        self.assertEqual(b["publisher"], "Audible Studios")
        self.assertEqual(b["isbn"], "9783839816448")
        self.assertEqual(b["runtime_min"], 553)
        self.assertEqual(b["cover_url"], "https://images.thalia.media/cover.jpg")
        self.assertEqual(b["description"], "Zwei entsetzliche & Kindermorde")
        self.assertEqual(b["id"], "thalia:9783839816448")

    def test_product_page_series_from_title_and_og_fallback(self):
        html = '<meta property="og:title" content="Die drei ??? Toteninsel, Band 3: Der Fluch der Gräber - Thalia">'
        b = links.parse_product_page(html, "https://www.thalia.de/y", "thalia")
        self.assertEqual(b["series"], "Die drei ??? Toteninsel")
        self.assertEqual(b["series_position"], "3")
        self.assertEqual(b["title"], "Der Fluch der Gräber")

    def test_blocked_shop(self):
        with mock.patch.object(audible, "http_get", side_effect=audible.AudibleError("HTTP 403 für x")):
            with self.assertRaises(links.LinkError) as cm:
                links.fetch("https://www.thalia.de/shop/home/artikeldetails/A1", None)
        self.assertIn("blockiert", str(cm.exception))

    def test_iso_minutes(self):
        self.assertEqual(links._iso_minutes("PT9H13M"), 553)
        self.assertEqual(links._iso_minutes("PT45M30S"), 46)
        self.assertEqual(links._iso_minutes("quatsch"), 0)


if __name__ == "__main__":
    unittest.main()
