"""Entdecken: audiobook suggestions – for your library, by genre, and from book reviews.

Sources
* Audible "similar titles" for the books in the library (what listeners connect with them)
* Audible genre lists (best sellers and new releases)
* Deutschlandfunk Kultur "Buchkritik" (RSS): reviewed books, matched against Audible – only
  what really exists as an audiobook is shown

Quality: minimum rating and number of ratings, unabridged, German, no adult titles, nothing
already in the library. Whether a non-fiction book is *true* cannot be checked automatically –
so by default non-fiction only comes from the editorial source, never from Audible lists.

Lists are cached in the database and refreshed in the background (at most once a day unless asked).
"""
from __future__ import annotations

import re
import threading
import time
import xml.etree.ElementTree as ET
from typing import Callable, Dict, List, Optional
from urllib import parse

import applog
import audible
import db as dbm
from i18n import t

MAX_AGE = 24 * 3600
SECTIONS = ("foryou", "genre", "critics")
FEEDS = (("Deutschlandfunk Kultur", "https://www.deutschlandfunkkultur.de/buchkritik-100.rss"),)
FEED_HOSTS = {parse.urlparse(u).hostname for _, u in FEEDS}
# Top-level genres that are non-fiction (German and English store names)
NONFICTION = {
    "Beziehungen, Elternschaft & persönliche Entwicklung", "Bildung & Lernen", "Biografien & Erinnerungen",
    "Computer & Technologie", "Geld & Finanzen", "Geschichte", "Gesundheit & Wellness", "Haushalt & Garten",
    "Politik & Sozialwissenschaften", "Reisen & Tourismus", "Religion & Spiritualität", "Sport & Freizeit",
    "Wirtschaft & Karriere", "Wissenschaft & Technik",
    "Relationships, Parenting & Personal Development", "Education & Learning", "Biographies & Memoirs",
    "Computers & Technology", "Money & Finance", "History", "Health & Wellness", "Home & Garden",
    "Politics & Social Sciences", "Travel & Tourism", "Religion & Spirituality", "Sports & Outdoors",
    "Business & Careers", "Science & Engineering",
}
HIDDEN_GENRES = {"Erotik", "Erotica"}
ITEM_KEYS = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_position", "runtime_min",
             "release_date", "cover_url", "url", "rating", "ratings", "genres", "format_type", "language", "region")
# "Buchkritik - „Titel“ von Autor", "Buchkritik: "Titel" v. A. Autor", "Autor, Autor2: "Titel""
REVIEW_RX = re.compile(r'^\s*(?:buchkritik\s*[-:–]\s*)?(?:(?P<a1>[^"„»“]+?)\s*:\s*)?["„»“](?P<title>[^"“”«]+)["“”«]'
                       r'(?:\s*(?:von|v\.)\s*(?P<a2>.+?))?\s*$', re.I)


def parse_review_title(text: str) -> Optional[dict]:
    """Title and first author of a review headline, or None for interviews, essays & co."""
    m = REVIEW_RX.match(text or "")
    if not m:
        return None
    author = (m.group("a2") or m.group("a1") or "").strip()
    author = re.split(r",|\bund\b|\s&\s", author)[0].strip()
    title = m.group("title").strip()
    return {"title": title, "author": author} if title and author else None


def is_nonfiction(book: dict) -> bool:
    return any(g in NONFICTION for g in book.get("genres") or [])


