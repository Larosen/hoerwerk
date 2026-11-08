"""Every German UI text has an English translation (and the catalog has no leftovers).

Collected like the app uses them:
* Rust and TypeScript: string literals in ``t("…")`` (Rust test modules are left out)
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
CATALOG = os.path.join(ROOT, "static", "i18n", "en.json")
CALL = re.compile(r'(?<![\w.])t\(\s*"((?:[^"\\]|\\.)*)"')
BAD_CALL = re.compile(r'(?<![\w.])t\(\s*(f"|`|\'|[A-Za-z_])')


def _unescape(s: str) -> str:
    return json.loads(f'"{s}"')


def _sources() -> list:
    """(path, text) of the code that shows texts: the Rust backend without its test modules, the TypeScript UI."""
    out = []
    for path in sorted(glob.glob(os.path.join(ROOT, "rust", "hoerwerk", "src", "**", "*.rs"), recursive=True)):
        if path.endswith("text.rs"):
            continue
        text = open(path, encoding="utf-8").read()
        out.append((path, re.split(r"\n#\[cfg\(test\)\]\nmod ", text)[0]))
    for path in sorted(glob.glob(os.path.join(ROOT, "frontend", "src", "*.ts"))):
        out.append((path, open(path, encoding="utf-8").read()))
    return out


def code_keys() -> dict:
    keys = {}
    for path, text in _sources():
        for m in CALL.finditer(text):
            keys.setdefault(_unescape(m.group(1)), os.path.relpath(path, ROOT))
    return keys


def bad_calls() -> list:
    out = []
    for path, text in _sources():
        for n, line in enumerate(text.splitlines(), 1):
            m = BAD_CALL.search(line)
            if m and "fn t(" not in line and "function t(" not in line:
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
    for page in ("index.html", "login.html"):
        p.feed(open(os.path.join(ROOT, "static", page), encoding="utf-8").read())
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

    def test_only_literal_texts_are_translated(self):
        self.assertEqual(bad_calls(), [], "t() braucht einen festen Text (Werte über {name}-Platzhalter)")


if __name__ == "__main__":
    if "--missing" in sys.argv:
        cat = catalog()
        print(json.dumps({k: "" for k in sorted(all_keys() - set(cat))}, ensure_ascii=False, indent=1))
    else:
        unittest.main()
