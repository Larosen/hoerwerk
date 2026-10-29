"""What is already in the library – so nothing gets downloaded or filed twice.

The index covers the books Better Renamer filed (M4B with ASIN tags) and older
collections: folders of MP3s/M4As/… are recognised by their tags or, failing
that, by the usual "Author/Title" or "Author/Series/Title" folder layout.
"""
from __future__ import annotations

import os
import re
import threading
import time
from typing import Dict, List, Optional

import db as dbm
from audible import _STOP
from usenet import match_words

AUDIO_EXTS = (".m4b", ".m4a", ".mp3", ".mp4", ".aac", ".flac", ".ogg", ".opus", ".wma")
# "CD1", "CD 2", "Disc 3", "Teil 1" … are parts of one book, not books of their own
DISC_RX = re.compile(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)[\s_.-]*\d+\b")


def _surname(name: str) -> str:
    words = match_words(name or "")
    if not words:
        return ""
    # "Fitzek, Sebastian" -> fitzek; "Sebastian Fitzek" -> fitzek
    return words[0] if "," in (name or "") else words[-1]


def title_words(title: str) -> List[str]:
    """Significant words of a title (no articles, no "Band"/"Teil", no series cruft in brackets)."""
    title = re.sub(r"\([^)]*\)|\[[^\]]*\]", " ", title or "")
    words = match_words(title)
    significant = [w for w in words if w not in _STOP and (len(w) > 1 or w.isdigit())]
    return significant or words      # "Das Buch" consists of stop words only


def book_key(authors: List[str], title: str) -> Optional[str]:
    surname = _surname((authors or [""])[0])
    words = title_words(title)
    return f"{surname}|{' '.join(words)}" if surname and words else None