class Discover:
    def __init__(self, db: dbm.Database, client, index, settings: Callable[[], dict],
                 fetch: Optional[Callable[[str], bytes]] = None) -> None:
        self.db = db
        self.client = client            # AudibleClient
        self.index = index              # LibraryIndex
        self.settings = settings
        self.fetch = fetch or self._fetch_feed
        self.lock = threading.Lock()
        self.running: set = set()

    # -- cache ---------------------------------------------------------------
    def _key(self, section: str, genre: str = "") -> str:
        return f"{section}:{genre}" if section == "genre" else section

    def _cache(self) -> dict:
        data = self.db.get("discover") or {}
        return data if isinstance(data, dict) else {}

    def get(self, section: str, genre: str = "") -> dict:
        """The cached list; refreshed in the background when it is missing or older than a day."""
        key = self._key(section, genre)
        entry = self._cache().get(key) or {}
        if not entry or time.time() - entry.get("time", 0) > MAX_AGE:
            self.refresh(section, genre)
        with self.lock:
            running = key in self.running
        items = [i for i in entry.get("items") or [] if not self._owned(i)]   # bought in the meantime
        return {"items": items, "updated": entry.get("time"), "error": entry.get("error"), "running": running}

    def refresh(self, section: str, genre: str = "", wait: bool = False) -> None:
        if section not in SECTIONS or (section == "genre" and not genre):
            return
        key = self._key(section, genre)
        with self.lock:
            if key in self.running:
                return
            self.running.add(key)

        def run() -> None:
            try:
                items = {"foryou": self._for_you, "critics": self._critics}.get(section, lambda: self._genre(genre))()
                entry = {"time": time.time(), "items": items, "error": None}
            except Exception as e:  # noqa: BLE001 – a source being down must not break the page
                source = FEEDS[0][0] if section == "critics" else "Audible"
                applog.warn(t("Entdecken: {source} nicht erreichbar: {error}", source=source, error=e), "Entdecken")
                entry = {**(self._cache().get(key) or {}), "time": time.time(), "error": str(e)}
            with self.db.tx():
                self.db.put("discover", {**self._cache(), key: entry})
            with self.lock:
                self.running.discard(key)

        if wait:
            run()
        else:
            threading.Thread(target=run, daemon=True, name=f"discover-{key}").start()

    # -- filters ---------------------------------------------------------------
    def _region(self) -> str:
        return self.settings().get("ab_region", "de")

    def _owned(self, book: dict) -> bool:
        if not self.index:
            return False
        asin = (book.get("asin") or "").upper()
        with self.index.lock:
            if asin and any(e.get("asin") == asin for e in self.index.entries.values()):
                return True
        return bool(self.index.find(book))

    def _good(self, book: dict, from_store: bool = True) -> bool:
        """Quality rules; reviewed books (from_store=False) skip the rating rule – they are often new."""
        s = self.settings()
        if book.get("is_adult") or any(g in HIDDEN_GENRES for g in book.get("genres") or []):
            return False
        if s.get("discover_unabridged", True) and (book.get("format_type") or "").lower() == "abridged":
            return False
        if s.get("discover_german", True) and (book.get("language") or "").lower() not in ("german", "deutsch"):
            return False
        if from_store:
            if is_nonfiction(book) and not s.get("discover_nonfiction_store", False):
                return False
            if book.get("rating", 0) < float(s.get("discover_min_rating") or 0):
                return False
            if book.get("ratings", 0) < int(s.get("discover_min_count") or 0):
                return False
        return not self._owned(book)

    @staticmethod
    def _item(book: dict, **extra) -> dict:
        return {**{k: book.get(k) for k in ITEM_KEYS}, **extra}

    # -- sources ---------------------------------------------------------------
    def _for_you(self) -> List[dict]:
        """Audible's similar titles for the newest books of the library, counted across all of them."""
        with self.index.lock:
            seeds = sorted((e for e in self.index.entries.values() if e.get("asin")),
                           key=lambda e: -(e.get("mtime") or 0))[:30]
        if not seeds:
            return []
        region = self._region()
        found: Dict[str, dict] = {}
        because: Dict[str, List[str]] = {}
        for seed in seeds:
            try:
                sims = self.client.similar(seed["asin"], region, 15)
            except audible.AudibleError:
                continue
            for b in sims:
                found.setdefault(b["asin"], b)
                because.setdefault(b["asin"], [])
                if seed.get("title") not in because[b["asin"]]:
                    because[b["asin"]].append(seed.get("title") or "")
        good = [b for b in found.values() if self._good(b)]
        good.sort(key=lambda b: (-len(because[b["asin"]]), -b.get("rating", 0), -b.get("ratings", 0)))
        return [self._item(b, because=because[b["asin"]][:3], source="audible") for b in good[:48]]

    def _genre(self, genre_id: str) -> List[dict]:
        region = self._region()
        out: Dict[str, dict] = {}
        for sort in ("BestSellers", "-ReleaseDate"):
            for b in self.client.genre_products(region, genre_id, sort, 50):
                if b["asin"] not in out and self._good(b):
                    out[b["asin"]] = self._item(b, source="audible", list=sort)
        return list(out.values())[:48]

    def _fetch_feed(self, url: str) -> bytes:
        return audible.http_get(url, accept_json=False, max_bytes=4 << 20,
                                allow=lambda u: parse.urlparse(u).scheme == "https" and parse.urlparse(u).hostname in FEED_HOSTS)

    def _critics(self) -> List[dict]:
        """Reviewed books that exist as an audiobook (and are not in the library yet)."""
        region = self._region()
        out: List[dict] = []
        seen = set()
        for source, url in FEEDS:
            raw = self.fetch(url)
            if re.search(rb"<!DOCTYPE|<!ENTITY", raw[:4096], re.I):
                raise ValueError(t("Antwort enthält eine DTD – abgelehnt"))
            for item in ET.fromstring(raw).iter("item"):
                review = parse_review_title(item.findtext("title") or "")
                if not review:
                    continue
                link = (item.findtext("link") or "").strip()
                if not link.startswith("https://"):
                    link = ""
                book = self._audiobook_for(review, region)
                if not book or book["asin"] in seen or not self._good(book, from_store=False):
                    continue
                seen.add(book["asin"])
                out.append(self._item(book, source="review", review={
                    "source": source, "link": link, "date": (item.findtext("pubDate") or "")[:16]}))
        return out

    def _audiobook_for(self, review: dict, region: str) -> Optional[dict]:
        """The Audible audiobook of a reviewed book – same title (roughly) and same author."""
        try:
            hits = self.client.search(region, title=review["title"], author=review["author"], limit=5)
        except audible.AudibleError:
            return None
        surname = (review["author"].split() or [""])[-1].lower()
        for b in hits:
            if surname and not any(surname in a.lower() for a in b.get("authors") or []):
                continue
            if audible._ratio(b.get("title") or "", review["title"]) >= 0.6:
                return b
        return None

    def genres(self) -> List[dict]:
        """Store genres for the genre picker; non-fiction ones are marked (hidden unless allowed)."""
        try:
            gs = self.client.genres(self._region())
        except audible.AudibleError:
            return []
        allow_nf = self.settings().get("discover_nonfiction_store", False)
        return [{**g, "nonfiction": g["name"] in NONFICTION} for g in gs
                if g["name"] not in HIDDEN_GENRES and (allow_nf or g["name"] not in NONFICTION)]
