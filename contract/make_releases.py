"""Builds contract/golden/releases.json: how Hörwerk judges releases (releases.py, packs.py, usenet.py) – the Python
answers for many names and books. Other implementations (rust/hoerwerk) must decide the same.
tests/test_golden.py keeps the file current."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))

BOOKS = [
    {"title": "Der Schimmelreiter", "authors": ["Theodor Storm"], "runtime_min": 300, "language": "german"},
    {"title": "Twilight", "authors": ["Stephenie Meyer"], "series": "Twilight-Saga", "series_position": "1",
     "runtime_min": 700, "language": "german"},
    {"title": "Todesmal", "authors": ["Andreas Gruber"], "series": "Ein Sneijder-und-Nemez-Krimi",
     "series_position": "5", "runtime_min": 720},
    {"title": "Eragon - Der Auftrag des Ältesten (Eragon 2)", "authors": ["Christopher Paolini"],
     "series": "Eragon", "series_position": "2,0", "runtime_min": 1500, "language": "German"},
    {"title": "The Hobbit", "authors": ["J. R. R. Tolkien"], "runtime_min": 660, "language": "english",
     "format_type": "dramatized"},
    {"title": "Faust: Band 1", "authors": ["Johann Wolfgang von Goethe"], "narrators": ["A", "B", "C", "D"]},
    {"title": "Die drei ??? und der Super-Papagei", "authors": ["Robert Arthur"], "series": "Die drei ???",
     "series_position": "1", "runtime_min": 75, "genres": ["Hörspiel"]},
    {"title": "Ng", "authors": ["Celeste Ng"]},
]
RELEASES = [
    {"title": "Theodor.Storm-Der.Schimmelreiter.German.Hoerbuch.M4B-GROUP", "size": 300 * 60 * 128000 // 8, "grabs": 40},
    {"title": "Theodor Storm - Der Schimmelreiter (ungekürzt) 64kbps MP3", "size": 140 * 1024 * 1024},
    {"title": "Storm - Schimmelreiter - gekürzt - German", "size": 50 * 1024 * 1024},
    {"title": "Theodor.Storm.Der.Schimmelreiter.English.Audiobook.2019", "size": 400 * 1024 * 1024},
    {"title": "Theodor Storm - Der Schimmelreiter FLAC", "size": 90 * 1024 * 1024},
    {"title": "Theodor Storm - Der Schimmelreiter Dolby Atmos 2021", "size": 900 * 1024 * 1024, "grabs": 500},
    {"title": "Stephenie.Meyer-Twilight-Saga.5.German.Hoerbuch", "size": 600 * 1024 * 1024},
    {"title": "Stephenie Meyer - Twilight 1 - Biss zum Morgengrauen - DE", "size": 700 * 1024 * 1024},
    {"title": "Stephenie Meyer - Twilight Band 01-04 German", "size": 3000 * 1024 * 1024},
    {"title": "Andreas Gruber - Sneijder und Nemez 01 - 07 (2024)", "size": 4000 * 1024 * 1024},
    {"title": "Andreas Gruber - Todesmal - Band 5 - ungekuerzt", "size": 500 * 1024 * 1024, "grabs": 3},
    {"title": "Andreas Gruber - Todesmal - Band 4", "size": 500 * 1024 * 1024},
    {"title": "Christopher Paolini - Eragon 2 - Der Auftrag des Aeltesten - Hoerbuch", "size": 1500 * 1024 * 1024},
    {"title": "Paolini.Eragon.Der.Auftrag.des.Altesten.Teil.2.German.128k", "size": 1400 * 1024 * 1024},
    {"title": "J.R.R.Tolkien.The.Hobbit.BBC.Full.Cast.Radio.Drama.ENG", "size": 300 * 1024 * 1024},
    {"title": "Tolkien - Der Hobbit - Hörspiel - German", "size": 300 * 1024 * 1024},
    {"title": "Goethe - Faust 1 - Hörspiel", "size": 0},
    {"title": "Die drei ??? Folge 1 und der Super-Papagei - Robert Arthur", "size": 60 * 1024 * 1024},
    {"title": "Celeste Ng - Kleine Feuer überall", "size": 100},
]
NAMES = [
    "Sneijder und Nemez 01 - 07", "Andreas Gruber - Sneijder und Nemez 1-7", "Band 1-3", "Folgen 10–12",
    "Harry Potter 1 bis 7 (Hörbuch)", "7CD", "128kbps", "Album 1-4CD", "Paket 3 - 2", "Reihe 1 - 99",
    "Die.Tribute.von.Panem.Teil.1-3.German", "Max Muster - Krimi-Reihe Bände 1-5",
]
FOLDERS = [
    ("03 - Todesmal", "Andreas Gruber - Sneijder und Nemez 01 - 07"), ("Band 3 - Titel", None),
    ("Sneijder und Nemez 3 - Todesfrist", "Andreas Gruber - Sneijder und Nemez 01 - 07"),
    ("Teil 9", "Band 1-3"), ("CD 1", None), ("Hörbuch 128kbps", None), ("Folge.04.Die.Spur", "Folgen 1-6"),
]
INFO = ["Buch.German.M4B", "Book ENG MP3 64kbps", "Hoerbuch-DE-gekuerzt", "Hörspiel UNGEKÜRZT FLAC",
        "Book abridged english 96 kbit", "DEcember Song", "Bonde - AAC", "Dramatisierung opus"]
QUERIES = ["Fräulein Smilla", "Jo Nesbø", "Céline Dion", "Straße", "plain ascii", "Ærø Œuvre Łódź"]
TITLES = ["Der Schimmelreiter", "Die Wolke (Ein Jugendroman 3)", "Mord im Orientexpress: Band 4",
          "Tannöd - Teil 2", "[Band 1]", "Titel (A) [B]", "  viel   Raum  "]
SERIES = ["Ein Bodenstein-Kirchhoff-Krimi", "Die Tribute von Panem", "The Expanse Series", "Harry Potter",
          "Die drei ???", "Eine Sneijder-und-Nemez-Reihe", "Twilight-Saga"]
MISMATCH = [("Stephenie Meyer - Twilight 3", 1, 0), ("Twilight English", 1, 0), ("Twilight", 1, 3000),
            ("Twilight", 1, 42000), ("Twilight Band 1-4", 1, 0), ("Schimmelreiter", 0, 18000)]

FEED = b"""<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/">
<channel><title>Indexer</title>
<item><title>Theodor.Storm-Der.Schimmelreiter.German.M4B</title><guid>https://idx.example/details/1</guid>
 <link>https://idx.example/getnzb/1</link><comments>https://idx.example/details/1#comments</comments>
 <pubDate>Mon, 02 Mar 2026 14:05:09 +0100</pubDate><category>Audio &gt; Audiobook</category>
 <enclosure url="https://idx.example/getnzb/1.nzb" length="123456" type="application/x-nzb"/>
 <newznab:attr name="category" value="3000"/><newznab:attr name="category" value="3030"/>
 <newznab:attr name="size" value="987654321"/><newznab:attr name="grabs" value="17"/></item>
