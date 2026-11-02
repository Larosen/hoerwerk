"""Entdecken: audiobook suggestions – for your library, by genre, and from book reviews.

Sources
* Audible "similar titles" for the books in the library (what listeners connect with them)
* Audible genres and their subcategories, as in the shop (best sellers, best rated, new releases; small
  genres are filled up with titles rated just as well by fewer listeners)
* Book reviews (RSS): Deutschlandfunk Kultur "Buchkritik" / "Lesart", Deutschlandfunk "Büchermarkt",
  Perlentaucher's audiobook lists and "Bücherschau des Tages" /
  "Bücherbrief" – only author and title are taken, matched against Audible (only what really exists as an
  audiobook is shown) and linked back to the review. Reviews are remembered for 180 days, since the feeds
  only carry the latest few.

Quality: minimum rating and number of ratings, unabridged, German, no adult titles, nothing
already in the library (or being downloaded / processed right now). Whether a non-fiction book is *true* cannot be checked automatically –
so non-fiction from Audible lists is only shown when the user picks a non-fiction genre (and marked there),
never mixed into "Für dich" unless allowed in the settings.

Lists are cached in the database and refreshed in the background (at most once a day unless asked).
"""
from __future__ import annotations

import calendar
import re
import threading
import html
import time
import unicodedata
import xml.etree.ElementTree as ET
from typing import Callable, Dict, List, Optional
from urllib import parse

import applog
import audible
import db as dbm
from i18n import t

MAX_AGE = 24 * 3600
SECTIONS = ("foryou", "genre", "critics")
# (source, address, kind): RSS feeds, and Perlentaucher's list of reviewed books that came out as audiobooks
FEEDS = (("Deutschlandfunk Kultur", "https://www.deutschlandfunkkultur.de/buchkritik-100.rss", "rss"),
         ("Deutschlandfunk Kultur", "https://www.deutschlandfunkkultur.de/lesart-100.rss", "rss"),
         ("Deutschlandfunk", "https://www.deutschlandfunk.de/buechermarkt-100.rss", "rss"),
         ("Perlentaucher", "https://www.perlentaucher.de/buchKSL/hoerbuecher-literatur-aktuell.html", "list"),
         ("Perlentaucher", "https://www.perlentaucher.de/buchKSL/hoerbuecher-sachbuch-aktuell.html", "list"),
         ("Perlentaucher", "https://www.perlentaucher.de/index.rss", "rss"))
PT_BASE = "https://www.perlentaucher.de"
_PT_BOOK = re.compile(r'<h2>\s*<a href="(?P<href>/buch/[^"]+\.html)">\s*(?P<author>[^<:]{2,80}):\s*<span>(?P<title>[^<]{1,200})</span>'
                      r'(?P<rest>.*?)</article>', re.S)
_PT_NOTES = re.compile(r"<span>\s*(\d{1,3})\s+Notiz")
REVIEW_DAYS = 180
MAX_REVIEWS = 1500
CRITICS_MAX = 40          # newest reviewed audiobooks shown (older remembered reviews keep the list long)
# Perlentaucher items that recommend books (the feed also carries film, debates, essays …)
PT_CATEGORIES = ("BÜCHERSCHAU", "BÜCHERBRIEF", "TASCHENBÜCHERBRIEF")
FEED_HOSTS = {parse.urlparse(u).hostname for _, u, _ in FEEDS}
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
GENRE_MIN = 30            # a genre list is filled up to this many (with fewer-rated titles if needed)
GENRE_MAX = 48
# fill-up rule for small genres: still well rated, but by fewer listeners
LOOSE_RATING, LOOSE_COUNT = 4.0, 5
WARM_PAUSE = 1.0          # seconds between two genres when all lists are prepared in the background
ITEM_KEYS = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_position", "runtime_min",
             "release_date", "cover_url", "url", "rating", "ratings", "genres", "format_type", "language", "region")
# "Buchkritik - „Titel“ von Autor", "Buchkritik: "Titel" v. A. Autor", "Autor, Autor2: "Titel"",
# "Autor: „Titel“ – Unterzeile (Buchkritik)"
REVIEW_RX = re.compile(r'^\s*(?:buchkritik\s*[-:–]\s*)?(?:(?P<a1>[^"„»“]+?)\s*:\s*)?["„»“](?P<title>[^"“”«]+)["“”«]'
                       r'(?:\s*(?:von|v\.)\s*(?P<a2>.+?))?(?:\s+[-–—]\s+.*)?\s*$', re.I)


