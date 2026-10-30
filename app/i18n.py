"""German / English.

German is the source language: every user-facing text is written in German and
wrapped in ``t("…")``; ``static/i18n/en.json`` maps it to English. The same
catalog serves the web UI (``/i18n.js``). ``tests/test_i18n.py`` fails when a
text has no translation.

Placeholders: ``t("{n} Datei(en)", n=3)`` – only the given names are replaced,
so texts that talk about template placeholders like ``{author}`` stay intact.
"""
from __future__ import annotations

import json
import os
from typing import Dict

LANGUAGES = ("de", "en")
CATALOG_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "static", "i18n")

_lang = "de"
_catalogs: Dict[str, Dict[str, str]] = {}


def catalog(lang: str) -> Dict[str, str]:
    if lang == "de":
        return {}
    if lang not in _catalogs:
        try:
            with open(os.path.join(CATALOG_DIR, f"{lang}.json"), encoding="utf-8") as fh:
                _catalogs[lang] = json.load(fh)
        except (OSError, ValueError):
            _catalogs[lang] = {}
    return _catalogs[lang]


def set_language(lang: str) -> None:
    global _lang
    _lang = lang if lang in LANGUAGES else "de"


def language() -> str:
    return _lang


def t(text: str, **values) -> str:
    out = catalog(_lang).get(text, text) if _lang != "de" else text
    for k, v in values.items():
        out = out.replace("{" + k + "}", str(v))
    return out
