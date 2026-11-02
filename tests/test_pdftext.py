"""Text from the monthly best-of list PDFs: columns, places, title and author, two font encodings, limits."""
import os
import sys
import unittest
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import pdftext  # noqa: E402

# Identity-H font: code 0x0001.. -> characters of this string (as with embedded subset fonts)
GLYPHS = "Ein unglückcherTod"
CMAP = ("/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <0000> <FFFF> "
        "endcodespacerange %d beginbfchar %s endbfchar endcmap end end" % (
            len(GLYPHS), " ".join("<%04X> <%04X>" % (i + 1, ord(c)) for i, c in enumerate(GLYPHS))))


def cid(text):
    return "<" + "".join("%04X" % (GLYPHS.index(c) + 1) for c in text) + ">"


def winansi(text):
    return "(" + text.encode("cp1252").decode("latin-1").replace("(", "\\(").replace(")", "\\)") + ")"


def column(x, y, rank, title_lines, author_lines, font_title="F1"):
    ops = [f"BT /F1 9 Tf 1 0 0 1 {x} {y} Tm {winansi(rank)} Tj ET"]
    for i, line in enumerate(title_lines):
        text = f"{cid(line)} Tj" if font_title == "F2" else f"[{winansi(line[:3])} -10 {winansi(line[3:])}] TJ"
        ops.append(f"BT /{font_title} 12 Tf 1 0 0 1 {x} {y - 18 - 13 * i} Tm {text} ET")
    ops.append(f"BT /F1 9 Tf 1 0 0 1 {x + 78} {y - 30 - 13 * len(title_lines)} Tm 0 0 Td")
    for line in author_lines:
        ops.append(f"{winansi(line)} Tj 0 -10 Td")
    ops.append("ET")
    return "\n".join(ops)


def make_pdf(content, compress=True):
    stream = content.encode("latin-1")
    cmap = CMAP.encode()
    objs = [
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Count 1/Kids[3 0 R]>>",
        b"<</Type/Page/Parent 2 0 R/Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R>>",
        (b"<</Length %d/Filter/FlateDecode>>stream\n" % len(zlib.compress(stream)) + zlib.compress(stream) + b"\nendstream")
        if compress else (b"<</Length %d>>stream\n" % len(stream) + stream + b"\nendstream"),
        b"<</Type/Font/Subtype/TrueType/BaseFont/Arial/Encoding/WinAnsiEncoding>>",
        b"<</Type/Font/Subtype/Type0/BaseFont/X/Encoding/Identity-H/ToUnicode 7 0 R>>",
        b"<</Length %d/Filter/FlateDecode>>stream\n" % len(zlib.compress(cmap)) + zlib.compress(cmap) + b"\nendstream",
    ]
    out = b"%PDF-1.7\n"
    for i, o in enumerate(objs, 1):
        out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    return out + b"trailer<</Root 1 0 R>>\n%%EOF"


LIST = "\n".join([
    "BT /F1 18 Tf 1 0 0 1 28 676 Tm (Die besten Krimis im Oktober 2026) Tj ET",
    column(28, 622, "1 (1)", ["Ein unglücklicher Tod"], ["Peter Grainger", "Aus dem Englischen von", "Eva Regul", "284 Seiten"], "F2"),
    column(208, 622, "2 (-)", ["Endzeitfaschismus. Und der", "Kampf um unsere Welt"], ["Naomi Klein und", "Astra Taylor", "Übersetzt von U. Becker"]),
    column(387, 622, "5 (–)", ["Die Schakalin"], ["Mike Nicol", "511 Seiten"]),
    column(28, 380, "5 (10)", ["Glanz und Elend der", "Konservativen"], ["Roger de Weck", "18 Euro"]),
    "BT /F1 9 Tf 1 0 0 1 28 300 Tm (Kapstadt, Südafrika. Captain Zara Dewane) Tj ET",
])


class BestListTest(unittest.TestCase):
    def test_columns_places_titles_authors(self):
        self.assertEqual(pdftext.best_list(make_pdf(LIST)), [
            {"rank": 1, "title": "Ein unglücklicher Tod", "author": "Peter Grainger"},          # Identity-H + ToUnicode
            {"rank": 2, "title": "Endzeitfaschismus. Und der Kampf um unsere Welt", "author": "Naomi Klein und Astra Taylor"},
            {"rank": 5, "title": "Die Schakalin", "author": "Mike Nicol"},
            {"rank": 5, "title": "Glanz und Elend der Konservativen", "author": "Roger de Weck"}])   # shared place, 2nd row
        self.assertEqual(len(pdftext.best_list(make_pdf(LIST, compress=False))), 4)

    def test_not_a_list_or_not_a_pdf(self):
        self.assertEqual(pdftext.best_list(make_pdf("BT /F1 9 Tf 1 0 0 1 28 600 Tm (Nur Text) Tj ET")), [])
        with self.assertRaises(ValueError):
            pdftext.best_list(b"<html>not a pdf</html>")
        with self.assertRaises(ValueError):
            pdftext.best_list(b"%PDF" + b"0" * pdftext.MAX_PDF)

    def test_broken_and_huge_streams_are_harmless(self):
        bomb = zlib.compress(b"0" * (pdftext.MAX_STREAM * 3))          # unpacks far beyond the limit
        raw = (b"%PDF-1.7\n1 0 obj\n<</Type/Page/Contents 2 0 R>>\nendobj\n2 0 obj\n<</Filter/FlateDecode>>stream\n"
               + bomb + b"\nendstream\nendobj\n3 0 obj\n<</Filter/FlateDecode>>stream\nnot zlib\nendstream\nendobj\n")
        self.assertEqual(pdftext.best_list(raw), [])
        objs = pdftext._objects(raw)
        self.assertLessEqual(len(objs[2][1]), pdftext.MAX_STREAM)
        self.assertEqual(objs[3][1], b"")


if __name__ == "__main__":
    unittest.main()
