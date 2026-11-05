"""Builds contract/golden/discover.json: how Hörwerk reads book reviews and best-of list PDFs (discover.py,
pdftext.py) – the Python answers. Other implementations (rust/hoerwerk) must give the same. Also writes
rust/hoerwerk/src/trusted_authors.txt (the list of established non-fiction authors, one per line).
tests/test_golden.py keeps both current."""
import base64
import json
import os
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))
sys.path.insert(0, os.path.join(HERE, "..", "tests"))

TRUSTED_TXT = os.path.join(HERE, "..", "rust", "hoerwerk", "src", "trusted_authors.txt")

REVIEW_TITLES = [
    'Buchkritik: "Machtspiele" von Alice Zeniter', 'Gaia Vince: "Das nomadische Jahrhundert"',
    '„Zwei Leben“ von A. Autorin und B. Berger', "Ein Gespräch über Bücher",
    "Manuel Bittorf: „Goldener Retriever“ – Weltschmerz und Übermut (Buchkritik)",
    'Pernille Radeid, Jannicke Hansen (Ill.): "Wenn der Nebel flüstert"',
    "„Das glückliche Schicksal“: Ach, diese wunderbaren Lücken!", "Buchkritik - »Der Fluss« v. Eva Ufer",
    'BUCHKRITIK – "Nacht" von Ann & Bo', "Autor: „Titel“", '"Nur Titel"', "", "Anna Autorin: „Leer“ — Unterzeile",
    'Kim Ko (Hg.): "Alles" von', 'Lea Lux: "Ein Titel" von Tom Tal, Uwe Ulm',
]
MENTIONS = [
    '<a href="x"><img src="y"/></a><b>Dora Kapralovas</b> kleines Juwel "Die Mariborhypnose" und\n'
    '<b>Jachym Topols</b> barocker Roman "Die Hölle gibt es nicht" vertreten den Schwerpunkt. Die <i>FAZ</i> denkt\n'
    'über <b>Schuld und Sühne </b>nach. Und eine Biografie über <b>Rudolf Augstein</b>.',
    '<b>Anna von Regie</b> dreht „Ein Film“. <b>Bert Berger</b> schreibt. Dann „Ohne Autor“.',
    '<b>Ca&szlig;ius Kl&ouml;n</b> mit &quot;Das &amp; Mehr&quot; und <b>x y</b> "Klein" sowie <b>Eins Zwei Drei Vier Fünf Sechs</b> "Zu viel"',
    "<b>Lu Li</b> „Eins“ und „Zwei“! <B>Groß Klein</B> «Spitz»",
]
PT_LIST = """<html><body>
<article title="Radio Nacht. Roman - 7 Notizen" class="book teaser-block"><h2><a href="/buch/juri-andruchowytsch/radio-nacht.html">Juri Andruchowytsch: <span>Radio Nacht. Roman</span> </a></h2>
<div class="paragraph">Aus dem Ukrainischen.</div><a href="/buch/juri-andruchowytsch/radio-nacht.html" class="gold"><img src="x.svg"><span> 7 Notizen</span></a></article>
<article title="Ein Leben" class="book teaser-block"><h2><a href="/buch/anna-autorin/ein-leben.html">Anna Autorin: <span>Ein Leben &amp; Sterben. teil zwei. Band 3</span> </a></h2></article>
<article><h2> <a href="/buch/x/y.html"> Ohne: <span>Notiz</span></a></h2><span>123 Notizen</span></article>
</body></html>"""
MONTH = (
    '<script>{&quot;content&quot;:&quot;&lt;a class=\\&quot;download-url-tracking\\&quot; href=\\&quot;x.pdf\\&quot;&gt;'
    'Kerstin Preiwu&szlig;: \\&quot;Wir alle\\&quot; (PDF)&lt;/a&gt; (B&uuml;chermarkt, 02.11.2026)&quot;},'
    '{&quot;content&quot;:&quot;&lt;a href=\\&quot;y.pdf\\&quot;&gt;Sachbuchbestenliste - Die besten Sachb&uuml;cher (PDF)&lt;/a&gt; (Studio 9, 29.10.2026)&quot;},'
    '{&quot;content&quot;:&quot;&lt;a href=\\&quot;z.pdf\\&quot;&gt;Carlos Barrag&aacute;n: &bdquo;Hi Honey. Nigerias Romance-Scam-Industrie&ldquo; (PDF)&lt;/a&gt; (Andruck, 31.10.2026)&quot;},'
    '{&quot;content&quot;:&quot;&lt;a href=\\&quot;w.pdf\\&quot;&gt;Ute Typo: \\&quot;Falsch\\&quot; (PDF)&lt;/a&gt; (Lesart 31.11.2026)&quot;},'
    '{&quot;teaserHeadline&quot;:&quot;Kerstin Preiwu&szlig;: \\&quot;Wir alle\\&quot;&quot;},'
    '{&quot;teaserHeadline&quot;:&quot;Daron Acemoglu: \\&quot;Das liberale Versprechen - Ein Essay\\&quot;&quot;},'
    '{&quot;teaserHeadline&quot;:&quot;Rezensionen&quot;}</script>')
