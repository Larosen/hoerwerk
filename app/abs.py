"""Audiobookshelf: ``metadata.json`` next to every book, and a library scan after filing.

Audiobookshelf reads ``metadata.json`` in the book folder (its own format) together
with the embedded tags, ``desc.txt``, ``reader.txt`` and ``cover.jpg``. On network
shares (Unraid user shares) its file watcher often misses new books – so after
filing, Better Renamer can ask Audiobookshelf to scan the library (API token).
"""
from __future__ import annotations

import json
import os
import re
import threading
from typing import Callable, List, Optional
from urllib import parse, request
from urllib.error import HTTPError, URLError

import db as dbm

METADATA_FILE = "metadata.json"


def _year(date: Optional[str]) -> Optional[str]:
    m = re.match(r"^(\d{4})", date or "")
    return m.group(1) if m else None


def metadata(book: dict, chapters: List[dict], duration: float = 0.0) -> dict:
    """Audiobookshelf's metadata.json for a book (chapters with start/end in seconds)."""
    series = []
    if book.get("series"):
        pos = book.get("series_position")
        series.append(f"{book['series']} #{pos}" if pos not in (None, "") else book["series"])
    chs = []
    for i, c in enumerate(chapters):
        end = chapters[i + 1]["start"] if i + 1 < len(chapters) else (c.get("end") or duration or c["start"])
        chs.append({"id": i, "start": round(float(c["start"]), 3), "end": round(float(end), 3),
                    "title": c.get("title") or f"Kapitel {i + 1}"})
    return {
        "tags": list(book.get("tags") or []),
        "chapters": chs,
        "title": book.get("title") or "",
        "subtitle": book.get("subtitle") or None,
        "authors": list(book.get("authors") or []),
        "narrators": list(book.get("narrators") or []),
        "series": series,
        "genres": list(book.get("genres") or []),
        "publishedYear": _year(book.get("release_date")),
        "publishedDate": book.get("release_date") or None,
        "publisher": book.get("publisher") or None,
        "description": book.get("description") or None,
        "isbn": book.get("isbn") or None,
        "asin": book.get("asin") or None,
        "language": book.get("language") or None,
        "explicit": bool(book.get("is_adult")),
        "abridged": (book.get("format_type") or "").lower() == "abridged",
    }


def write_metadata(folder: str, book: dict, chapters: List[dict], duration: float = 0.0) -> str:
    path = os.path.join(folder, METADATA_FILE)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(metadata(book, chapters, duration), fh, ensure_ascii=False, indent=2)
        fh.write("\n")
    return path


# --------------------------------------------------------------------------- #
# API: list libraries, scan one
# --------------------------------------------------------------------------- #

class AbsError(Exception):
    pass


class AbsTimeout(AbsError):
    """No answer in time – for a scan that is normal: Audiobookshelf answers only when it is done."""


def _host(url: str) -> str:
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
        "Authorization": f"Bearer {token}", "Accept": "application/json", "User-Agent": "BetterRenamer/4"})
    try:
        with request.build_opener(_NoRedirect).open(req, timeout=timeout) as resp:
            body = resp.read(2 * 1024 * 1024)
    except HTTPError as e:
        if e.code in (401, 403):
            raise AbsError("Audiobookshelf: API-Token ungültig oder ohne Admin-Rechte") from e
        raise AbsError(f"Audiobookshelf: HTTP {e.code}") from e
    except (URLError, OSError) as e:
        reason = getattr(e, "reason", e)
        if isinstance(e, TimeoutError) or isinstance(reason, TimeoutError):
            raise AbsTimeout("Audiobookshelf antwortet nicht rechtzeitig") from e
        raise AbsError(f"Audiobookshelf nicht erreichbar: {reason}") from e
    try:
        return json.loads(body) if body.strip() else {}
    except ValueError:
        return {}


class Audiobookshelf:
    """Connection settings (in the database; the token never goes to the browser)."""
    SECRET = "token"

    def __init__(self, db: dbm.Database, call: Callable[..., dict] = _call, delay: float = 30.0) -> None:
        self.db = db
        self._call = call
        self.delay = delay              # several books in a row: one scan after the last one
        self._timer: Optional[threading.Timer] = None
        self._lock = threading.Lock()
        self.last: Optional[str] = None  # last scan result for the UI
        self.scan_timeout = 10.0

    def _load(self) -> dict:
        data = self.db.get("abs") or {}
        return data if isinstance(data, dict) else {}

    def configured(self) -> bool:
        c = self._load()
        return bool(c.get("url") and c.get(self.SECRET) and c.get("library"))

    def public_config(self) -> dict:
        c = self._load()
        return {"url": c.get("url", ""), "library": c.get("library", ""), "library_name": c.get("library_name", ""),
                "token_set": bool(c.get(self.SECRET)), "configured": self.configured(), "last": self.last}

    def update_config(self, new: dict) -> dict:
        old = self._load()
        url = str(new.get("url") or "").strip()
        keep = old.get(self.SECRET, "") if _host(old.get("url", "")) == _host(url) else ""   # no key to another server
        self.db.put("abs", {"url": url, "library": str(new.get("library") or "").strip(),
                            "library_name": str(new.get("library_name") or old.get("library_name") or ""),
                            self.SECRET: str(new.get(self.SECRET) or "").strip() or keep})
        return self.public_config()

    def libraries(self) -> List[dict]:
        c = self._load()
        if not c.get("url") or not c.get(self.SECRET):
            raise AbsError("Audiobookshelf ist noch nicht eingerichtet")
        data = self._call("GET", f"{_base(c['url'])}/api/libraries", c[self.SECRET])
        libs = data.get("libraries") if isinstance(data, dict) else data
        return [{"id": str(lib.get("id")), "name": str(lib.get("name") or ""), "type": lib.get("mediaType")}
                for lib in libs or [] if isinstance(lib, dict) and lib.get("id")]

    def test(self) -> str:
        """Check the connection; pick the only book library when none is chosen yet."""
        libs = [lib for lib in self.libraries() if lib.get("type") in (None, "book")]
        c = self._load()
        if not c.get("library") and len(libs) == 1:
            self.db.put("abs", {**c, "library": libs[0]["id"], "library_name": libs[0]["name"]})
            return f"verbunden, Bibliothek „{libs[0]['name']}“ gewählt"
        chosen = next((lib for lib in libs if lib["id"] == c.get("library")), None)
        if chosen:
            self.db.put("abs", {**c, "library_name": chosen["name"]})
            return f"verbunden, Bibliothek „{chosen['name']}“"
        names = ", ".join(f"{lib['name']} ({lib['id']})" for lib in libs) or "keine"
        return f"verbunden – bitte Bibliothek wählen: {names}"

    def scan_now(self) -> None:
        c = self._load()
        if not self.configured():
            return
        try:
            # Audiobookshelf answers when the scan is finished – a big library takes minutes
            self._call("POST", f"{_base(c['url'])}/api/libraries/{parse.quote(c['library'])}/scan", c[self.SECRET],
                       timeout=self.scan_timeout)
            self.last = "Scan abgeschlossen"
        except AbsTimeout:
            self.last = "Scan läuft"
        except AbsError as e:
            self.last = str(e)
            print(e, flush=True)

    def request_scan(self) -> None:
        """A book was filed: scan once things have calmed down."""
        if not self.configured():
            return
        with self._lock:
            if self._timer:
                self._timer.cancel()
            self._timer = threading.Timer(self.delay, self.scan_now)
            self._timer.daemon = True
            self._timer.start()
