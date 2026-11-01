"""Book requests – like Overseerr/Jellyseerr in front of Radarr.

A family member asks for a *book* (from Audible or BookBeat), never for a release:

    pending ──approve──▶ searching ──grab──▶ downloading ──SABnzbd done──▶ processing ──filed──▶ available
       │                    ▲   (nothing found: again every few hours)             │
       └──decline──▶ declined   └──── download failed / import didn't fit: release blocked, next one ◀┘

* Admins and accounts with the right *auto_approve* skip the approval.
* Quota: requests per calendar month (declined or withdrawn ones don't count).
* Each account sees its own requests; *see_all_requests* (and admins) see everyone's.
* Releases are judged by ``releases.evaluate``; the best accepted one is sent to SABnzbd with the ASIN
  (or title and author) and the request id, so the inbox recognises the book and reports back.
"""
from __future__ import annotations

import threading
import time
from datetime import datetime
from typing import Callable, Dict, List, Optional

import applog
import audible
import db as dbm
import releases
from i18n import t
from usenet import UsenetError, match_words

OPEN = ("pending", "searching", "downloading", "processing")
STATUSES = OPEN + ("available", "declined", "failed")
BOOK_KEYS = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_position", "runtime_min",
             "release_date", "cover_url", "url", "source", "region", "isbn", "language", "format_type", "id")
MAX_ATTEMPTS = 5            # releases tried before a request counts as failed
DOWNLOAD_TIMEOUT = 2 * 86400


def book_key(book: dict) -> str:
    if book.get("asin"):
        return "asin:" + str(book["asin"]).upper()
    author = releases.surname((book.get("authors") or [""])[0])
    return "book:" + " ".join(match_words(book.get("title") or "")) + "|" + author


def month_start(now: float) -> float:
    d = datetime.fromtimestamp(now)
    return datetime(d.year, d.month, 1).timestamp()


class RequestError(Exception):
    def __init__(self, message: str, status: int = 400) -> None:
        super().__init__(message)
        self.status = status


