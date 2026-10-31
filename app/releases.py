"""Is this release the requested book – and how good is it? (Like Radarr's decision engine.)

``evaluate`` gives every indexer result a verdict with a reason a person can read:
  ✓ accepted with a score (format, language, popularity, a plausible size), or
  ✗ rejected: other author, other title, abridged, other language, size doesn't fit the runtime,
    a pack without this volume, or blocked after a failed download.
"""
from __future__ import annotations

from typing import Iterable, List, Optional

import packs
from i18n import t
from usenet import match_words

STOP = {"der", "die", "das", "und", "ein", "eine", "the", "and", "von", "mit", "roman", "band", "teil", "folge"}
FORMAT_SCORE = {"M4B": 30, "M4A": 20, "MP3": 15, "FLAC": 10}
MIN_KBPS = 20          # below that a "full" audiobook is too small – usually abridged or broken
MAX_KBPS = 1600        # above that it is not this book (or a video)


def surname(name: str) -> str:
    words = match_words(name or "")
    return words[-1] if words else ""


def title_words(title: str) -> List[str]:
    return [w for w in match_words(title or "") if w not in STOP and len(w) > 1]


def fits(release: str, book: dict) -> bool:
    """Author's surname and (nearly) all words of the title in the release name."""
    return evaluate({"title": release}, book, check_size=False)["ok"]


def _volumes(book: dict) -> Optional[int]:
    try:
        return int(float(str(book.get("series_position") or "").replace(",", ".")))
    except ValueError:
        return None


def evaluate(release: dict, book: dict, prefer_language: str = "de", unabridged: bool = True,
             blocked: Iterable[str] = (), check_size: bool = True) -> dict:
    """{ok, score, reason (why rejected), notes (why it scores), kbps}."""
    name = release.get("title") or ""
    words = set(match_words(name))
    notes: List[str] = []

    def no(reason: str) -> dict:
        return {"ok": False, "score": 0.0, "reason": reason, "notes": notes, "kbps": kbps}

    kbps = None
    runtime = int(book.get("runtime_min") or 0)
    size = int(release.get("size") or 0)
    pack = packs.pack_info(name)
    if runtime and size and not pack:
        kbps = round(size * 8 / 1000 / (runtime * 60))
    if name in set(blocked):
        return no(t("gesperrt (Download oder Import ging schief)"))
    author = surname((book.get("authors") or [""])[0])
    if author and author not in words:
        return no(t("Autor „{name}“ fehlt im Namen", name=(book.get("authors") or [""])[0]))
    title = title_words(book.get("title") or "")
    if not title:
        return no(t("Buch ohne Titel"))
    share = sum(1 for w in title if w in words) / len(title)
    if share < 0.8:
        return no(t("Titel passt nicht ({n} % der Wörter)", n=round(share * 100)))
    if pack:
        vol = _volumes(book)
        if vol is None or vol not in pack["volumes"]:
            return no(t("Paket ohne diesen Band ({first}–{last})", first=pack["volumes"][0], last=pack["volumes"][-1]))
        notes.append(t("Paket – nur Band {n} wird übernommen", n=vol))
    if unabridged and release.get("abridged"):
        return no(t("gekürzte Fassung"))
    lang = release.get("language")
    if prefer_language and lang and lang != prefer_language:
        return no(t("andere Sprache ({lang})", lang=lang))
    if check_size and kbps is not None:
        if kbps < MIN_KBPS:
            return no(t("zu klein für {h} h Laufzeit ({kbps} kbit/s)", h=round(runtime / 60, 1), kbps=kbps))
        if kbps > MAX_KBPS:
            return no(t("zu groß für {h} h Laufzeit ({kbps} kbit/s)", h=round(runtime / 60, 1), kbps=kbps))
    score = float(FORMAT_SCORE.get(release.get("format") or "", 5))
    if release.get("format"):
        notes.append(release["format"])
    if lang and lang == prefer_language:
        score += 10
        notes.append(t("Sprache passt"))
    if kbps is not None and 48 <= kbps <= 192:
        score += 5
    score += min(int(release.get("grabs") or 0), 200) / 20
    score += share * 5 - (8 if pack else 0)
    return {"ok": True, "score": round(score, 2), "reason": None, "notes": notes, "kbps": kbps}


def rank(results: List[dict], book: dict, **kw) -> List[dict]:
    """All results with their verdict, the accepted ones first (best first)."""
    out = [{**r, "verdict": evaluate(r, book, **kw)} for r in results]
    out.sort(key=lambda r: (not r["verdict"]["ok"], -r["verdict"]["score"]))
    return out
