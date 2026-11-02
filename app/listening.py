"""What the family listens to: Hörwerk accounts linked to Audiobookshelf accounts and Plex profiles.

* Links live in the account's prefs: ``abs_user`` / ``plex_account`` = {"id", "name"}. An Audiobookshelf account
  or Plex profile belongs to at most one Hörwerk account.
* Admins link anyone (Einstellungen → Konten). Everyone can link their own Audiobookshelf account by signing in
  to it once – the password is only used for that check and never stored.
* Audiobookshelf knows progress and listening time per day; Plex only keeps plays that reached the end
  (with one M4B per book: finished books).
* The listened books are the seeds of a personal "Für dich".
"""
from __future__ import annotations

import threading
import time
from datetime import datetime
from typing import Callable, Dict, List, Optional

import applog
from abs import AbsError
from i18n import t
from plex import PlexError

CACHE_SECONDS = 15 * 60
LINK_KEYS = {"abs": "abs_user", "plex": "plex_account"}


class LinkError(ValueError):
    pass


class Listening:
    def __init__(self, accounts, abs, plex, index=None, clock: Callable[[], float] = time.time) -> None:
        self.accounts = accounts
        self.abs = abs
        self.plex = plex
        self.index = index
        self.clock = clock
        self._cache: Dict[tuple, tuple] = {}
        self._lock = threading.Lock()

    # -- which services there are ----------------------------------------------
    def available(self) -> dict:
        abs_c, plex_c = self.abs.public_config(), self.plex.public_config()
        return {"abs": bool(abs_c.get("url") and abs_c.get("token_set")),
                "plex": bool(plex_c.get("url") and plex_c.get("token_set"))}

    # -- links -------------------------------------------------------------------
    def links(self, uid: int) -> dict:
        prefs = self.accounts.prefs(uid)
        return {kind: prefs.get(key) or None for kind, key in LINK_KEYS.items()}

    def all_links(self) -> Dict[int, dict]:
        return {u["id"]: self.links(u["id"]) for u in self.accounts.list()}

    def set_link(self, uid: int, kind: str, target: Optional[dict]) -> dict:
        """Link (or with target None: unlink); the same target is taken away from any other account."""
        if kind not in LINK_KEYS or not self.accounts.get(uid):
            raise LinkError(t("Ungültige Daten"))
        key = LINK_KEYS[kind]
        if target:
            target = {"id": str(target["id"])[:100], "name": str(target.get("name") or "")[:100]}
            for u in self.accounts.list():
                other = self.accounts.prefs(u["id"]).get(key) or {}
                if u["id"] != uid and str(other.get("id")) == target["id"]:
                    self.accounts.set_prefs(u["id"], {key: None})
        self.accounts.set_prefs(uid, {key: target})
        with self._lock:
            self._cache = {k: v for k, v in self._cache.items() if k[1] != uid}
        return self.links(uid)

    def abs_accounts(self) -> List[dict]:
        return self.abs.users()

    def plex_accounts(self) -> List[dict]:
        return self.plex.accounts()

    def link_own_abs(self, uid: int, username: str, password: str) -> dict:
        """Sign in to Audiobookshelf once: proves the account is yours, then it is linked."""
        if not username or not password:
            raise LinkError(t("Benutzername und Passwort fehlen"))
        found = self.abs.check_login(username[:200], password[:500])
        return self.set_link(uid, "abs", found)

    # -- history -------------------------------------------------------------------
    def _cached(self, key: tuple, fn: Callable[[], dict]) -> dict:
        now = self.clock()
        with self._lock:
            hit = self._cache.get(key)
        if hit and hit[0] > now - CACHE_SECONDS:
            return hit[1]
        value = fn()
        with self._lock:
            if len(self._cache) > 200:
                self._cache.clear()
            self._cache[key] = (now, value)
        return value

    def _abs_history(self, abs_id: str) -> dict:
        items = self.abs.library_items()
        stats = self.abs.listening_stats(abs_id)
        books = []
        for p in self.abs.progress(abs_id):
            item = items.get(str(p.get("libraryItemId"))) or {}
            if not item and not p.get("displayTitle"):
                continue
            books.append({"title": item.get("title") or p.get("displayTitle") or "", "author": item.get("author") or "",
                          "asin": item.get("asin"), "progress": float(p.get("progress") or 0),
                          "finished": bool(p.get("isFinished")),
                          "finished_at": (p.get("finishedAt") or 0) / 1000 or None,
                          "last": (p.get("lastUpdate") or 0) / 1000 or None, "source": "abs"})
        days = {d: float(s or 0) for d, s in (stats.get("days") or {}).items() if isinstance(d, str)}
        return {"books": books, "days": days}

    def _plex_history(self, plex_id: str) -> dict:
        plays = self.plex.history(plex_id)
        lengths = self.plex.durations([p["key"] for p in plays])
        days: Dict[str, float] = {}
        books: Dict[str, dict] = {}
        for p in plays:
            seconds = lengths.get(p["key"], 0.0)
            day = datetime.fromtimestamp(p["viewed"]).strftime("%Y-%m-%d")
            days[day] = days.get(day, 0.0) + seconds
            b = books.setdefault(p["book_key"], {"title": p["book"], "author": p["author"], "asin": None, "progress": 1.0,
                                                 "finished": True, "finished_at": p["viewed"], "last": p["viewed"],
                                                 "source": "plex"})
            b["last"] = max(b["last"], p["viewed"])
        return {"books": list(books.values()), "days": days}

    def history(self, uid: int) -> dict:
        """Everything this account heard: {books: [...], days: {date: seconds}, errors: [...], linked: [...]}."""
        links = self.links(uid)
        out: dict = {"books": [], "days": {}, "errors": [], "linked": []}
        for kind, fetch in (("abs", self._abs_history), ("plex", self._plex_history)):
            link = links.get(kind)
            if not link:
                continue
            out["linked"].append(kind)
            try:
                part = self._cached((kind, uid, link["id"]), lambda f=fetch, i=link["id"]: f(i))
            except (AbsError, PlexError) as e:
                out["errors"].append(str(e))
                continue
            except Exception as e:  # noqa: BLE001 – an odd answer must not break the page
                applog.warn(t("Hörverlauf ({source}): {error}", source=kind, error=e), "Hörverlauf")
                out["errors"].append(str(e))
                continue
            out["books"] += part["books"]
            for d, s in part["days"].items():
                out["days"][d] = out["days"].get(d, 0.0) + s
        out["books"].sort(key=lambda b: -(b.get("last") or 0))
        return out

    def summary(self, uid: int, since: float = 0) -> dict:
        """Numbers for the statistics: seconds heard, books heard / finished, last time – since a moment."""
        h = self.history(uid)
        day0 = datetime.fromtimestamp(since).strftime("%Y-%m-%d") if since else ""
        seconds = sum(s for d, s in h["days"].items() if d >= day0)
        heard = [b for b in h["books"] if (b.get("last") or 0) >= since and (b["progress"] >= 0.02 or b["finished"])]
        finished = [b for b in h["books"] if b["finished"] and (b.get("finished_at") or b.get("last") or 0) >= since]
        last = max((b.get("last") or 0 for b in h["books"]), default=0) or None
        return {"linked": h["linked"], "seconds": round(seconds), "books": len(heard), "finished": len(finished),
                "last": last, "errors": h["errors"]}

    def seeds(self, uid: int, limit: int = 25) -> List[dict]:
        """The books this account heard most recently (ASIN from Audiobookshelf or from the library) – for "Für dich"."""
        out: List[dict] = []
        seen = set()
        for b in self.history(uid)["books"]:
            if b["progress"] < 0.15 and not b["finished"]:
                continue                                  # only looked in: says little
            asin = b.get("asin")
            if not asin and self.index and b.get("title"):
                hit = (self.index.find({"title": b["title"], "authors": [b.get("author") or ""]}) or [None])[0]
                asin = (hit or {}).get("asin")
            if asin and asin not in seen:
                seen.add(asin)
                out.append({"asin": asin, "title": b["title"]})
            if len(out) >= limit:
                break
        return out