class Requests:
    def __init__(self, db: dbm.Database, accounts, usenet, index, notifier, settings: Callable[[], dict],
                 jobs: Callable[[], list] = list, clock=time.time) -> None:
        self.db = db
        self.accounts = accounts
        self.usenet = usenet
        self.index = index
        self.notifier = notifier
        self.settings = settings
        self.jobs = jobs                      # pipeline jobs (to follow a download through the inbox)
        self.clock = clock
        self.lock = threading.RLock()
        self.busy: set = set()                # request ids being searched right now

    # -- storage -------------------------------------------------------------
    def _row(self, row) -> Optional[dict]:
        if row is None:
            return None
        data = dbm.loads(row["data"], {}) or {}
        user = self.accounts.get(row["user_id"]) if row["user_id"] else None
        return {**data, "id": row["id"], "user_id": row["user_id"], "user": user["name"] if user else "–",
                "book": dbm.loads(row["book"], {}), "book_key": row["book_key"], "status": row["status"],
                "created": row["created"], "updated": row["updated"]}

    def get(self, rid: int) -> Optional[dict]:
        return self._row(self.db.one("SELECT * FROM requests WHERE id=?", (rid,)))

    def _save(self, req: dict, status: Optional[str] = None, note: str = "") -> dict:
        status = status or req["status"]
        data = {k: v for k, v in req.items() if k not in ("id", "user_id", "user", "book", "book_key", "status",
                                                          "created", "updated")}
        if note:
            data["history"] = (data.get("history") or [])[-29:] + [{"time": self.clock(), "text": note}]
        self.db.run("UPDATE requests SET status=?, updated=?, data=? WHERE id=?",
                    (status, self.clock(), dbm.dumps(data), req["id"]))
        return self.get(req["id"])

    # -- rights and quota ----------------------------------------------------
    @staticmethod
    def sees_all(user: dict) -> bool:
        return user["role"] == "admin" or bool(user["perms"].get("see_all_requests"))

    @staticmethod
    def may_search(user: dict) -> bool:
        return user["role"] == "admin" or bool(user["perms"].get("interactive_search"))

    def can_see(self, user: dict, req: dict) -> bool:
        return self.sees_all(user) or req["user_id"] == user["id"]

    def quota(self, user: dict) -> dict:
        if user["role"] == "admin":
            return {"limit": None, "used": 0, "left": None}
        limit = user["quota"] if user.get("quota") is not None else int(self.settings().get("request_quota", 5))
        used = self.db.one("SELECT COUNT(*) AS n FROM requests WHERE user_id=? AND created>=? AND status<>'declined'",
                           (user["id"], month_start(self.clock())))["n"]
        return {"limit": limit, "used": used, "left": max(0, limit - used)}

    # -- list ----------------------------------------------------------------
    def list(self, user: dict, everyone: bool = False) -> List[dict]:
        if everyone and self.sees_all(user):
            rows = self.db.all("SELECT * FROM requests ORDER BY created DESC LIMIT 500")
        else:
            rows = self.db.all("SELECT * FROM requests WHERE user_id=? ORDER BY created DESC LIMIT 500", (user["id"],))
        return [self._row(r) for r in rows]

    def status_by_key(self) -> Dict[str, str]:
        """Book key -> status of its newest request (for "requested" marks in Entdecken and the search)."""
        out: Dict[str, str] = {}
        for r in self.db.all("SELECT book_key, status FROM requests ORDER BY created"):
            out[r["book_key"]] = r["status"]
        return out

    def pending_count(self) -> int:
        return self.db.one("SELECT COUNT(*) AS n FROM requests WHERE status='pending'")["n"]

    def open_asins(self) -> set:
        return {r["book_key"][5:] for r in self.db.all("SELECT book_key FROM requests WHERE status IN "
                                                         "('downloading','processing')") if r["book_key"].startswith("asin:")}

    # -- create / decide -----------------------------------------------------
    def _owned(self, book: dict) -> bool:
        if not self.index:
            return False
        asin = (book.get("asin") or "").upper()
        with self.index.lock:
            if asin and any(e.get("asin") == asin for e in self.index.entries.values()):
                return True
        return bool(self.index.find(book))

    def create(self, user: dict, book: dict) -> dict:
        clean = {k: book.get(k) for k in BOOK_KEYS if book.get(k) not in (None, "", [])}
        clean["title"] = str(clean.get("title") or "").strip()[:300]
        clean["authors"] = [str(a)[:120] for a in clean.get("authors") or []][:5]
        if not clean["title"] or not clean["authors"]:
            raise RequestError(t("Titel und Autor fehlen"))
        if clean.get("asin"):
            clean["asin"] = str(clean["asin"]).upper()[:20]
        for k in ("cover_url", "url"):
            if clean.get(k) and not str(clean[k]).startswith("https://"):
                clean.pop(k)
        if self._owned(clean):
            raise RequestError(t("Das Buch ist schon in der Bibliothek"), 409)
        key = book_key(clean)
        now = self.clock()
        auto = user["role"] == "admin" or bool(user["perms"].get("auto_approve"))
        with self.lock, self.db.tx() as c:
            other = c.execute("SELECT user_id FROM requests WHERE book_key=? AND status IN (?,?,?,?)", (key, *OPEN)).fetchone()
            if other:
                raise RequestError(t("Das Buch ist schon angefragt"), 409)
            q = self.quota(user)
            if q["limit"] is not None and q["left"] <= 0:
                raise RequestError(t("Kontingent für diesen Monat aufgebraucht ({n} Anfragen)", n=q["limit"]), 429)
            status = "searching" if auto else "pending"
            data = {"history": [{"time": now, "text": t("Angefragt von {name}", name=user["name"])}], "blocked": [],
                    "attempts": 0}
            cur = c.execute("INSERT INTO requests(user_id, book_key, book, status, created, updated, data) VALUES(?,?,?,?,?,?,?)",
                            (user["id"], key, dbm.dumps(clean), status, now, now, dbm.dumps(data)))
            rid = cur.lastrowid
        req = self.get(rid)
        applog.info(t("Anfrage: „{title}“ von {name}", title=clean["title"], name=user["name"]), "Anfragen")
        if status == "pending":
            self.notifier.send("request_new", t("Neue Buchanfrage"), f"{user['name']}: {self._label(req)}")
        else:
            self.search_soon(rid)
        return req

    def approve(self, rid: int, admin: dict) -> dict:
        req = self._need(rid)
        if req["status"] != "pending":
            raise RequestError(t("Die Anfrage wartet nicht auf Freigabe"), 409)
        req["decided_by"] = admin["name"]
        req = self._save(req, "searching", t("Freigegeben von {name}", name=admin["name"]))
        self.search_soon(rid)
        return req

    def decline(self, rid: int, admin: dict, reason: str = "") -> dict:
        req = self._need(rid)
        if req["status"] not in ("pending", "searching", "failed"):
            raise RequestError(t("Die Anfrage läuft schon – erst zurückziehen"), 409)
        req["decided_by"], req["reason"] = admin["name"], (reason or "").strip()[:300]
        req = self._save(req, "declined", t("Abgelehnt von {name}", name=admin["name"]) + (f": {req['reason']}" if req["reason"] else ""))
        self._tell(req, t("Anfrage abgelehnt"), self._label(req) + (f"\n{req['reason']}" if req["reason"] else ""))
        return req

    def withdraw(self, rid: int, user: dict) -> None:
        req = self._need(rid)
        if user["role"] != "admin":
            if req["user_id"] != user["id"]:
                raise RequestError(t("Nicht gefunden"), 404)
            if req["status"] not in ("pending", "searching", "failed", "declined"):
                raise RequestError(t("Der Download läuft schon – zurückziehen geht nicht mehr"), 409)
        self.db.run("DELETE FROM requests WHERE id=?", (rid,))

    def retry(self, rid: int) -> dict:
        req = self._need(rid)
        if req["status"] not in ("failed", "searching"):
            raise RequestError(t("Die Anfrage läuft gerade"), 409)
        req.update(error=None, attempts=0, last_search=None)
        req = self._save(req, "searching", t("Neuer Versuch"))
        self.search_soon(rid)
        return req

    def _need(self, rid: int) -> dict:
        req = self.get(rid)
        if not req:
            raise RequestError(t("Nicht gefunden"), 404)
        return req

    # -- searching and grabbing --------------------------------------------
    def _queries(self, book: dict, query: Optional[str] = None) -> tuple:
        author = releases.surname((book.get("authors") or [""])[0])
        if query:
            return query, []
        extra = [book["title"]]
        if book.get("series") and book.get("series_position"):
            extra.append(f"{book['series']} {book['series_position']}")
        return f"{author} {book['title']}".strip(), extra

    def _judge(self, req: dict, results: List[dict]) -> List[dict]:
        s = self.settings()
        lang = "de" if (req["book"].get("language") or "german").lower() in ("german", "deutsch", "de") else ""
        return releases.rank(results, req["book"], prefer_language=lang,
                             unabridged=bool(s.get("discover_unabridged", True)), blocked=req.get("blocked") or [])

    def releases(self, rid: int, query: Optional[str] = None) -> dict:
        """All releases for a request with the verdict on each (interactive search)."""
        req = self._need(rid)
        if not self.usenet.configured()["indexers"]:
            raise RequestError(t("Noch kein Indexer eingerichtet"))
        q, extra = self._queries(req["book"], (query or "").strip()[:200] or None)
        try:
            res = self.usenet.search(q, "", extra)
        except UsenetError as e:
            raise RequestError(str(e), 502)
        ranked = self._judge(req, res["results"])
        return {"request": req, "query": q, "queries": res.get("queries") or [q], "errors": res.get("errors") or [],
                "results": ranked}

    def search_soon(self, rid: int) -> None:
        threading.Thread(target=self._search_safe, args=(rid,), daemon=True, name=f"request-{rid}").start()

    def _search_safe(self, rid: int) -> None:
        try:
            self.search(rid)
        except Exception as e:  # noqa: BLE001 – logged, the next round tries again
            applog.warn(t("Anfrage {id}: Suche fehlgeschlagen: {error}", id=rid, error=e), "Anfragen")

    def search(self, rid: int) -> Optional[dict]:
        """Search the indexers; the best accepted release goes to SABnzbd right away."""
        with self.lock:
            if rid in self.busy:
                return None
            self.busy.add(rid)
        try:
            req = self.get(rid)
            if not req or req["status"] != "searching":
                return req
            if self._owned(req["book"]):
                return self._save(req, "available", t("Schon in der Bibliothek"))
            if not self.usenet.configured()["indexers"]:
                req.update(last_search=self.clock(), error=t("Noch kein Indexer eingerichtet"))
                return self._save(req)
            q, extra = self._queries(req["book"])
            try:
                res = self.usenet.search(q, "", extra)
            except UsenetError as e:
                req.update(last_search=self.clock(), error=str(e))
                return self._save(req)
            ranked = self._judge(req, res["results"])
            good = [r for r in ranked if r["verdict"]["ok"]]
            req.update(last_search=self.clock(), error="; ".join(res.get("errors") or []) or None,
                       found=len(good), seen=len(ranked))
            if not good:
                return self._save(req, note=t("Nichts Passendes gefunden ({n} Releases geprüft) – neuer Versuch in {h} h",
                                              n=len(ranked), h=self.interval_hours()))
            if not self.usenet.configured()["sab"]:
                req["error"] = t("SABnzbd ist nicht eingerichtet")
                return self._save(req, note=t("{n} passende Releases gefunden", n=len(good)))
            return self.grab(rid, good[0]["id"], by=None, release=good[0])
        finally:
            with self.lock:
                self.busy.discard(rid)

    def grab(self, rid: int, release_id: str, by: Optional[dict] = None, release: Optional[dict] = None) -> dict:
        req = self._need(rid)
        if req["status"] in ("downloading", "processing", "available"):
            raise RequestError(t("Für diese Anfrage läuft schon ein Download"), 409)
        book = req["book"]
        hints = {"title": book["title"], "author": ", ".join(book.get("authors") or []), "asin": book.get("asin"),
                 "request_id": rid}
        vol = releases._volumes(book)
        name = (release or {}).get("title") or ""
        if name and vol is not None and releases.packs.pack_info(name):
            hints["volumes"] = [vol]
        try:
            sent = self.usenet.send(release_id, hints)
        except UsenetError as e:
            raise RequestError(str(e), 400)
        req.update(release=sent["title"], nzo_id=sent["nzo_id"], sent_at=self.clock(), error=None,
                   attempts=int(req.get("attempts") or 0) + 1)
        who = t(" (von Hand: {name})", name=by["name"]) if by else ""
        req = self._save(req, "downloading", t("Lädt: {release}", release=sent["title"]) + who)
        applog.info(t("Anfrage „{title}“: {release} an SABnzbd", title=book["title"], release=sent["title"]), "Anfragen")
        return req

    def block(self, rid: int, release_title: str) -> dict:
        req = self._need(rid)
        blocked = req.get("blocked") or []
        if release_title and release_title not in blocked:
            req["blocked"] = (blocked + [release_title[:300]])[-50:]
        return self._save(req, note=t("Release gesperrt: {release}", release=release_title))

    def _failed_release(self, req: dict, why: str) -> dict:
        """Download or import went wrong: block that release and try the next one (or give up)."""
        if req.get("release"):
            req["blocked"] = ((req.get("blocked") or []) + [req["release"]])[-50:]
        req.update(nzo_id=None, job_id=None, last_search=None)
        if int(req.get("attempts") or 0) >= MAX_ATTEMPTS:
            req["error"] = why
            req = self._save(req, "failed", t("Aufgegeben nach {n} Versuchen: {why}", n=req["attempts"], why=why))
            self.notifier.send("request_failed", t("Buchanfrage fehlgeschlagen"), f"{self._label(req)}\n{why}")
            self._tell(req, t("Anfrage fehlgeschlagen"), self._label(req))
            return req
        req = self._save(req, "searching", t("{why} – nächstes Release", why=why))
        self.search_soon(req["id"])
        return req

    # -- following the download --------------------------------------------
    def interval_hours(self) -> int:
        return max(1, int(self.settings().get("wanted_interval_hours") or 12))

    def tick(self) -> None:
        """Every few minutes: search again when due, follow downloads and imports, notice arrivals."""
        rows = self.db.all("SELECT id FROM requests WHERE status IN (?,?,?,?)", OPEN)
        if not rows:
            return
        activity = None
        for (rid,) in [tuple(r) for r in rows]:
            req = self.get(rid)
            if not req:
                continue
            try:
                if req["status"] != "pending" and self._owned(req["book"]):
                    self._arrived(req, "")
                    continue
                if req["status"] == "searching":
                    if (req.get("last_search") or 0) < self.clock() - self.interval_hours() * 3600:
                        self.search(rid)
                elif req["status"] == "downloading":
                    if activity is None:
                        activity = self._activity()
                    self._follow_download(req, activity)
                elif req["status"] == "processing":
                    self._follow_job(req)
            except Exception as e:  # noqa: BLE001 – one request must not stop the others
                applog.warn(t("Anfrage {id}: {error}", id=rid, error=e), "Anfragen")

    def _activity(self) -> List[dict]:
        try:
            return self.usenet.activity().get("items", []) if self.usenet.configured()["sab"] else []
        except UsenetError:
            return []

    def _follow_download(self, req: dict, items: List[dict]) -> None:
        item = next((i for i in items if i.get("id") and i.get("id") == req.get("nzo_id")), None)
        if item and item.get("done"):
            if item.get("status") == "Failed":
                self._failed_release(req, t("Download fehlgeschlagen: {error}", error=item.get("error") or "SABnzbd"))
            else:
                self._save(req, "processing", t("Download fertig – wird verarbeitet"))
        elif not item and self.clock() - (req.get("sent_at") or 0) > DOWNLOAD_TIMEOUT:
            self._failed_release(req, t("Download nach 2 Tagen nicht in SABnzbd zu finden"))
        else:
            self._follow_job(req)                      # the inbox may be quicker than SABnzbd's history

    def _follow_job(self, req: dict, job=None) -> None:
        job = job or next((j for j in reversed(self.jobs()) if (j.hints or {}).get("request_id") == req["id"]), None)
        if not job:
            return
        if job.status == "done":
            self._arrived(req, job.dest or "")
        elif job.status == "failed":
            self._failed_release(req, t("Import fehlgeschlagen: {error}", error=job.error or ""))
        elif job.status == "review" and not req.get("review_noted"):
            req["review_noted"] = True
            self._save(req, "processing", t("Wartet auf Prüfung durch einen Admin"))
        elif req["status"] == "downloading":
            self._save(req, "processing", t("Download fertig – wird verarbeitet"))

    def on_job(self, job) -> None:
        """Called by the pipeline when a job is linked to a request, finishes, fails or waits for review.

        Also for books that simply turned up in the inbox (not downloaded by Hörwerk): once the
        recognition linked them to a request, the request follows them like its own download."""
        rid = (job.hints or {}).get("request_id")
        if not rid:
            return
        req = self.get(int(rid))
        if not req or req["status"] not in OPEN:
            return
        if job.status in ("done", "failed", "review"):
            if req["status"] != "processing":
                req = self._save(req, "processing", t("Im Eingang gefunden: {name}", name=getattr(job, "name", "")))
            self._follow_job(req, job)
        elif req["status"] != "processing":
            self._save(req, "processing", t("Im Eingang gefunden: {name}", name=getattr(job, "name", "")))

    def open_for_matching(self) -> List[dict]:
        """Open requests for the recognition in the inbox: [{id, user, key, book}]."""
        out = []
        for r in self.db.all("SELECT * FROM requests WHERE status IN (?,?,?,?)", OPEN):
            user = self.accounts.get(r["user_id"]) if r["user_id"] else None
            out.append({"id": r["id"], "user": user["name"] if user else "–", "key": r["book_key"],
                        "book": dbm.loads(r["book"], {})})
        return out

    def _arrived(self, req: dict, dest: str) -> None:
        req["path"] = dest
        req = self._save(req, "available", t("Verfügbar in der Bibliothek"))
        applog.info(t("Anfrage erfüllt: „{title}“", title=req["book"]["title"]), "Anfragen")
        self._tell(req, t("Dein Hörbuch ist da"), self._label(req))

    # -- notifications -----------------------------------------------------
    @staticmethod
    def _label(req: dict) -> str:
        b = req["book"]
        return f"{', '.join(b.get('authors') or [])} – {b.get('title')}"

    def _tell(self, req: dict, title: str, message: str) -> None:
        """The requester's own ntfy topic (set in their account), if any."""
        user = self.accounts.get(req["user_id"]) if req.get("user_id") else None
        topic = (self.accounts.prefs(user["id"]).get("ntfy_topic") if user else "") or ""
        if topic:
            self.notifier.send_to(topic, title, message)

    # -- old "Gesucht" list ----------------------------------------------------
    def import_wanted(self) -> int:
        """The wanted list of version 5.3 becomes requests of the first admin (once)."""
        items = self.db.get("wanted")
        if not isinstance(items, dict) or not items:
            return 0
        admin = next((u for u in self.accounts.list() if u["role"] == "admin"), None)
        if not admin:
            return 0
        n = 0
        for item in items.values():
            try:
                self.create(admin, item)
                n += 1
            except RequestError:
                pass
        self.db.run("DELETE FROM kv WHERE key='wanted'")
        return n


def summary(req: dict, user: dict, sees_all: bool) -> dict:
    """What the browser gets: no blocked-release internals for family accounts."""
    out = {k: req.get(k) for k in ("id", "book", "status", "created", "updated", "reason", "release", "error",
                                    "last_search", "found", "seen", "history", "path", "attempts")}
    out["mine"] = req["user_id"] == user["id"]
    if sees_all:
        out["user"] = req["user"]
    if user["role"] == "admin":
        out["blocked"] = req.get("blocked") or []
    return out


__all__ = ["Requests", "RequestError", "book_key", "summary", "STATUSES", "OPEN", "audible"]
