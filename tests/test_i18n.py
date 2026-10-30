"""Every German UI text has an English translation (and the catalog has no leftovers).

Collected like the app uses them:
* Python and TypeScript: string literals in ``t("…")``
* index.html: text nodes (outside code/script/style) and placeholder/title/aria-label

``python3 tests/test_i18n.py --missing`` prints the missing texts as JSON (for translating).
"""
import glob
import json
import os
import re
import sys
import unittest
from html.parser import HTMLParser

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
CATALOG = os.path.join(ROOT, "app", "static", "i18n", "en.json")
CALL = re.compile(r'(?<![\w.])t\(\s*"((?:[^"\\]|\\.)*)"')
BAD_CALL = re.compile(r'(?<![\w.])t\(\s*(f"|`|\'|[A-Za-z_])')


def _unescape(s: str) -> str:
    return json.loads(f'"{s}"')


def code_keys() -> dict:
    keys = {}
    files = glob.glob(os.path.join(ROOT, "app", "**", "*.py"), recursive=True) + \
        glob.glob(os.path.join(ROOT, "frontend", "src", "*.ts"))
    for path in files:
        if path.endswith("i18n.py"):
            continue
        text = open(path, encoding="utf-8").read()
        for m in CALL.finditer(text):
            keys.setdefault(_unescape(m.group(1)), os.path.relpath(path, ROOT))
    return keys


def bad_calls() -> list:
    out = []
    for path in glob.glob(os.path.join(ROOT, "app", "**", "*.py"), recursive=True) + \
            glob.glob(os.path.join(ROOT, "frontend", "src", "*.ts")):
        if path.endswith("i18n.py"):
            continue
        for n, line in enumerate(open(path, encoding="utf-8"), 1):
            m = BAD_CALL.search(line)
            if m and "def t(" not in line and "function t(" not in line:
                out.append(f"{os.path.relpath(path, ROOT)}:{n}: {line.strip()}")
    return out


class _Html(HTMLParser):
    SKIP = ("code", "script", "style")

    def __init__(self) -> None:
        super().__init__()
        self.depth = 0
        self.keys = set()

    def handle_starttag(self, tag, attrs):
        if tag in self.SKIP:
            self.depth += 1
        for k, v in attrs:
            if k in ("placeholder", "title", "aria-label") and v:
                self._add(v)

    def handle_endtag(self, tag):
        if tag in self.SKIP:
            self.depth -= 1

    def handle_data(self, data):
        if not self.depth:
            self._add(data)

    def _add(self, text: str) -> None:
        key = " ".join(text.split())
        if re.search(r"[A-Za-zÄÖÜäöüß]{2}", key):
            self.keys.add(key)


def html_keys() -> set:
    p = _Html()
    p.feed(open(os.path.join(ROOT, "app", "static", "index.html"), encoding="utf-8").read())
    return p.keys


def all_keys() -> set:
    return set(code_keys()) | html_keys()


def catalog() -> dict:
    with open(CATALOG, encoding="utf-8") as fh:
        return json.load(fh)


class I18nTest(unittest.TestCase):
    def test_every_text_is_translated(self):
        missing = sorted(all_keys() - set(catalog()))
        self.assertEqual(missing, [], f"{len(missing)} Texte ohne Übersetzung – python3 tests/test_i18n.py --missing")

    def test_no_leftover_translations(self):
        unused = sorted(set(catalog()) - all_keys())
        self.assertEqual(unused, [], "Übersetzungen, die nirgends mehr verwendet werden")

    def test_placeholders_survive(self):
        for de, en in catalog().items():
            self.assertEqual(sorted(re.findall(r"\{\w+\}", de)), sorted(re.findall(r"\{\w+\}", en)), de)
            self.assertTrue(en.strip(), de)

    def test_switching_language(self):
        sys.path.insert(0, os.path.join(ROOT, "app"))
        import i18n
        try:
            i18n.set_language("en")
            self.assertEqual(i18n.t("Kapitel {n}", n=3), "Chapter 3")
            self.assertEqual(i18n.t("Text ohne Übersetzung"), "Text ohne Übersetzung")
            i18n.set_language("xx")
            self.assertEqual(i18n.language(), "de")
            self.assertEqual(i18n.t("Kapitel {n}", n=3), "Kapitel 3")
        finally:
            i18n.set_language("de")

    def test_only_literal_texts_are_translated(self):
        self.assertEqual(bad_calls(), [], "t() braucht einen festen Text (Werte über {name}-Platzhalter)")


if __name__ == "__main__":
    if "--missing" in sys.argv:
        cat = catalog()
        print(json.dumps({k: "" for k in sorted(all_keys() - set(cat))}, ensure_ascii=False, indent=1))
    else:
        unittest.main()
