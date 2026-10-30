"""Book metadata from several sources: Audible first, others when Audible has nothing.

* Audible catalog API – the reference (runtime, narrators, series, chapters).
* Audnexus – Audible data by ASIN; steps in when the Audible API is down.
* Apple Books (iTunes Search API) – audiobook shop with German titles; no narrators/runtime.
* Open Library – print editions; title, author, year, cover as a last resort.

Only Audible/Audnexus hits can be accepted automatically. Hits from the other
shops lack runtime and narrators, so they always go to the review queue.
"""
from __future__ import annotations

import json
import re
from typing import Callable, List, Optional, Tuple
from urllib import parse

import audible
from i18n import t

AUTO_SOURCES = ("audible", "audnexus")
ITUNES = "https://itunes.apple.com/search"
OPENLIBRARY = "https://openlibrary.org/search.json"
ITUNES_COUNTRY = {"de": "de", "us": "us", "uk": "gb", "au": "au", "ca": "ca", "fr": "fr", "it": "it",
                  "es": "es", "in": "in", "jp": "jp"}
HOSTS = ("api.audnex.us", "itunes.apple.com", "openlibrary.org")


def _host_ok(url: str) -> bool:
    p = parse.urlparse(url)
    return p.scheme == "https" and (p.hostname or "").lower() in HOSTS


def get_json(url: str) -> dict:
    try:
        return json.loads(audible.http_get(url, timeout=15, retries=2, allow=_host_ok))
    except ValueError as e:
        raise audible.AudibleError(t("Ungültige Antwort von {url}", url=parse.urlparse(url).hostname)) from e


def _book(source: str, **kw) -> dict:
    book = {"asin": None, "title": "", "subtitle": None, "authors": [], "narrators": [], "translators": [],
            "series": None, "series_position": None, "publisher": None, "release_date": None,
            "language": None, "runtime_min": 0, "format_type": None, "genres": [], "tags": [],
            "description": "", "cover_url": None, "region": None, "url": None, "source": source, "isbn": None}
    book.update({k: v for k, v in kw.items() if v is not None})
    return book


def _names(text: str) -> List[str]:
    return [n.strip() for n in re.split(r"\s*(?:,|&|\bund\b|\band\b)\s*", text or "") if n.strip()]


# --------------------------------------------------------------------------- #
# Normalisation of each source
# --------------------------------------------------------------------------- #

def from_audnexus(d: dict, region: str) -> dict:
    series = d.get("seriesPrimary") or {}
    return _book(
        "audnexus", id=d.get("asin"), asin=d.get("asin"), title=(d.get("title") or "").strip(),
        subtitle=(d.get("subtitle") or "").strip() or None,
        authors=[a["name"] for a in d.get("authors") or [] if a.get("name")],
        narrators=[n["name"] for n in d.get("narrators") or [] if n.get("name")],
        series=series.get("name"), series_position=audible.series_position(series.get("position")),
        publisher=d.get("publisherName"), release_date=(d.get("releaseDate") or "")[:10] or None,
        language=(d.get("language") or "").capitalize() or None, runtime_min=int(d.get("runtimeLengthMin") or 0),
        format_type=d.get("formatType"),
        genres=[g["name"] for g in d.get("genres") or [] if g.get("type") == "genre" and g.get("name")],
        tags=[g["name"] for g in d.get("genres") or [] if g.get("type") == "tag" and g.get("name")],
        description=audible.html_to_text(d.get("summary") or d.get("description") or ""),
        cover_url=d.get("image"), region=region, url=audible.store_url(d.get("asin", ""), region))


def from_itunes(d: dict) -> dict:
    cover = d.get("artworkUrl100") or d.get("artworkUrl60")
    if cover:
        cover = re.sub(r"/\d+x\d+bb\.(jpg|png)$", r"/1400x1400bb.\1", cover)
    title = re.sub(r"\s*\((?:ungekürzt|gekürzt|unabridged|abridged)\)\s*$", "", d.get("collectionName") or "", flags=re.I)
    return _book(
        "apple", id=f"apple:{d.get('collectionId')}", title=title.strip(), authors=_names(d.get("artistName") or ""),
        release_date=(d.get("releaseDate") or "")[:10] or None,
        genres=[d["primaryGenreName"]] if d.get("primaryGenreName") else [],
        description=audible.html_to_text(d.get("description") or ""), cover_url=cover,
        url=(d.get("collectionViewUrl") or "").split("?")[0] or None)