LISTS_PAGE = (
    MONTH + '<p><a title="Datei herunterladen" class="download-url-tracking" '
    'href="https://bilder.deutschlandfunk.de/87/07/krimibestenliste-oktober2026-100.pdf" data-tracking="{}">'
    'Krimibestenliste - Die besten Krimis im Oktober 2026 (PDF)</a> (Lesart, 02.10.2026)</p>'
    '<a href="https://bilder.deutschlandfunk.de/1/sachbuch.pdf">Sachbuchbestenliste: Die besten Sachbücher (PDF)</a> (29.02.2026)'
    '<a href="https://bilder.deutschlandfunk.de/87/07/krimibestenliste-oktober2026-100.pdf">Bestenliste nochmal (PDF)</a> (01.10.2026)'
    '<a href="https://evil.example/x-bestenliste.pdf">Krimibestenliste (PDF)</a> (L, 02.10.2026)')
RSS = """<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel>
<item><title>Buchkritik: "Das stille Haus" von Anna Autorin</title><link>https://www.deutschlandfunkkultur.de/a-100.html</link>
<pubDate>Fri, 30 Oct 2026 10:00:00 +0100</pubDate></item>
<item><title>Bert Berger: „Die Geschichte der Wolken“</title><link> http://unsicher.example/b </link></item>
<item><title>Ein Gespräch über Bücher</title><link>https://www.deutschlandfunkkultur.de/c-100.html</link></item>
<item><title><![CDATA[Cara C: „Daten & Fakten“]]></title><link>https://x.example/&amp;y</link><pubDate>Sat, 1 Nov 2026</pubDate></item>
</channel></rss>"""
PT = """<?xml version="1.0"?><rss version="2.0"><channel>
<item><category>BÜCHERBRIEF</category><title>BÜCHERBRIEF: Oktober</title>
<link>https://www.perlentaucher.de/buecherbrief/oktober.html</link><pubDate>Mon, 12 Oct 2026 08:53:37 +0000</pubDate>
<description><![CDATA[<a href="x"><img src="y"/></a><b>Dora Kapralovas</b> kleines Juwel "Die Mariborhypnose" und
<b>Jachym Topols</b> barocker Roman "Die Hölle gibt es nicht" vertreten den Schwerpunkt.]]></description></item>
<item><category>IM KINO</category><title>Film</title><link>https://www.perlentaucher.de/k.html</link>
<description><![CDATA[<b>Anna Regie</b> dreht "Ein Film"]]></description></item>
<item><category> taschenbücherbrief extra</category><title>T</title>
<description>&lt;b&gt;Tim Taschen&lt;/b&gt; mit "Klein und fein"</description></item>
</channel></rss>"""


def pdf_cases():
    import test_pdftext as tp
    cases = {
        "list": tp.make_pdf(tp.LIST), "list_plain": tp.make_pdf(tp.LIST, compress=False),
        "text_only": tp.make_pdf("BT /F1 9 Tf 1 0 0 1 28 600 Tm (Nur Text) Tj ET"),
        "not_pdf": b"<html>not a pdf</html>",
        # leading, T*, TD, octal and escapes, a hex string, array spacing, a name where a number belongs
        "operators": tp.make_pdf("\n".join([
            "BT /F1 9 Tf 1 0 0 1 100 500 Tm (3 \\(4\\)) Tj ET",
            "BT /F1 12 Tf 100 482 Td 14 TL (Der \\344rger) Tj T* [(Zwei) -300 (te)] TJ ET",
            "BT /F1 9 Tf 180 440 TD (Ann Au\\\ntor) Tj 0 -12 Td <4265 6e20 4265> Tj /x -5 Td (Hrsg. von X) Tj ET",
            "BT /F1 9 Tf 1 0 0 1 300 500 Tm (12 \\(neu\\)) Tj ET",
            "BT /F2 12 Tf 1 0 0 1 300 482 Tm <0001000200030004> Tj [<0005> /oops] TJ ET",
            "BT /F1 9 Tf 1 0 0 1 340 470 Tm (Ein Mann) ' ET",
        ])),
    }
    # object stream with the page and font inside; ToUnicode with bfrange; two content streams
    cmap = b"begincmap 1 beginbfrange <41> <5A> <0061> endbfrange 1 beginbfchar <21> <00DF> endbfchar endcmap"
    c1 = b"BT /F1 9 Tf 1 0 0 1 50 400 Tm (7 \\(7\\)) Tj ET"
    c2 = b"BT /F1 12 Tf 1 0 0 1 50 382 Tm (GRO!) Tj ET BT /F1 9 Tf 1 0 0 1 95 360 Tm (EVA ERNST) Tj ET"
    inner = [b"<</Type/Page/Resources 9 0 R/Contents[4 0 R 5 0 R 77 0 R]>>", b"<</Font 10 0 R>>", b"<</F1 11 0 R>>",
             b"<</Type/Font/ToUnicode 6 0 R>>"]
    offs, body = [], b""
    for o in inner:
        offs.append(len(body))
        body += o + b" "
    head = b" ".join(b"%d %d" % (n, off) for n, off in zip((3, 9, 10, 11), offs)) + b" "
    stm = zlib.compress(head + body)
    raw = b"%PDF-1.5\n"
    for n, o in [(1, b"<</Type/Catalog>>"),
                 (2, b"<</Type/ObjStm/N 4/First %d/Filter[/FlateDecode]/Length %d>>stream\r\n" % (len(head), len(stm)) + stm + b"\nendstream"),
                 (4, b"<</Length 1>>stream\n" + c1 + b"\nendstream"),
                 (5, b"<</Filter/FlateDecode>>stream\n" + zlib.compress(c2) + b"\nendstream"),
                 (6, b"<</Filter /FlateDecode>>stream\n" + zlib.compress(cmap) + b"\nendstream"),
                 (8, b"<</Subtype/Image/Filter/DCTDecode>>stream\nxx\nendstream"),
                 (12, b"<</Filter/LZWDecode>>stream\nxx\nendstream")]:
        raw += b"%d 0 obj\n" % n + o + b"\nendobj\n"
    cases["objstm"] = raw + b"%%EOF"
    return cases