def parse_review_title(text: str) -> Optional[dict]:
    """Title and first author of a review headline, or None for interviews, essays & co."""
    m = REVIEW_RX.match(text or "")
    if not m:
        return None
    author = (m.group("a2") or m.group("a1") or "").strip()
    author = re.split(r",|\bund\b|\s&\s|\(", author)[0].strip()
    title = m.group("title").strip()
    return {"title": title, "author": author} if title and author else None


_PT_TOKEN = re.compile(r'<b>\s*(?P<name>[^<]{3,60}?)\s*</b>|["„“»](?P<title>[^"„“”«»<]{2,150})["“”«]')
_NAME_WORD = re.compile(r"^(?:[A-ZÄÖÜÀ-Þ][\w'’.-]*|von|van|de|der|del|da|di|du|le|la|al|ibn|bin|zu|ten|ter)$")


def _person(text: str) -> Optional[str]:
    """A bold text that looks like a name ("Shida Bazyar", "Jachym Topols") – not "Schuld und Sühne"."""
    words = html.unescape(text).split()
    if not 2 <= len(words) <= 5 or not all(_NAME_WORD.match(w) for w in words):
        return None
    if not (words[0][:1].isupper() and words[-1][:1].isupper()):
        return None
    return " ".join(words)


def parse_book_mentions(text: str) -> List[dict]:
    """Author/title pairs from a Perlentaucher teaser: <b>Author</b> … "Title" in one sentence."""
    out: List[dict] = []
    for sentence in re.split(r"(?<=[.!?])\s+(?=[A-ZÄÖÜ<])", text or ""):
        author = None
        for m in _PT_TOKEN.finditer(sentence):
            if m.group("name"):
                author = _person(m.group("name")) or author
            elif author:
                out.append({"title": html.unescape(m.group("title")).strip(), "author": author})
                author = None
    return out


def parse_pt_list(text: str) -> List[dict]:
    """Books on a Perlentaucher list page: author, title (without subtitle), link and number of reviews."""
    out: List[dict] = []
    for m in _PT_BOOK.finditer(text or ""):
        title = html.unescape(m.group("title")).strip()
        title = re.split(r"\.\s+(?=[A-ZÄÖÜ])", title)[0].strip()        # "Radio Nacht. Roman" -> "Radio Nacht"
        notes = _PT_NOTES.search(m.group("rest"))
        out.append({"title": title, "author": html.unescape(m.group("author")).strip(),
                    "link": PT_BASE + m.group("href"), "notes": int(notes.group(1)) if notes else 0})
    return out


def _review_time(review: dict) -> float:
    """When a review appeared: the feed date, else when it was first seen (lists have no date)."""
    try:                                   # stored shortened: "Fri, 30 Oct 2026"
        return calendar.timegm(time.strptime((review.get("date") or "")[:16], "%a, %d %b %Y"))
    except ValueError:
        return review.get("seen", 0)


def _mix_sources(reviews: List[dict]) -> List[dict]:
    """Newest first within each source, the sources taking turns – so one busy feed does not push out the others."""
    by_source: Dict[str, List[dict]] = {}
    for r in sorted(reviews, key=lambda r: -_review_time(r)):
        by_source.setdefault(r.get("source") or "", []).append(r)
    queues = list(by_source.values())
    out: List[dict] = []
    while queues:
        for q in queues:
            out.append(q.pop(0))
        queues = [q for q in queues if q]
    return out


def _fold(text: str) -> str:
    """Lower case without accents: "Faulerová" -> "faulerova"."""
    return "".join(c for c in unicodedata.normalize("NFKD", text or "") if not unicodedata.combining(c)).lower()


def _surnames(author: str) -> List[str]:
    """Surname of a reviewed author, also without the German genitive s ("Kapralovas" -> "kapralova")."""
    last = _fold((author.split() or [""])[-1]).strip(".,")
    return [last, last[:-1]] if len(last) > 3 and last.endswith("s") else [last]


def is_nonfiction(book: dict) -> bool:
    return any(g in NONFICTION for g in book.get("genres") or [])