def from_openlibrary(d: dict) -> dict:
    return _book(
        "openlibrary", id=f"openlibrary:{d.get('key')}", title=(d.get("title") or "").strip(),
        subtitle=d.get("subtitle"), authors=list(d.get("author_name") or [])[:4],
        release_date=str(d["first_publish_year"]) if d.get("first_publish_year") else None,
        cover_url=f"https://covers.openlibrary.org/b/id/{d['cover_i']}-L.jpg" if d.get("cover_i") else None,
        url=f"https://openlibrary.org{d['key']}" if d.get("key") else None,
        isbn=(d.get("isbn") or [None])[0])


# --------------------------------------------------------------------------- #
# Lookups
# --------------------------------------------------------------------------- #

class Providers:
    def __init__(self, client: audible.AudibleClient, get: Callable[[str], dict] = get_json,
                 fallbacks: Tuple[str, ...] = ("audnexus", "apple", "openlibrary"),
                 enabled: Callable[[], bool] = lambda: True) -> None:
        self.client = client
        self.enabled = enabled           # setting "ab_fallback": other shops on/off
        self._get = get
        self.fallbacks = fallbacks

    def audnexus(self, asin: str, region: str) -> dict:
        return from_audnexus(self._get(f"https://api.audnex.us/books/{asin}?region={region}"), region)

    def apple(self, hints: dict, region: str, limit: int = 8) -> List[dict]:
        term = " ".join(filter(None, [hints.get("title"), hints.get("author")])) or hints.get("query") or ""
        if not term:
            return []
        params = {"term": term, "media": "audiobook", "country": ITUNES_COUNTRY.get(region, "us"), "limit": limit}
        data = self._get(f"{ITUNES}?{parse.urlencode(params)}")
        return [from_itunes(d) for d in data.get("results") or [] if d.get("collectionName")]

    def openlibrary(self, hints: dict, limit: int = 5) -> List[dict]:
        params = {"limit": limit, "fields": "key,title,subtitle,author_name,first_publish_year,cover_i,isbn"}
        if hints.get("title"):
            params["title"] = hints["title"]
            if hints.get("author"):
                params["author"] = hints["author"]
        elif hints.get("query"):
            params["q"] = hints["query"]
        else:
            return []
        data = self._get(f"{OPENLIBRARY}?{parse.urlencode(params)}")
        return [from_openlibrary(d) for d in data.get("docs") or [] if d.get("title")]

    def lookup(self, hints: dict, region: str) -> Tuple[List[dict], List[str]]:
        """Ranked candidates and notes for the job log (which source failed or helped)."""
        ranked, notes, _failed = self._lookup(hints, region)
        return ranked, notes

    def _lookup(self, hints: dict, region: str) -> Tuple[List[dict], List[str], bool]:
        notes: List[str] = []
        failed = False
        try:
            ranked = audible.lookup(self.client, hints, region)
        except audible.AudibleError as e:
            ranked, failed = [], True
            notes.append(str(e) if str(e).startswith("Audible") else t("Audible nicht erreichbar: {error}", error=e))
        if ranked or not self.enabled():
            return ranked, notes, failed
        found: List[dict] = []
        if failed and hints.get("asin") and "audnexus" in self.fallbacks:
            try:
                found.append(self.audnexus(hints["asin"], region))
                notes.append(t("Daten von Audnexus (Audible-Spiegel)"))
            except audible.AudibleError as e:
                notes.append(f"Audnexus: {e}")
        if not found:
            for name, fn in (("Apple Books", lambda: self.apple(hints, region)),
                             ("Open Library", lambda: self.openlibrary(hints))):
                key = "apple" if name == "Apple Books" else "openlibrary"
                if key not in self.fallbacks:
                    continue
                try:
                    hits = fn()
                except audible.AudibleError as e:
                    notes.append(f"{name}: {e}")
                    continue
                if hits:
                    notes.append(t("{n} Treffer bei {source}", n=len(hits), source=name))
                found += hits
        return audible.rank(found, hints), notes, failed

    def search(self, hints: dict, region: str) -> List[dict]:
        """Manual search in the review: Audible errors are reported, the other shops fill gaps."""
        ranked, notes, failed = self._lookup(hints, region)
        if not ranked and failed:
            raise audible.AudibleError(notes[0])
        return ranked


def auto_match(ranked: List[dict], threshold: float) -> Optional[dict]:
    """Automatic acceptance only for Audible data (runtime and narrators known)."""
    return audible.best_match([b for b in ranked if b.get("source") in AUTO_SOURCES], threshold)