def build() -> dict:
    import discover
    import pdftext
    from trusted_authors import TRUSTED_AUTHORS

    def best(raw):
        try:
            return pdftext.best_list(raw)
        except ValueError as e:
            return {"error": str(e)}
    pdfs = pdf_cases()
    best_entry = {"name": "Die besten Krimis", "url": "https://bilder.deutschlandfunk.de/k.pdf", "date": "Fri, 02 Oct 2026"}
    reviews = [{"source": "DLF", "date": f"Fri, {d} Oct 2026", "n": d} for d in (23, 30, 16)] + \
              [{"source": "PT", "date": "", "seen": 1, "n": 1}, {"source": "PT", "date": "Kaputt", "seen": 5, "n": 5},
               {"date": "Sun, 01 Nov 2026", "found": 1893456000, "n": 7}, {"source": "DLF", "date": "Mon, 30 Feb 2026", "n": 9}]
    return {
        "review_titles": [[x, discover.parse_review_title(x)] for x in REVIEW_TITLES],
        "mentions": [[x, discover.parse_book_mentions(x)] for x in MENTIONS],
        "pt_list": [PT_LIST, discover.parse_pt_list(PT_LIST)],
        "dlf_month": [MONTH, discover.parse_dlf_month(MONTH)],
        "dlf_lists": [LISTS_PAGE, discover.dlf_best_lists(LISTS_PAGE)],
        "pdfs": {k: [base64.b64encode(v).decode(), best(v)] for k, v in pdfs.items()},
        "best_list_reviews": discover.best_list_reviews(pdfs["list"], best_entry),
        "best_entry": best_entry,
        "rss": [[src, kind, raw, discover.Discover._reviews_from(None, src, raw.encode(), kind)]
                for src, kind, raw in (("Deutschlandfunk Kultur", "rss", RSS), ("Perlentaucher", "rss", PT),
                                       ("Perlentaucher", "list", PT_LIST))],
        "slugs": [[n, discover.pt_slug(n)] for n in ("Rüdiger Maas", "Cathy O'Neil", "Josef M. Gaßner", "Özden Terli",
                                                     "  ", "Ana-María Ñúñez", "Jean d’Ormesson")],
        "has_books": [[p, discover.pt_has_books(p)] for p in ("<p>4 Bücher</p>", "<p>1 Buch</p>", "1 Buecher",
                                                              "<h1>Buchautor-Übersicht</h1> 12 Bücher", "Bücher", "x12 Bücherei")],
        "surnames": [[n, discover._surnames(n)] for n in ("Dora Kapralovas", "Ann Bus", "Hans Rosling", "", "Eva Faulerová.")],
        "main_titles": [[n, discover._main_title(n)] for n in ("Endzeitfaschismus - Und der Kampf", "Hi Honey. Nigerias Scam",
                                                               "U.S.A. heute", " - nur", "Eins – Zwei")],
        "mix": [reviews, [r["n"] for r in discover._mix_sources(reviews)]],
        "trusted_authors": list(TRUSTED_AUTHORS),
    }


def trusted_txt() -> str:
    from trusted_authors import TRUSTED_AUTHORS
    return "".join(a + "\n" for a in TRUSTED_AUTHORS)


if __name__ == "__main__":
    with open(os.path.join(HERE, "golden", "discover.json"), "w", encoding="utf-8") as fh:
        json.dump(build(), fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    with open(TRUSTED_TXT, "w", encoding="utf-8") as fh:
        fh.write(trusted_txt())
