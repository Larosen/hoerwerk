"""What is already in the library – so nothing gets downloaded or filed twice.

The index covers the books Hörwerk filed (M4B with ASIN tags) and older
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

COVER_FILES = ("cover.jpg", "cover.jpeg", "cover.png", "folder.jpg", "folder.png", "front.jpg", "front.png")
AUDIO_EXTS = (".m4b", ".m4a", ".mp3", ".mp4", ".aac", ".flac", ".ogg", ".opus", ".wma")
# "CD1", "CD 2", "Disc 3", "Teil 1" … are parts of one book, not books of their own
DISC_RX = re.compile(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)[\s_.-]*\d+\b")
# "Sebastian Fitzek, Simon Jäger", "Autor / Sprecher", "A & B", "A und B", "A feat. B"
NAME_SPLIT = re.compile(r"\s*(?:[;/&|+]|,(?=\s*\S+\s+\S)|\bund\b|\band\b|\bfeat\.?|\bgelesen von\b|\bread by\b)\s*", re.I)
ENTRY_VERSION = 5       # bump when the way entries are read changes: cached folders are read again


def _surname(name: str) -> str:
    words = match_words(name or "")
    if not words:
        return ""
    # "Storm, Theodor" -> storm; "Theodor Storm" -> storm
    return words[0] if "," in (name or "") else words[-1]


def title_words(title: str) -> List[str]:
    """Significant words of a title (no articles, no "Band"/"Teil", no series cruft in brackets)."""
    title = re.sub(r"\([^)]*\)|\[[^\]]*\]", " ", title or "")
    words = match_words(title)
    significant = [w for w in words if w not in _STOP and (len(w) > 1 or w.isdigit())]
    return significant or words      # "Das Buch" consists of stop words only


def names_of(*values: str) -> List[str]:
    """Every name in author-like tags: the whole value and its parts ("A, B" -> "A, B", "A", "B")."""
    out: List[str] = []
    for v in values:
        v = (v or "").strip()
        if not v:
            continue
        for n in [v, *NAME_SPLIT.split(v)]:
            n = n.strip(" ,")
            if n and n not in out:
                out.append(n)
    return out


def surnames(names: List[str]) -> set:
    return {s for s in (_surname(n) for n in names) if s}


def split_author_title(name: str) -> Optional[tuple]:
    """Folder names like "Storm, Theodor - Der Schimmelreiter" -> (author, title)."""
    m = re.match(r"^\s*(.+?)\s+[-–]\s+(.+?)\s*$", name or "")
    if not m or re.match(r"^\d+$", m.group(1)):      # "01 - Title" is a volume number, not an author
        return None
    return m.group(1), m.group(2)


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
        # changes by add()/remove() while a scan walks the disk: the scan's picture is older than them, so they are
        # applied on top of it at the end instead of being overwritten (path -> entry, None = removed)
        self._during: Optional[Dict[str, Optional[dict]]] = None
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
            self._during = {}
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
                if old and old.get("mtime") == mtime and old.get("files") == len(audio) and old.get("v") == ENTRY_VERSION:
                    found[rel] = old
                    continue
                found[rel] = self._entry(folder, rel, sorted(audio), mtime)
                changed.append(found[rel])
            with self.lock:
                for rel, entry in (self._during or {}).items():      # newer than what the walk saw
                    if entry is None:
                        found.pop(rel, None)
                    else:
                        found[rel] = entry
                changed = [c for c in changed if c["path"] not in (self._during or {})]
                removed = [p for p in self.entries if p not in found]
                self.entries = found
                self.last_scan = time.time()
                self._store(changed, removed)
            return len(found)
        finally:
            with self.lock:
                self.scanning = False
                self._during = None
            self.notify()

    def _entry(self, folder: str, rel: str, audio: List[str], mtime: float) -> dict:
        book: dict = {}
        cover = any(os.path.isfile(os.path.join(folder, n)) and not os.path.islink(os.path.join(folder, n))
                    for n in COVER_FILES)
        names: List[str] = []           # every name that may be the author: tags (also narrators) + folders
        m4b = next((f for f in audio if f.lower().endswith((".m4b", ".m4a"))), None)
        try:
            if m4b and self._read_m4b:
                book = self._read_m4b(os.path.join(folder, m4b))
                names = names_of(*(book.get("authors") or []))
            elif self._probe:
                info = self._probe(os.path.join(folder, audio[0])) or {}
                cover = cover or bool(info.get("cover_codec"))
                tags = info.get("tags", {})
                names = names_of(tags.get("album_artist", ""), tags.get("artist", ""), tags.get("composer", ""))
                book = {"title": tags.get("album") or "", "authors": names[:1],
                        "asin": tags.get("asin") or tags.get("audible_asin")}
        except Exception:  # noqa: BLE001 – one unreadable file must not stop the scan
            book = {}
        parts = rel.split(os.sep)
        # Folder layout: Author/Title, Author/Series/NN - Title, or "Author - Title" as one folder
        folder_author = parts[0] if len(parts) > 1 else None
        folder_title = re.sub(r"^\s*\d+(?:[.,]\d+)?\s*[-–.]\s*", "", parts[-1])
        if not folder_author and (pair := split_author_title(parts[-1])):
            folder_author, folder_title = pair
        if folder_author:
            names = [*names, *[n for n in names_of(folder_author) if n not in names]]
        title = book.get("title") or folder_title
        # Album tags like "Theodor Storm - Der Schimmelreiter": the author is not part of the title
        pair = split_author_title(title)
        if pair and surnames(names_of(pair[0])) & surnames(names):
            title = pair[1]
        # M4B: our own tags. MP3 & co.: the artist tag is often the narrator – a folder named after the author wins
        authors = ([folder_author] if folder_author and not m4b else book.get("authors")) or (
            [folder_author] if folder_author else [])
        return {
            "v": ENTRY_VERSION, "path": rel, "files": len(audio), "mtime": mtime, "names": names,
            "cover": cover or bool(book.get("_cover")),
            "title": title, "authors": authors, "asin": (book.get("asin") or "").upper() or None,
            "series": book.get("series") or (parts[1] if len(parts) > 2 else None),
            # Author/Series/03 - Title: the number in front is the volume
            "series_position": book.get("series_position") or (
                m.group(1) if len(parts) > 2 and (m := re.match(r"^\s*(\d{1,3})(?:[.,]\d+)?\s*[-–.]", parts[-1])) else None),
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
            "v": ENTRY_VERSION, "names": names_of(*(book.get("authors") or [])),
            "cover": any(os.path.isfile(os.path.join(folder, n)) for n in COVER_FILES)
            or bool(self._read_m4b and self._read_m4b(dest_file).get("_cover")),
        }
        with self.lock:
            self.entries[rel] = entry
            if self._during is not None:
                self._during[rel] = entry
            self._store([entry], [])
        self.notify()

    def remove(self, dest_file: str) -> None:
        rel = os.path.relpath(os.path.dirname(dest_file), self.root)
        with self.lock:
            if self._during is not None:
                self._during[rel] = None
            if self.entries.pop(rel, None) is not None:
                self._store([], [rel])
        self.notify()

    # -- lookups -----------------------------------------------------------
    @staticmethod
    def _names(e: dict) -> set:
        return surnames([*(e.get("authors") or []), *(e.get("names") or [])])

    def find(self, book: dict) -> List[dict]:
        """Library entries that are this book: same ASIN, same author and title, or same series and volume."""
        asin = (book.get("asin") or "").upper()
        want = surnames(names_of(*(book.get("authors") or [])))
        words = title_words(book.get("title") or "")
        series = (book.get("series") or "").strip().lower()
        pos = str(book.get("series_position") or "")
        out = []
        with self.lock:
            entries = list(self.entries.values())
        for e in entries:
            if asin and e.get("asin") == asin:
                out.append({**e, "why": "asin"})
                continue
            names = self._names(e)
            if words and want & names and title_words(e.get("title") or "") == words:
                out.append({**e, "why": "title"})
            elif (series and pos and want & names and (e.get("series") or "").strip().lower() == series
                  and str(e.get("series_position") or "") == pos):
                out.append({**e, "why": "series"})
        return out

    def match_release(self, release: str, lenient: bool = False) -> List[dict]:
        """Library books whose every title word appears in a release name (or a search term).

        Normally the author's surname must be there as well; titles of two or more significant
        words also count without it ("Toedliche.Saat.German.Hoerbuch"). ``lenient`` (for what
        the user typed) accepts one-word titles without the author too."""
        words = set(match_words(release))
        hits = []
        with self.lock:
            entries = list(self.entries.values())
        for e in entries:
            tw = title_words(e.get("title") or "")
            if not tw or not all(w in words for w in tw):
                continue
            if self._names(e) & words:
                hits.append({**e, "why": "title"})
            elif lenient or len(tw) >= 2:
                hits.append({**e, "why": "similar"})
        # author found first, then the most specific title ("Eragon - Der Auftrag des Ältesten" before "Eragon")
        hits.sort(key=lambda e: (e["why"] != "title", -len(title_words(e.get("title") or ""))))
        return hits

    def series_owned(self, series: str, author: str = "") -> Dict[int, str]:
        """Volumes of a series that are in the library: {volume: folder}."""
        def norm(s: str) -> str:
            return " ".join(w for w in title_words(s) if w not in ("reihe", "serie", "series", "saga", "trilogie"))
        want = norm(series)
        surnames_wanted = surnames(names_of(author)) if author else set()
        out: Dict[int, str] = {}
        if not want:
            return out
        with self.lock:
            entries = list(self.entries.values())
        for e in entries:
            if norm(e.get("series") or "") != want:
                continue
            if surnames_wanted and not (surnames_wanted & self._names(e)):
                continue
            m = re.match(r"^\s*(\d{1,3})", str(e.get("series_position") or ""))
            if m:
                out.setdefault(int(m.group(1)), e["path"])
        return out

    def books(self, query: str = "", offset: int = 0, limit: int = 100) -> dict:
        """Browse the library: every word of the query in title, author, series or path."""
        want = match_words(query)
        with self.lock:
            entries = list(self.entries.values())
        if want:
            def text(e: dict) -> set:
                return set(match_words(" ".join([e.get("title") or "", " ".join(e.get("authors") or []),
                                                 " ".join(e.get("names") or []), e.get("series") or "", e["path"]])))
            entries = [e for e in entries if all(any(t.startswith(w) for t in text(e)) for w in want)]
        entries.sort(key=lambda e: [w.lower() for w in re.split(r"(\d+)", e["path"])])
        keys = ("path", "title", "authors", "names", "series", "series_position", "asin", "format", "files", "mtime", "cover")
        return {"total": len(entries), "offset": offset,
                "books": [{k: e.get(k) for k in keys} for e in entries[offset:offset + limit]]}

    def stats(self) -> dict:
        with self.lock:
            n = len(self.entries)
            m4b = sum(1 for e in self.entries.values() if e.get("format") == "m4b")
        return {"books": n, "m4b": m4b, "other": n - m4b, "last_scan": self.last_scan,
                "scanning": self.scanning}
