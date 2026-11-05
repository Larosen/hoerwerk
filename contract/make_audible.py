"""Builds contract/golden/audible.json: how Hörwerk reads and matches shop data (audible.py, providers.py) – the
Python answers. Other implementations (rust/hoerwerk) must give the same. tests/test_golden.py keeps it current."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))

PRODUCTS = [
    {"asin": "B0STORM001", "title": " Der Schimmelreiter ", "subtitle": "", "language": "german", "runtime_length_min": 302,
     "authors": [{"name": "Theodor Storm"}, {"name": "Max Muster - Übersetzer"}, {"name": "Eva Vorwort - foreword"}],
     "narrators": [{"name": "Erika Beispiel"}, {"name": "Erika Beispiel"}], "publisher_name": "Hörverlag",
     "release_date": "2019-03-01", "format_type": "unabridged",
     "series": [{"title": "Novellen", "asin": "B0SER", "sequence": "Buch 3,5"}, {"title": "Andere", "sequence": "2-4"}],
     "category_ladders": [{"ladder": [{"name": "Klassiker"}, {"name": "Novellen"}]}, {"ladder": [{"name": "Klassiker"}, {"name": "Deutsch"}]}],
     "product_images": {"500": "https://m.media-amazon.com/images/I/51abc._SL500_.jpg"},
     "publisher_summary": "<p>Ein <b>Deich</b>graf &amp; sein Schimmel.</p><p>Zweiter&nbsp;Absatz<br>neue Zeile</p><ul><li>eins</li><li>zwei</li></ul>",
     "rating": {"overall_distribution": {"display_average_rating": "4.6", "num_ratings": 1234}}},
    {"asin": "B0STORM002", "title": "Der Schimmelreiter", "authors": [{"name": "Theodor Storm"}], "runtime_length_min": 120,
     "format_type": "abridged", "language": "GERMAN"},
    {"asin": "B0OTHER003", "title": "Der Schimmelreiter und andere Novellen", "authors": [{"name": "Theodor Storm"}],
     "runtime_length_min": 900, "format_type": "unabridged", "issue_date": "2001-01-01T00:00"},
    {"asin": "B0TWILI004", "title": "Biss zum Morgengrauen", "authors": [{"name": "Stephenie Meyer"}],
     "narrators": [{"name": "Ulrike Grote"}], "runtime_length_min": 690, "format_type": "unabridged",
     "series": [{"title": "Twilight-Saga", "sequence": "1"}]},
]
HINTS = [
    {"query": "Theodor Storm Der Schimmelreiter", "title": "Der Schimmelreiter", "author": "Theodor Storm", "duration": 18100},
    {"query": "Schimmelreiter", "title": "", "author": "", "duration": 0},
    {"query": "Storm Schimmelreiter", "title": "Schimmelreiter", "author": "Storm, Theodor", "narrator": "Erika Beispiel", "duration": 7300},
    {"query": "Stephenie Meyer Twilight 1 Bis(s) zum Morgengrauen", "title": "", "author": "Stephenie Meyer", "duration": 41400},
]
QUERIES = ["Stephenie.Meyer.-.Twilight.1.-.Bis(s).zum.Morgengrauen", "Der Schimmelreiter (Ungekürzt) [MP3 128kbps] 2019",
           "Autor - Titel (CD 1) {B0ABCDEFGH}", "Freund(in) im Sturm_Teil 2 – Hörbuch", "Mord: Im Orient | Express (1934)",
           "J.K. Rowling - Harry Potter und der Stein der Weisen", "(Hörspiel) Die drei ??? Folge 1"]
ASINS = [["B0ABCDEFGH"], ["xx b0abcdefgh"], ["3453123456"], ["ASIN 3453123456 im Text"], ["Buch 1234567890 lang"], [""]]
CONTENT = [("Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl HörbucHHamburg DE MP3", "Stephenie Meyer"),
           ("Maerchen aus aller Welt Teil Gruene Feuer", ""), ("Twilight 4 Biss zum Ende der Nacht", "Meyer, Stephenie"),
           ("Ein ganz langer Titel mit vielen Woertern 2019 2CDs", "Ng"), ("Kurz", "")]
RATIOS = [("schimmelreiter", "der schimmelreiter"), ("abc", ""), ("theodor storm", "storm theodor"),
          ("a" * 250 + "b" * 10, "a" * 240 + "c" * 30), ("e " * 150 + "x", "e " * 149 + "y")]


def build() -> dict:
    import audible
    import i18n
    import providers
    from difflib import SequenceMatcher
    i18n.set_language("de")
    books = [audible.normalize_product(p, "de") for p in PRODUCTS]
    ranked = [audible.rank(books, h) for h in HINTS]

    class Fake:
        def __init__(self):
            self.calls = []

        def _get(self, url):
            self.calls.append(url)
            if "/catalog/products/B0STORM001" in url:
                return {"product": PRODUCTS[0]}
            if "keywords=Kurz" in url or "keywords=Schimmel" in url:
                return {"products": PRODUCTS[:3]}
            return {"products": []}
    looks = []
    for h in [{"asin": "B0STORM001", "query": "x"}, {"query": "Schimmelreiter", "title": "", "author": ""},
              {"query": "Nichts da", "title": "T", "author": "A", "alt_query": "Kurz"}]:
        fake = Fake()
        client = audible.AudibleClient(get=fake._get)
        looks.append([h, [b["asin"] for b in audible.lookup(client, h, "de")], fake.calls])
    return {
        "description": "How Hörwerk reads and matches shop data (Python answers; see make_audible.py).",
        "products": PRODUCTS, "normalized": books, "hints": HINTS, "ranked": ranked,
        "best": [[(audible.best_match(r, th) or {}).get("asin") for th in (0.5, 0.8, 0.95)] for r in ranked],
        "auto": [(providers.auto_match(r, 0.5) or {}).get("asin") for r in ranked],
        "clean_query": [[q, audible.clean_query(q)] for q in QUERIES],
        "find_asin": [[a, audible.find_asin(*a)] for a in ASINS],
        "content_queries": [[q, a, audible.content_queries(q, a)] for q, a in CONTENT],
        "ratio": [[a, b, SequenceMatcher(None, a, b).ratio()] for a, b in RATIOS],
        "series_position": [[x, audible.series_position(x)] for x in ["3", "Buch 3,5", "1-3", "Band 2 - 4", "", None, "x"]],
        "lookup": looks,
        "audnexus": providers.from_audnexus({"asin": "B0N", "title": "T ", "authors": [{"name": "A"}],
                                              "genres": [{"type": "genre", "name": "G"}, {"type": "tag", "name": "X"}],
                                              "seriesPrimary": {"name": "S", "position": "2"}, "language": "english",
                                              "releaseDate": "2020-01-02T00:00:00Z", "summary": "<p>Hi</p>"}, "us"),
        "itunes": providers.from_itunes({"collectionId": 7, "collectionName": "Titel (Ungekürzt)", "artistName": "A & B und C",
                                         "artworkUrl100": "https://x/100x100bb.jpg", "collectionViewUrl": "https://a/b?c=d"}),
        "openlibrary": providers.from_openlibrary({"key": "/works/OL1W", "title": "T", "author_name": ["A", "B"],
                                                   "first_publish_year": 1888, "cover_i": 5, "isbn": ["123"]}),
    }


if __name__ == "__main__":
    with open(os.path.join(HERE, "golden", "audible.json"), "w", encoding="utf-8") as fh:
        json.dump(build(), fh, ensure_ascii=False, indent=1)
        fh.write("\n")
