"""Gesucht: audiobooks you want that no indexer has had yet.

Books are added from Entdecken or from an Audible/BookBeat lookup. Every few hours (12 by default) each one
is searched on the indexers again. A release only counts when its name has the author's surname and nearly all
words of the title. New finds are announced via ntfy; sending the best one to SABnzbd automatically is an
option (off by default – a wrong release would otherwise be loaded unasked).
A book leaves the list by itself once it is in the library.
"""
from __future__ import annotations

import hashlib
import threading
import time
from typing import Callable, List, Optional

import applog
import db as dbm
from i18n import t
from usenet import UsenetError, match_words

KEYS = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_position", "runtime_min",
        "release_date", "cover_url", "url", "source", "region")
STOP = {"der", "die", "das", "und", "ein", "eine", "the", "and", "von", "mit", "roman", "band", "teil", "folge"}
MAX_ITEMS = 300


def _surname(name: str) -> str:
    words = match_words(name)
    return words[-1] if words else ""


def fits(release: str, book: dict) -> bool:
    """Is this release this book? Author's surname and (nearly) all words of the title in the name."""
    words = set(match_words(release))
    author = _surname((book.get("authors") or [""])[0])
    if author and author not in words:
        return False
    title = [w for w in match_words(book.get("title") or "") if w not in STOP and len(w) > 1]
    if not title:
        return False
    return sum(1 for w in title if w in words) / len(title) >= 0.8


def book_id(book: dict) -> str:
    if book.get("asin"):
        return str(book["asin"]).upper()
    key = f"{(book.get('authors') or [''])[0]}|{book.get('title')}".lower()
    return "t" + hashlib.sha1(key.encode()).hexdigest()[:12]