class LibraryIndex:
    def __init__(self, root: str, db: dbm.Database, read_m4b=None, probe=None, notify=None) -> None:
        self.root = os.path.realpath(root) if root else ""
        self.notify = notify or (lambda: None)   # live updates for the browser
        self.db = db
        self._read_m4b = read_m4b          # tagger.read_book
        self._probe = probe                # meta cache probe (tags of mp3 & co.)
        self.entries: Dict[str, dict] = {}  # book folder (relative) -> entry
        self.last_scan: Optional[float] = None
        self.scanning = False
        self.lock = threading.Lock()
        self._load()

    # -- persistence (table "library") ----------------------------------------
    def _load(self) -> None:
        self.entries = {r["path"]: dbm.library_entry(r) for r in self.db.all("SELECT * FROM library")}
        self.last_scan = self.db.get("library_last_scan")

    def _store(self, changed: List[dict], removed: List[str]) -> None:
        with self.db.tx() as c:
            for path in removed:
                c.execute("DELETE FROM library WHERE path=?", (path,))
            for e in changed:
                dbm.save_library_entry(c, e)
            if self.last_scan:
                c.execute("INSERT OR REPLACE INTO kv(key, value) VALUES('library_last_scan', ?)",
                          (dbm.dumps(self.last_scan),))

    # -- scanning ----------------------------------------------------------
    def scan(self) -> int:
        """Walk the library; unchanged folders are taken from the cache."""
        if not self.root or not os.path.isdir(self.root):
            return 0
        with self.lock:
            if self.scanning:
                return len(self.entries)
            self.scanning = True
        self.notify()
        try:
            books: Dict[str, List[str]] = {}       # book folder (relative) -> its audio files
            for folder, dirs, files in os.walk(self.root):
                dirs[:] = sorted(d for d in dirs if not d.startswith((".", "_"))
                                 and not os.path.islink(os.path.join(folder, d)))
                audio = sorted(f for f in files if f.lower().endswith(AUDIO_EXTS) and not f.startswith("."))
                if not audio:
                    continue
                rel = os.path.relpath(folder, self.root)
                book, sub = rel, ""
                if DISC_RX.match(os.path.basename(folder)) and os.path.dirname(rel):
                    book, sub = os.path.dirname(rel), os.path.basename(folder)   # CD1 belongs to its parent
                books.setdefault(book, []).extend(os.path.join(sub, f) if sub else f for f in audio)
            found: Dict[str, dict] = {}
            changed: List[dict] = []
            for rel, audio in books.items():
                folder = os.path.join(self.root, rel)
                try:
                    mtime = max(os.path.getmtime(os.path.join(folder, f)) for f in audio)
                except OSError:
                    continue
                old = self.entries.get(rel)
                if old and old.get("mtime") == mtime and old.get("files") == len(audio):
                    found[rel] = old
                    continue
                found[rel] = self._entry(folder, rel, sorted(audio), mtime)
                changed.append(found[rel])
            with self.lock:
                removed = [p for p in self.entries if p not in found]
                self.entries = found
                self.last_scan = time.time()
                self._store(changed, removed)
            return len(found)
        finally:
            self.scanning = False
            self.notify()

    def _entry(self, folder: str, rel: str, audio: List[str], mtime: float) -> dict:
        book: dict = {}
        m4b = next((f for f in audio if f.lower().endswith((".m4b", ".m4a"))), None)
        try:
            if m4b and self._read_m4b:
                book = self._read_m4b(os.path.join(folder, m4b))
            elif self._probe:
                tags = (self._probe(os.path.join(folder, audio[0])) or {}).get("tags", {})
                book = {"title": tags.get("album") or "", "authors": [a for a in [
                    tags.get("album_artist") or tags.get("artist") or ""] if a],
                    "asin": tags.get("asin") or tags.get("audible_asin")}
        except Exception:  # noqa: BLE001 – one unreadable file must not stop the scan
            book = {}
        parts = rel.split(os.sep)
        # Folder layout as fallback: Author/Title or Author/Series/NN - Title
        title = book.get("title") or re.sub(r"^\s*\d+(?:[.,]\d+)?\s*[-–.]\s*", "", parts[-1])
        authors = book.get("authors") or ([parts[0]] if len(parts) > 1 else [])
        return {
            "path": rel, "files": len(audio), "mtime": mtime,
            "title": title, "authors": authors, "asin": (book.get("asin") or "").upper() or None,
            "series": book.get("series") or (parts[1] if len(parts) > 2 else None),
            "series_position": book.get("series_position"),
            "format": "m4b" if m4b else os.path.splitext(audio[0])[1][1:].lower(),
        }

    def add(self, dest_file: str, book: dict) -> None:
        """Register a freshly filed book without waiting for the next scan."""
        folder = os.path.dirname(dest_file)
        rel = os.path.relpath(folder, self.root)
        entry = {
            "path": rel, "files": 1, "mtime": os.path.getmtime(dest_file),
            "title": book.get("title") or "", "authors": book.get("authors") or [],
            "asin": (book.get("asin") or "").upper() or None, "series": book.get("series"),
            "series_position": book.get("series_position"), "format": "m4b",
        }
        with self.lock:
            self.entries[rel] = entry
            self._store([entry], [])
        self.notify()

    def remove(self, dest_file: str) -> None:
        rel = os.path.relpath(os.path.dirname(dest_file), self.root)
        with self.lock:
            if self.entries.pop(rel, None) is not None:
                self._store([], [rel])
        self.notify()

    # -- lookups -----------------------------------------------------------
    def find(self, book: dict) -> List[dict]:
        """Library entries that are this book: same ASIN, or same author and title."""
        asin = (book.get("asin") or "").upper()
        key = book_key(book.get("authors") or [], book.get("title") or "")
        series = (book.get("series") or "").strip().lower()
        pos = str(book.get("series_position") or "")
        out = []
        with self.lock:
            entries = list(self.entries.values())
        for e in entries:
            if asin and e.get("asin") == asin:
                out.append({**e, "why": "asin"})
            elif key and book_key(e.get("authors") or [], e.get("title") or "") == key:
                out.append({**e, "why": "title"})
            elif (series and pos and (e.get("series") or "").strip().lower() == series
                  and str(e.get("series_position") or "") == pos
                  and _surname((e.get("authors") or [""])[0]) == _surname((book.get("authors") or [""])[0])):
                out.append({**e, "why": "series"})
        return out

    def match_release(self, release: str) -> List[dict]:
        """Library books whose author and every title word appear in a release name."""
        words = set(match_words(release))
        hits = []
        with self.lock:
            entries = list(self.entries.values())
        for e in entries:
            surname = _surname((e.get("authors") or [""])[0])
            tw = title_words(e.get("title") or "")
            if surname and tw and surname in words and all(w in words for w in tw):
                hits.append(e)
        # the most specific title first ("Eragon - Der Auftrag des Ältesten" before "Eragon")
        hits.sort(key=lambda e: -len(title_words(e.get("title") or "")))
        return hits

    def stats(self) -> dict:
        with self.lock:
            n = len(self.entries)
            m4b = sum(1 for e in self.entries.values() if e.get("format") == "m4b")
        return {"books": n, "m4b": m4b, "other": n - m4b, "last_scan": self.last_scan,
                "scanning": self.scanning}