class Discover:
    def __init__(self, db: dbm.Database, client, index, settings: Callable[[], dict],
                 fetch: Optional[Callable[[str], bytes]] = None,
                 pending: Optional[Callable[[], set]] = None, prepare: bool = True) -> None:
        self.db = db
        self.client = client            # AudibleClient
        self.index = index              # LibraryIndex
        self.settings = settings
        self.fetch = fetch or self._fetch_feed
        self.pending = pending or set     # ASINs being downloaded or processed right now
        self.seeds: Callable[[int], List[dict]] = lambda user_id: []   # what an account heard (listening.py)
        self.lock = threading.Lock()
        self.running: set = set()
        self._known: set = set()          # genre ids of the store
        self._warming = not prepare     # tests: no background preparation
        db.run("DELETE FROM kv WHERE key='discover'")     # the single big cache row of 5.x–7.2
        self._warmed = 0.0

    # -- cache ---------------------------------------------------------------
    def _key(self, section: str, genre: str = "") -> str:
        return f"{section}:{genre}" if section == "genre" else section

    def _entry(self, key: str) -> dict:
        """One cached list (each in its own row – there are ~200 genre lists)."""
        data = self.db.get(f"discover:{key}") or {}
        return data if isinstance(data, dict) else {}

    def _personal_seeds(self, user_id: Optional[int]) -> List[dict]:
        """What this account heard (Audiobookshelf / Plex) – the seeds of its own "Für dich"."""
        if not user_id:
            return []
        try:
            return self.seeds(user_id) or []
        except Exception as e:  # noqa: BLE001 – then the suggestions for the whole library
            applog.warn(t("Entdecken: Hörverlauf nicht lesbar: {error}", error=e), "Entdecken")
            return []

    def get(self, section: str, genre: str = "", user_id: Optional[int] = None) -> dict:
        """The cached list; refreshed in the background when it is missing or older than a day.

        "Für dich" is personal when the account's listening history is known (linked Audiobookshelf / Plex)."""
        seeds = self._personal_seeds(user_id) if section == "foryou" else []
        key = f"foryou:u{user_id}" if seeds else self._key(section, genre)
        entry = self._entry(key)
        if not entry or time.time() - entry.get("time", 0) > MAX_AGE:
            self.refresh(section, genre, seeds=seeds, key=key)
        with self.lock:
            running = key in self.running
        busy = self._busy()
        items = [i for i in entry.get("items") or [] if i.get("asin") not in busy and not self._owned(i)]   # got it meanwhile
        return {"items": items, "updated": entry.get("time"), "error": entry.get("error"), "running": running,
                "personal": bool(seeds)}

    def refresh(self, section: str, genre: str = "", wait: bool = False, seeds: Optional[List[dict]] = None,
                key: str = "", user_id: Optional[int] = None) -> None:
        if section not in SECTIONS or (section == "genre" and not (genre and self.known_genre(genre))):
            return
        if section == "foryou" and user_id and seeds is None:
            seeds = self._personal_seeds(user_id)
            key = f"foryou:u{user_id}" if seeds else ""
        key = key or self._key(section, genre)
        with self.lock:
            if key in self.running:
                return
            self.running.add(key)

        def run() -> None:
            try:
                items = {"foryou": lambda: self._for_you(seeds), "critics": self._critics}.get(section, lambda: self._genre(genre))()
                entry = {"time": time.time(), "items": items, "error": None}
            except Exception as e:  # noqa: BLE001 – a source being down must not break the page
                source = " / ".join(dict.fromkeys(n for n, _, _ in FEEDS)) if section == "critics" else "Audible"
                applog.warn(t("Entdecken: {source} nicht erreichbar: {error}", source=source, error=e), "Entdecken")
                entry = {**self._entry(key), "time": time.time(), "error": str(e)}
            with self.db.tx():
                self.db.put(f"discover:{key}", entry)
            with self.lock:
                self.running.discard(key)

        if wait:
            run()
        else:
            threading.Thread(target=run, daemon=True, name=f"discover-{key}").start()

    # -- filters ---------------------------------------------------------------
    def _region(self) -> str:
        return self.settings().get("ab_region", "de")

    def _busy(self) -> set:
        try:
            return {a.upper() for a in self.pending() if a}
        except Exception:  # noqa: BLE001 – only a filter
            return set()

    def _owned(self, book: dict) -> bool:
        if not self.index:
            return False
        asin = (book.get("asin") or "").upper()
        with self.index.lock:
            if asin and any(e.get("asin") == asin for e in self.index.entries.values()):
                return True
        return bool(self.index.find(book))

    def _good(self, book: dict, from_store: bool = True, chosen_genre: bool = False, loose: bool = False) -> bool:
        """Quality rules; reviewed books (from_store=False) skip the rating rule – they are often new.

        chosen_genre: the user picked this genre themselves – non-fiction is then shown too (marked in the page).
        loose: fill-up for small genres – rated 4.0+ by at least a few listeners instead of the strict minimum."""
        s = self.settings()
        if book.get("is_adult") or any(g in HIDDEN_GENRES for g in book.get("genres") or []):
            return False
        if s.get("discover_unabridged", True) and (book.get("format_type") or "").lower() == "abridged":
            return False
        if s.get("discover_german", True) and (book.get("language") or "").lower() not in ("german", "deutsch"):
            return False
        if from_store:
            if is_nonfiction(book) and not chosen_genre and not s.get("discover_nonfiction_store", False):
                return False
            min_rating, min_count = float(s.get("discover_min_rating") or 0), int(s.get("discover_min_count") or 0)
            if loose:
                min_rating, min_count = min(min_rating, LOOSE_RATING), min(min_count, LOOSE_COUNT)
            if book.get("rating", 0) < min_rating:
                return False
            if book.get("ratings", 0) < min_count:
                return False
        return (book.get("asin") or "").upper() not in self._busy() and not self._owned(book)

    @staticmethod
    def _item(book: dict, **extra) -> dict:
        return {**{k: book.get(k) for k in ITEM_KEYS}, **extra}

    # -- sources ---------------------------------------------------------------
    def _for_you(self, seeds: Optional[List[dict]] = None) -> List[dict]:
        """Audible's similar titles, counted across the seeds: the books this account heard (personal), or else
        the newest books of the library."""
        if not seeds:
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
        """Best sellers, best rated and new releases of a genre. Small genres (few German titles with many
        ratings) are filled up with books rated just as well by fewer listeners, so no genre stays empty."""
        region = self._region()
        strict: Dict[str, dict] = {}
        loose: Dict[str, dict] = {}
        fetches = [("BestSellers", 1), ("AvgRating", 1), ("-ReleaseDate", 1), ("BestSellers", 2), ("AvgRating", 2)]
        for n, (sort, page) in enumerate(fetches):
            if n >= 3 and len(strict) >= GENRE_MIN:
                break
            for b in self.client.genre_products(region, genre_id, sort, 50, page=page):
                if b["asin"] in strict or b["asin"] in loose:
                    continue
                if self._good(b, chosen_genre=True):
                    strict[b["asin"]] = self._item(b, source="audible", list=sort)
                elif self._good(b, chosen_genre=True, loose=True):
                    loose[b["asin"]] = self._item(b, source="audible", list=sort, loose=True)
        out = list(strict.values())[:GENRE_MAX]
        if len(out) < GENRE_MIN:
            out += sorted(loose.values(), key=lambda b: (-(b.get("rating") or 0), -(b.get("ratings") or 0)))[:GENRE_MIN - len(out)]
        return out

    def _fetch_feed(self, url: str) -> bytes:
        """Only the fixed feed addresses, HTTPS, max. 4 MB."""
        return audible.http_get(url, accept_json=False, max_bytes=4 << 20,
                                allow=lambda u: parse.urlparse(u).scheme == "https" and parse.urlparse(u).hostname in FEED_HOSTS)

    def _reviews_from(self, source: str, raw: bytes, kind: str = "rss") -> List[dict]:
        """{title, author, link, date} of the books a feed reviews or recommends."""
        if kind == "list":
            return [{**r, "source": source, "date": ""} for r in parse_pt_list(raw.decode("utf-8", "replace"))]
        if re.search(rb"<!DOCTYPE|<!ENTITY", raw[:4096], re.I):
            raise ValueError(t("Antwort enthält eine DTD – abgelehnt"))
        out: List[dict] = []
        for item in ET.fromstring(raw).iter("item"):
            link = (item.findtext("link") or "").strip()
            meta = {"source": source, "link": link if link.startswith("https://") else "",
                    "date": (item.findtext("pubDate") or "")[:16]}
            if source == "Perlentaucher":
                if not (item.findtext("category") or "").strip().upper().startswith(PT_CATEGORIES):
                    continue
                found = parse_book_mentions(item.findtext("description") or "")
            else:
                one = parse_review_title(item.findtext("title") or "")
                found = [one] if one else []
            out += [{**r, **meta} for r in found]
        return out

    def _critics(self) -> List[dict]:
        """Reviewed books that exist as an audiobook (and are not in the library yet).

        Every review seen is remembered with its Audible match, so the list grows beyond the few newest
        feed items and each book is looked up only once."""
        region = self._region()
        memory = self.db.get("discover_reviews") or {}
        now, errors, fetched = time.time(), [], 0
        for source, url, kind in FEEDS:
            try:
                reviews = self._reviews_from(source, self.fetch(url), kind)
                fetched += 1
            except Exception as e:  # noqa: BLE001 – one feed down: still show the other
                errors.append(f"{source}: {e}")
                applog.warn(t("Entdecken: {source} nicht erreichbar: {error}", source=source, error=e), "Entdecken")
                continue
            for r in reviews:
                key = f"{region}|{_fold(r['author'])}|{_fold(r['title'])}"
                if key not in memory:
                    memory[key] = {**r, "seen": now, "book": self._audiobook_for(r, region)}
                else:                                   # still listed: keep it fresh, newest review count
                    memory[key].update({k: r[k] for k in ("notes",) if r.get(k)}, seen=max(memory[key].get("seen", 0), now - 1))
        if not fetched and errors:
            raise RuntimeError("; ".join(errors))
        fresh = sorted(((k, v) for k, v in memory.items() if now - v.get("seen", 0) < REVIEW_DAYS * 86400),
                       key=lambda kv: -kv[1].get("seen", 0))[:MAX_REVIEWS]
        memory = dict(fresh)
        with self.db.tx():
            self.db.put("discover_reviews", memory)
        out: List[dict] = []
        seen = set()
        for r in _mix_sources([v for _, v in fresh]):
            book = r.get("book")
            if not book or book["asin"] in seen or not self._good(book, from_store=False):
                continue
            seen.add(book["asin"])
            out.append(self._item(book, source="review", review={k: r.get(k) for k in ("source", "link", "date", "notes")}))
        return out[:CRITICS_MAX]

    def _audiobook_for(self, review: dict, region: str) -> Optional[dict]:
        """The Audible audiobook of a reviewed book – same title (roughly) and same author."""
        names = _surnames(review["author"])
        hits: List[dict] = []
        for author in (review["author"], (review["author"].split() or [""])[0] + " " + names[-1], ""):
            try:
                hits = self.client.search(region, title=review["title"], author=author.strip(), limit=5)
            except audible.AudibleError:
                return None
            if hits:
                break
        for b in hits:
            authors = [_fold(a) for a in b.get("authors") or []]
            if not any(n and n in a for n in names for a in authors):
                continue
            if audible._ratio(b.get("title") or "", review["title"]) >= 0.6:
                return {k: b.get(k) for k in ITEM_KEYS + ("is_adult", "format_type", "language", "genres")}
        return None

    def genres(self) -> List[dict]:
        """All store genres with their subcategories, like in the Audible shop (no adult ones).

        Non-fiction genres are marked. Subcategories that turned out to have no German audiobook at all
        (e.g. "Cricket") are left out once their list is known. Starts preparing all lists in the background."""
        try:
            gs = self.client.genres(self._region())
        except audible.AudibleError:
            return []
        def empty(gid: str) -> bool:
            entry = self._entry(self._key("genre", gid))
            return bool(entry) and not entry.get("items") and not entry.get("error")

        out = []
        for g in gs:
            if g["name"] in HIDDEN_GENRES:
                continue
            kids = [{"id": c["id"], "name": c["name"]} for c in g.get("children") or []
                    if c["name"] not in HIDDEN_GENRES and not empty(c["id"])]
            out.append({"id": g["id"], "name": g["name"], "nonfiction": g["name"] in NONFICTION, "children": kids})
        self._known = {g["id"] for g in out} | {c["id"] for g in out for c in g["children"]}
        self.warm(out)
        return out

    def known_genre(self, genre_id: str) -> bool:
        """Only store genres are looked up and cached (not any number someone sends)."""
        if not self._known:
            self.genres()
        return genre_id in self._known if self._known else genre_id.isdigit()

    def warm(self, tree: List[dict]) -> None:
        """Prepare every genre list in the background (one after the other, gently), at most once a day –
        so a picked genre shows at once, and empty subcategories can be hidden."""
        with self.lock:
            if self._warming or time.time() - self._warmed < MAX_AGE:
                return
            self._warming = True

        def run() -> None:
            try:
                for g in tree:
                    for gid in [g["id"]] + [c["id"] for c in g["children"]]:
                        entry = self._entry(self._key("genre", gid))
                        if entry and time.time() - entry.get("time", 0) < MAX_AGE:
                            continue
                        self.refresh("genre", gid, wait=True)
                        time.sleep(WARM_PAUSE)
            finally:
                with self.lock:
                    self._warming, self._warmed = False, time.time()

        threading.Thread(target=run, daemon=True, name="discover-warm").start()
