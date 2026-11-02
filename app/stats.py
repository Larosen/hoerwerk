"""Statistics per family account: who asked for how many books, when, how many hours, how much space,
and what they heard (Audiobookshelf / Plex).

* Admins see every account, can open each one (its requests and what it heard) and the totals.
* Everyone else sees their own numbers and the family's totals – without who asked or heard how much.

Everything comes from the requests table; sizes are read from the book folders in the library (once per
folder and change, cached). Only folders inside the library are measured.
"""
from __future__ import annotations

import os
import threading
import time
from collections import Counter
from datetime import datetime
from typing import Dict, List, Optional, Tuple

import db as dbm
from i18n import t

PERIODS = {"30": 30, "90": 90, "365": 365, "all": None}
MONTHS = 12


class Stats:
    def __init__(self, db: dbm.Database, accounts, index, clock=time.time, listening=None) -> None:
        self.db = db
        self.listening = listening               # what each account heard (Audiobookshelf / Plex) – optional
        self.accounts = accounts
        self.index = index                       # library index (root + entries) – may be None in tests
        self.clock = clock
        self._sizes: Dict[str, Tuple[float, int]] = {}
        self._lock = threading.Lock()

    # -- sizes ---------------------------------------------------------------
    def _folder(self, req_path: str, book: dict) -> Optional[str]:
        root = getattr(self.index, "root", "") or ""
        if not root:
            return None
        candidates = [req_path] if req_path else []
        if self.index and hasattr(self.index, "find"):
            candidates += [os.path.join(root, e["path"]) for e in (self.index.find(book) or [])[:1] if e.get("path")]
        for path in candidates:
            real = os.path.realpath(path)
            if os.path.commonpath([real, root]) == root and real != root and os.path.isdir(real):
                return real
        return None

    def size(self, req_path: str, book: dict) -> int:
        folder = self._folder(req_path, book)
        if not folder:
            return 0
        try:
            mtime = os.stat(folder).st_mtime
        except OSError:
            return 0
        with self._lock:
            hit = self._sizes.get(folder)
        if hit and hit[0] == mtime:
            return hit[1]
        total = 0
        for base, _dirs, files in os.walk(folder):
            for f in files:
                try:
                    total += os.lstat(os.path.join(base, f)).st_size
                except OSError:
                    pass
        with self._lock:
            if len(self._sizes) > 5000:
                self._sizes.clear()
            self._sizes[folder] = (mtime, total)
        return total

    # -- the numbers ------------------------------------------------------------
    @staticmethod
    def _blank(uid, name: str, role: str = "user") -> dict:
        return {"id": uid, "name": name, "role": role, "requests": 0, "books": 0, "available": 0, "open": 0,
                "declined": 0, "failed": 0, "series": 0, "minutes": 0, "minutes_available": 0, "bytes": 0,
                "english": 0, "wait": [], "last": None, "genres": Counter()}

    def summary(self, period: str = "365") -> dict:
        if period not in PERIODS:
            raise ValueError(t("Ungültige Daten"))
        now = self.clock()
        days = PERIODS[period]
        since = now - days * 86400 if days else 0
        users: Dict = {u["id"]: self._blank(u["id"], u["name"], u["role"]) for u in self.accounts.list()}
        total = self._blank(None, "")
        groups: Dict = {}
        month_keys = self._months(now)
        months = {k: {"month": k, "books": 0, "minutes": 0} for k in month_keys}
        per_user_months: Dict = {}
        for row in self.db.all("SELECT user_id, book, status, created, updated, data FROM requests ORDER BY created"):
            book = dbm.loads(row["book"], {}) or {}
            data = dbm.loads(row["data"], {}) or {}
            minutes = int(book.get("runtime_min") or 0)
            key = datetime.fromtimestamp(row["created"]).strftime("%Y-%m")
            if key in months and row["status"] != "declined":
                months[key]["books"] += 1
                months[key]["minutes"] += minutes
                mine = per_user_months.setdefault(row["user_id"], {}).setdefault(key, [0, 0])
                mine[0] += 1
                mine[1] += minutes
            if row["created"] < since:
                continue
            uid = row["user_id"]
            if uid not in users:
                users[uid] = self._blank(uid, t("gelöschtes Konto"))
            for s in (users[uid], total):
                s["books"] += 1
                s["last"] = max(s["last"] or 0, row["created"])
                status = row["status"]
                if status == "available":
                    s["available"] += 1
                    s["minutes_available"] += minutes
                elif status in ("declined", "failed"):
                    s[status] += 1
                else:
                    s["open"] += 1
                if status != "declined":
                    s["minutes"] += minutes
                if (book.get("language") or "").lower() == "english":
                    s["english"] += 1
                for g in (book.get("genres") or [])[:1]:
                    s["genres"][g] += 1
            gid = (data.get("group") or {}).get("id")
            if gid:
                if gid not in groups:
                    groups[gid] = uid
                    users[uid]["series"] += 1
                    users[uid]["requests"] += 1
                    total["series"] += 1
                    total["requests"] += 1
            else:
                users[uid]["requests"] += 1
                total["requests"] += 1
            if row["status"] == "available":
                size = self.size(data.get("path") or "", book)
                wait = max(0.0, row["updated"] - row["created"])
                for s in (users[uid], total):
                    s["bytes"] += size
                    s["wait"].append(wait)
        out = [self._finish(s) for s in users.values()]
        heard_total = {"seconds": 0, "books": 0, "finished": 0, "linked": 0}
        errors: List[str] = []
        for s in out:
            s["listen"] = None
            if self.listening and s["id"] is not None:
                h = self.listening.summary(s["id"], since)
                if h["linked"]:
                    s["listen"] = h
                    heard_total["linked"] += 1
                    for k in ("seconds", "books", "finished"):
                        heard_total[k] += h[k]
                    errors += [e for e in h["errors"] if e not in errors]
        out.sort(key=lambda s: (-s["books"], -s["minutes"], s["name"].lower()))
        lib = getattr(self.index, "entries", None)
        for s in out:
            got = per_user_months.get(s["id"], {})
            s["months"] = [{"month": k, "books": got.get(k, [0, 0])[0], "minutes": got.get(k, [0, 0])[1]} for k in month_keys]
        return {"period": period, "since": since or None, "users": out, "total": self._finish(total),
                "listen": heard_total if self.listening else None, "listen_errors": errors[:5],
                "months": [months[k] for k in month_keys],
                "library_books": len(lib) if isinstance(lib, dict) else None}

    # -- what each one sees -----------------------------------------------------------
    def overview(self, period: str = "365") -> dict:
        """For admins: every account (without the month rows – those are in ``account``)."""
        s = self.summary(period)
        for u in s["users"]:
            u.pop("months", None)
        return s | {"scope": "all"}

    def personal(self, uid: int, period: str = "365") -> dict:
        """For a family member: their own numbers next to everyone's together – nobody else by name."""
        s = self.summary(period)
        me = next((u for u in s["users"] if u["id"] == uid), None) or self._finish(self._blank(uid, ""))
        mine = {m["month"]: m for m in me.pop("months", [])}
        months = [m | {"mine": mine.get(m["month"], {}).get("books", 0)} for m in s["months"]]
        return {"scope": "own", "period": s["period"], "since": s["since"], "me": me, "total": s["total"],
                "accounts": len(s["users"]), "listen": s["listen"], "months": months,
                "library_books": s["library_books"]}

    def account(self, uid: int, period: str = "365") -> Optional[dict]:
        """One account in detail: its numbers, its requests and what it heard – None when there is no such account
        (a deleted account whose requests are still there is found too)."""
        s = self.summary(period)
        user = next((u for u in s["users"] if u["id"] == uid), None)
        if user is None:
            return None
        since = s["since"] or 0
        reqs = []
        for row in self.db.all("SELECT id, book, status, created, updated, data FROM requests WHERE user_id=? "
                               "AND created>=? ORDER BY created DESC LIMIT 500", (uid, since)):
            book = dbm.loads(row["book"], {}) or {}
            data = dbm.loads(row["data"], {}) or {}
            reqs.append({"id": row["id"], "title": book.get("title") or "", "authors": book.get("authors") or [],
                         "series": book.get("series"), "series_position": book.get("series_position"),
                         "cover_url": book.get("cover_url"), "language": book.get("language"),
                         "runtime_min": int(book.get("runtime_min") or 0), "status": row["status"],
                         "created": row["created"], "updated": row["updated"],
                         "group": (data.get("group") or {}).get("title"),
                         "bytes": self.size(data.get("path") or "", book) if row["status"] == "available" else 0})
        heard: List[dict] = []
        if self.listening:
            for b in self.listening.history(uid)["books"]:
                if (b.get("last") or 0) >= since and (b["progress"] >= 0.02 or b["finished"]):
                    heard.append({k: b.get(k) for k in ("title", "author", "progress", "finished", "finished_at",
                                                         "last", "source")})
        return {"period": s["period"], "since": s["since"], "user": user, "months": user.pop("months"),
                "requests": reqs, "heard": heard[:500]}

    @staticmethod
    def _finish(s: dict) -> dict:
        waits = sorted(s.pop("wait"))
        genres = s.pop("genres")
        s["wait_median"] = waits[len(waits) // 2] if waits else None
        s["top_genres"] = [g for g, _ in genres.most_common(3)]
        return s

    @staticmethod
    def _months(now: float) -> List[str]:
        d = datetime.fromtimestamp(now)
        y, m = d.year, d.month
        keys = []
        for _ in range(MONTHS):
            keys.append(f"{y:04d}-{m:02d}")
            m -= 1
            if not m:
                y, m = y - 1, 12
        return keys[::-1]