class Wanted:
    def __init__(self, db: dbm.Database, usenet, index, notifier, settings: Callable[[], dict],
                 pending: Optional[Callable[[], set]] = None) -> None:
        self.db = db
        self.usenet = usenet
        self.index = index
        self.notifier = notifier
        self.settings = settings
        self.pending = pending or set
        self.lock = threading.Lock()          # one search run at a time

    # -- storage ---------------------------------------------------------------
    def _load(self) -> dict:
        data = self.db.get("wanted") or {}
        return data if isinstance(data, dict) else {}

    def _save(self, data: dict) -> None:
        with self.db.tx():
            self.db.put("wanted", data)

    def _update(self, wid: str, **changes) -> Optional[dict]:
        data = self._load()
        if wid not in data:
            return None
        data[wid] = {**data[wid], **changes}
        self._save(data)
        return data[wid]

    def interval_hours(self) -> int:
        return max(1, int(self.settings().get("wanted_interval_hours") or 12))

    # -- list ------------------------------------------------------------------
    def _owned(self, item: dict) -> bool:
        if not self.index:
            return False
        asin = (item.get("asin") or "").upper()
        with self.index.lock:
            if asin and any(e.get("asin") == asin for e in self.index.entries.values()):
                return True
        return bool(self.index.find(item))

    def items(self) -> List[dict]:
        """The list, newest first; books that arrived in the library meanwhile are taken off."""
        data = self._load()
        arrived = [wid for wid, it in data.items() if self._owned(it)]
        if arrived:
            for wid in arrived:
                applog.info(t("Gesucht: „{title}“ ist jetzt in der Bibliothek", title=data[wid].get("title")), "Gesucht")
                del data[wid]
            self._save(data)
        try:
            busy = {a.upper() for a in self.pending() if a}
        except Exception:  # noqa: BLE001 – only a status
            busy = set()
        out = []
        for it in sorted(data.values(), key=lambda i: -(i.get("added") or 0)):
            loading = bool(it.get("asin")) and it["asin"].upper() in busy
            out.append({**it, "status": "loading" if loading else "found" if it.get("found") else "missing"})
        return out

    def ids(self) -> set:
        return set(self._load())

    def add(self, book: dict) -> dict:
        title = str(book.get("title") or "").strip()
        authors = [str(a) for a in book.get("authors") or [] if a][:5]
        if not title or not authors:
            raise ValueError(t("Titel und Autor fehlen"))
        data = self._load()
        if len(data) >= MAX_ITEMS:
            raise ValueError(t("Die Liste ist voll ({n} Bücher)", n=MAX_ITEMS))
        wid = book_id({**book, "title": title, "authors": authors})
        item = {**{k: book.get(k) for k in KEYS}, "title": title, "authors": authors, "id": wid,
                "added": data.get(wid, {}).get("added") or time.time(), "last_search": None, "found": [], "error": None}
        if isinstance(item.get("asin"), str):
            item["asin"] = item["asin"].upper()
        data[wid] = item
        self._save(data)
        applog.info(t("Gesucht: „{title}“ vorgemerkt", title=title), "Gesucht")
        return item

    def remove(self, wid: str) -> bool:
        data = self._load()
        if data.pop(wid, None) is None:
            return False
        self._save(data)
        return True

    # -- searching -------------------------------------------------------------
    def search(self, wid: str) -> Optional[dict]:
        """Search one book now (also used by the background run)."""
        item = self._load().get(wid)
        if not item:
            return None
        if not self.usenet.configured()["indexers"]:
            return self._update(wid, last_search=time.time(), error=t("Noch kein Indexer eingerichtet"))
        author = _surname((item.get("authors") or [""])[0])
        extra = [item["title"]]
        if item.get("series") and item.get("series_position"):
            extra.append(f"{item['series']} {item['series_position']}")
        lang = "de" if self.settings().get("discover_german", True) else ""
        try:
            res = self.usenet.search(f"{author} {item['title']}".strip(), lang, extra)
        except UsenetError as e:
            return self._update(wid, last_search=time.time(), error=str(e))
        hits = [r for r in res["results"] if fits(r["title"], item)]
        found = [{k: r.get(k) for k in ("title", "size", "indexer", "format", "language", "abridged", "published")}
                 for r in hits[:5]]
        before = {f["title"] for f in item.get("found") or []}
        new = [f for f in found if f["title"] not in before]
        updated = self._update(wid, last_search=time.time(), found=found,
                               error="; ".join(res.get("errors") or []) or None)
        if new:
            self._announce(item, hits)
        return updated

    def _announce(self, item: dict, hits: List[dict]) -> None:
        who = ", ".join(item.get("authors") or [])
        name = f"{who} – {item['title']}"
        applog.info(t("Gesucht: {n} Release(s) für „{title}“ gefunden", n=len(hits), title=item["title"]), "Gesucht")
        best = next((h for h in hits if not h.get("abridged")), hits[0])
        if self.settings().get("wanted_auto") and self.usenet.configured()["sab"] and not item.get("sent"):
            try:
                res = self.usenet.send(best["id"], {"title": item["title"], "author": who, "asin": item.get("asin")})
                self._update(item["id"], sent=time.time())
                self.notifier.send("wanted_found", t("Gesuchtes Hörbuch gefunden und geladen"), f"{name}\n{res['title']}")
                return
            except UsenetError as e:
                applog.warn(t("Gesucht: Download fehlgeschlagen: {error}", error=e), "Gesucht")
        self.notifier.send("wanted_found", t("Gesuchtes Hörbuch gefunden"), f"{name}\n{best['title']}")

    def run_due(self) -> int:
        """Search every book whose last search is older than the interval; returns how many were searched."""
        if not self.lock.acquire(blocking=False):
            return 0
        try:
            self.items()                                    # takes off what arrived meanwhile
            due_before = time.time() - self.interval_hours() * 3600
            due = [wid for wid, it in self._load().items() if (it.get("last_search") or 0) < due_before]
            for n, wid in enumerate(due):
                if n:
                    time.sleep(3)                           # be gentle with the indexers
                try:
                    self.search(wid)
                except Exception as e:  # noqa: BLE001 – one bad book must not stop the others
                    applog.warn(t("Gesucht: Suche fehlgeschlagen: {error}", error=e), "Gesucht")
            return len(due)
        finally:
            self.lock.release()
