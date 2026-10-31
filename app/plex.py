"""Plex: rescan the audiobook library after books were filed (like Audiobookshelf).

Plex's own folder watcher often misses changes on network shares (Unraid user
shares) – then a converted book shows up twice: the new M4B *and* the old MP3s,
which are gone on disk but still in Plex. After filing, Hörwerk asks Plex
to refresh the library section; optionally it then empties Plex's trash so the
old, no longer existing entries disappear at once.
"""
from __future__ import annotations

import json
import re
import threading
import time
from typing import Callable, List, Optional
from urllib import parse, request
from urllib.error import HTTPError, URLError

import applog
import db as dbm
from i18n import t


class PlexError(Exception):
    pass


def _origin(url: str) -> str:
    p = parse.urlparse(url if re.match(r"(?i)^https?://", url or "") else "http://" + (url or ""))
    return f"{p.scheme}://{(p.hostname or '').lower()}:{p.port or ''}"


def _base(url: str) -> str:
    url = (url or "").strip().rstrip("/")
    if url and not re.match(r"(?i)^https?://", url):
        url = "http://" + url
    return url


class _NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):   # the token must not travel anywhere else
        return None


def _call(method: str, url: str, token: str, timeout: float = 15) -> dict:
    req = request.Request(url, method=method, headers={
        "X-Plex-Token": token, "Accept": "application/json", "User-Agent": "Hoerwerk/5",
        "X-Plex-Product": "Hoerwerk", "X-Plex-Client-Identifier": "hoerwerk"})
    try:
        with request.build_opener(_NoRedirect).open(req, timeout=timeout) as resp:
            body = resp.read(4 * 1024 * 1024)
    except HTTPError as e:
        if e.code in (401, 403):
            raise PlexError(t("Plex: Token ungültig")) from e
        raise PlexError(f"Plex: HTTP {e.code}") from e
    except (URLError, OSError) as e:
        raise PlexError(t("Plex nicht erreichbar: {reason}", reason=getattr(e, "reason", e))) from e
    try:
        return json.loads(body) if body.strip() else {}
    except ValueError:
        return {}