<item><title>Ohne Link</title><link>javascript:alert(1)</link></item>
<item><title> Zweites Buch 64kbps </title><link>http://idx.example/getnzb/2</link>
 <pubDate>kein Datum</pubDate><newznab:attr name="grabs" value="viele"/></item>
</channel></rss>"""
CAPS = """<caps><server title="Test-Indexer"/><categories>
<category id="3000" name="Audio"><subcat id="3010" name="MP3"/><subcat id="3030" name="Audiobook"/></category>
<category id="7000" name="Other"><subcat id="7020" name="Hoerbuecher"/><subcat id="7030" name="Ebook"/></category>
<category id="8000" name="Musik"><subcat id="8010" name="Pop"/></category>
<category id="x" name="Hörspiel"/><category id="5070" name="Hörspiele"/></categories></caps>""".encode()


def build() -> dict:
    import i18n
    import packs
    import releases
    from usenet import audiobook_categories, parse_results, query_variants, release_info
    i18n.set_language("de")
    cases = []
    for b in BOOKS:
        for r in RELEASES:
            rel = {**r, **release_info(r["title"])}
            cases.append([BOOKS.index(b), r, releases.evaluate(rel, b),
                          releases.evaluate(rel, b, prefer_language="en", unabridged=False, check_size=False)])
    return {
        "description": "How Hörwerk judges releases (Python answers; see make_releases.py).",
        "books": BOOKS,
        "evaluate": cases,
        "blocked": releases.evaluate({"title": RELEASES[0]["title"]}, BOOKS[0], blocked=[RELEASES[0]["title"]]),
        "rank": [r["title"] for r in releases.rank([{**r, **release_info(r["title"])} for r in RELEASES], BOOKS[0])],
        "search_stages": [releases.search_stages(b) for b in BOOKS],
        "release_volumes": [[r["title"], i, sorted(releases.release_volumes(r["title"], b))]
                            for i, b in enumerate(BOOKS) for r in RELEASES],
        "pack_info": [[n, packs.pack_info(n)] for n in NAMES],
        "volume_of": [[f, p, packs.volume_of(f, packs.pack_info(p) if p else None)] for f, p in FOLDERS],
        "title_of": [[f, p, packs.title_of(f, packs.pack_info(p) if p else None)] for f, p in FOLDERS],
        "release_info": [[n, release_info(n)] for n in INFO + [r["title"] for r in RELEASES]],
        "query_variants": [[q, query_variants(q)] for q in QUERIES],
        "clean_title": [[x, releases.clean_title(x)] for x in TITLES],
        "series_core": [[x, releases.series_core(x)] for x in SERIES],
        "guess_language": [[r["title"], releases.guess_language(r["title"])] for r in RELEASES],
        "mismatch": [[n, i, d, releases.mismatch(n, BOOKS[i], d)] for n, i, d in MISMATCH],
        "feed": FEED.decode(), "parsed": parse_results(FEED, "Test"),
        "caps": CAPS.decode(), "caps_categories": audiobook_categories(CAPS),
        "dtd_refused": _refused(b'<?xml version="1.0"?><!-- x --><!DOCTYPE r [<!ENTITY a "b">]><r>&a;</r>'),
    }


def _refused(xml: bytes) -> str:
    from usenet import UsenetError, parse_results
    try:
        parse_results(xml, "Test")
    except UsenetError as e:
        return str(e)
    return ""


if __name__ == "__main__":
    with open(os.path.join(HERE, "golden", "releases.json"), "w", encoding="utf-8") as fh:
        json.dump(build(), fh, ensure_ascii=False, indent=1)
        fh.write("\n")