class Plex:
    """Connection settings (in the database; the token never goes to the browser)."""
    SECRET = "token"

    def __init__(self, db: dbm.Database, call: Callable[..., dict] = _call, delay: float = 30.0,
                 on_error: Optional[Callable[[str], None]] = None, poll: float = 10.0, max_wait: float = 900.0) -> None:
        self.db = db
        self._call = call
        self.delay = delay              # several books in a row: one refresh after the last one
        self.on_error = on_error
        self.poll = poll                # while waiting for the scan to finish (before emptying the trash)
        self.max_wait = max_wait
        self._timer: Optional[threading.Timer] = None
        self._lock = threading.Lock()
        self.last: Optional[str] = None

    def _load(self) -> dict:
        data = self.db.get("plex") or {}
        return data if isinstance(data, dict) else {}

    def configured(self) -> bool:
        c = self._load()
        return bool(c.get("url") and c.get(self.SECRET) and c.get("section"))

    def public_config(self) -> dict:
        c = self._load()
        return {"url": c.get("url", ""), "section": c.get("section", ""), "section_name": c.get("section_name", ""),
                "empty_trash": bool(c.get("empty_trash")), "token_set": bool(c.get(self.SECRET)),
                "configured": self.configured(), "last": self.last}

    def update_config(self, new: dict) -> dict:
        old = self._load()
        url = _base(str(new.get("url") or ""))
        if url:
            try:
                p = parse.urlparse(url)
                p.port  # noqa: B018 – raises ValueError for a broken port
            except ValueError:
                p = None
            if not p or p.scheme not in ("http", "https") or not p.hostname or "@" in p.netloc:
                raise PlexError(t("Ungültige Plex-Adresse"))
            url = f"{p.scheme}://{p.netloc}"      # "http://tower:32400/web/index.html" -> the server
        keep = old.get(self.SECRET, "") if _origin(old.get("url", "")) == _origin(url) else ""   # no token to another server
        section = str(new.get("section") or "").strip()
        self.db.put("plex", {
            "url": url, "section": section,
            "section_name": old.get("section_name", "") if section == old.get("section") else "",
            "empty_trash": bool(new.get("empty_trash")),
            self.SECRET: str(new.get(self.SECRET) or "").strip() or keep})
        return self.public_config()

    def sections(self) -> List[dict]:
        c = self._load()
        if not c.get("url") or not c.get(self.SECRET):
            raise PlexError(t("Plex ist noch nicht eingerichtet"))
        data = self._call("GET", f"{_base(c['url'])}/library/sections", c[self.SECRET])
        dirs = (data.get("MediaContainer") or {}).get("Directory") or []
        return [{"key": str(d.get("key")), "title": str(d.get("title") or ""), "type": d.get("type"),
                 "refreshing": bool(d.get("refreshing")),
                 "paths": [str(loc.get("path")) for loc in d.get("Location") or [] if isinstance(loc, dict)]}
                for d in dirs if isinstance(d, dict) and d.get("key")]

    def test(self) -> str:
        """Check the connection; pick the audiobook library when it is clear which one it is."""
        secs = self.sections()
        c = self._load()
        chosen = next((s for s in secs if s["key"] == c.get("section")), None)
        if not chosen:
            music = [s for s in secs if s["type"] == "artist"]       # Plex keeps audiobooks in music libraries
            named = [s for s in music if re.search(r"(?i)h(ö|oe)rb|audio ?book|book", s["title"] + " ".join(s["paths"]))]
            pick = named if len(named) == 1 else music if len(music) == 1 else []
            if pick:
                chosen = pick[0]
                self.db.put("plex", {**c, "section": chosen["key"], "section_name": chosen["title"]})
                return t("verbunden, Mediathek „{name}“ gewählt", name=chosen["title"])
            names = ", ".join(f"{s['title']} ({s['key']})" for s in secs) or "keine"
            return t("verbunden – bitte Mediathek wählen: {names}", names=names)
        self.db.put("plex", {**c, "section_name": chosen["title"]})
        return t("verbunden, Mediathek „{name}“", name=chosen["title"])

    def scan_now(self) -> None:
        c = self._load()
        if not self.configured():
            return
        base, key, token = _base(c["url"]), parse.quote(c["section"]), c[self.SECRET]
        try:
            self._call("GET", f"{base}/library/sections/{key}/refresh", token)
            self.last = t("Scan angestoßen")
            applog.info(t("Plex-Mediathek wird neu eingelesen"), "Plex")
            if c.get("empty_trash"):
                self._empty_trash_when_done(base, key, token)
        except PlexError as e:
            self.last = str(e)
            applog.warn(str(e), "Plex")
            if self.on_error:
                self.on_error(str(e))

    def _empty_trash_when_done(self, base: str, key: str, token: str) -> None:
        """Wait for the scan, then remove entries whose files are gone (the old MP3s)."""
        end = time.time() + self.max_wait
        time.sleep(min(self.poll, 2.0))
        while time.time() < end:
            secs = {s["key"]: s for s in self.sections()}
            if not secs.get(parse.unquote(key), {}).get("refreshing"):
                self._call("PUT", f"{base}/library/sections/{key}/emptyTrash", token)
                self.last = t("Scan abgeschlossen, Papierkorb geleert")
                applog.info(t("Plex: Papierkorb geleert (entfernte Dateien sind aus der Mediathek verschwunden)"), "Plex")
                return
            time.sleep(self.poll)
        self.last = t("Scan läuft noch – Papierkorb nicht geleert")

    def request_scan(self) -> None:
        """A book was filed: refresh once things have calmed down."""
        if not self.configured():
            return
        with self._lock:
            if self._timer:
                self._timer.cancel()
            self._timer = threading.Timer(self.delay, self.scan_now)
            self._timer.daemon = True
            self._timer.start()
