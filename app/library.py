"""Where a finished audiobook goes: path templates with optional parts.

Template syntax: ``{field}`` or ``{field:2}`` (zero-pad numbers) and
``[ … ]`` for optional parts that vanish when a field inside is empty, e.g.

    {author}/[{series}/][{series_position:2} - ]{title}

gives ``Hoffmann/Serapionsbrüder/01 - Das Fräulein`` or ``Storm/Der Schimmelreiter``.
"""
from __future__ import annotations

import errno
import os
import re
import shutil
import unicodedata
from typing import Dict, List, Optional, Tuple
from i18n import t

DEFAULT_FOLDER = "{author}/[{series}/][{series_position:2} - ]{title}"
OLD_DEFAULT_FOLDER = "{author}/[{series}/][{series_position} - ]{title}"   # v3.0
DEFAULT_FILE = "{title}"
FIELDS = ("author", "authors", "narrator", "narrators", "series", "series_position", "title",
          "subtitle", "year", "asin", "publisher", "language", "genre", "region")
_FIELD_RX = re.compile(r"\{(\w+)(?::(\d+))?\}")


def fields(book: dict, title: Optional[str] = None) -> Dict[str, str]:
    authors = book.get("authors") or []
    narrators = book.get("narrators") or []
    return {
        "author": authors[0] if authors else t("Unbekannt"),
        "authors": ", ".join(authors) or t("Unbekannt"),
        "narrator": narrators[0] if narrators else "",
        "narrators": ", ".join(narrators),
        "series": book.get("series") or "",
        "series_position": book.get("series_position") or "",
        "title": title or book.get("title") or t("Unbenannt"),
        "subtitle": book.get("subtitle") or "",
        "year": (book.get("release_date") or "")[:4],
        "asin": book.get("asin") or "",
        "publisher": book.get("publisher") or "",
        "language": book.get("language") or "",
        "genre": (book.get("genres") or [""])[0],
        "region": book.get("region") or "",
    }


def _pad(value: str, width: Optional[str]) -> str:
    if not width or not value:
        return value
    return re.sub(r"(?<![.\d])\d+", lambda m: m.group(0).zfill(int(width)), value)


def sanitize(part: str) -> str:
    """One path component, safe for SMB/Windows clients and Unraid shares."""
    part = re.sub(r'[*?"<>|\x00-\x1f]', "", part)
    part = re.sub(r"\s*:\s*", " - ", part)
    part = re.sub(r"[\\/]", "-", part)
    part = re.sub(r"\s+", " ", part).replace(" - - ", " - ").strip()
    part = part.strip(" .")
    while len(part.encode("utf-8")) > 180:
        part = part[:-1]
    return part.rstrip(" .") or "_"


def render(template: str, values: Dict[str, str]) -> str:
    """Expand placeholders; ``[...]`` blocks disappear if any field in them is empty."""
    def expand(text: str) -> Tuple[str, bool]:
        empty = False

        def sub(m: re.Match) -> str:
            nonlocal empty
            v = values.get(m.group(1), "")
            if not v:
                empty = True
            return _pad(v, m.group(2)).replace("/", "\x00")  # "/" inside a value is no folder
        return _FIELD_RX.sub(sub, text), empty

    out, i = [], 0
    for m in re.finditer(r"\[([^\[\]]*)\]", template):
        out.append(expand(template[i:m.start()])[0])
        text, empty = expand(m.group(1))
        if not empty:
            out.append(text)
        i = m.end()
    out.append(expand(template[i:])[0])
    path = "".join(out)
    parts = [sanitize(p.replace("\x00", "-")) for p in path.split("/") if p.strip()]
    return "/".join(parts)


def destination(root: str, book: dict, folder_tpl: str, file_tpl: str,
                title: Optional[str] = None) -> Tuple[str, str]:
    values = fields(book, title)
    folder = render(folder_tpl or DEFAULT_FOLDER, values) or sanitize(values["title"])
    name = render(file_tpl or DEFAULT_FILE, values).replace("/", "-") or t("Hörbuch")
    return os.path.join(root, folder), name + ".m4b"


def folder_keys(name: str) -> Tuple[str, str]:
    """Spelling-insensitive keys for a folder name.

    "E.T.A. Hoffmann", "E. T. A. Hoffmann" and "e t a hoffmann" share the first key;
    "Hoffmann, E. T. A." shares the second (word order ignored). Umlauts count as
    ae/oe/ue, accents and punctuation are ignored, "01" equals "1".
    """
    s = name.casefold().replace("ß", "ss").replace("ä", "ae").replace("ö", "oe").replace("ü", "ue")
    s = "".join(c for c in unicodedata.normalize("NFKD", s) if not unicodedata.combining(c))
    words = [str(int(w)) if w.isdecimal() else w for w in re.findall(r"[^\W_]+", s)]
    return "".join(words), " ".join(sorted(words))


def reuse_existing(root: str, folder: str) -> str:
    """Walk ``folder`` below ``root`` and swap in folders that already exist under another spelling.

    So a new book by "E.T.A. Hoffmann" lands in an existing "E. T. A. Hoffmann" folder, and
    "Die Serapionsbrüder" finds "die serapionsbrüder" – instead of creating a second one.
    """
    rel = os.path.relpath(folder, root)
    if rel.startswith(".."):
        return folder
    current = root
    for part in rel.split(os.sep):
        target = os.path.join(current, part)
        if not os.path.isdir(target):
            keys = folder_keys(part)
            try:
                existing = sorted(d for d in os.listdir(current)
                                  if not d.startswith((".", "_")) and os.path.isdir(os.path.join(current, d)))
            except OSError:
                existing = []
            match = next((d for d in existing if folder_keys(d)[0] == keys[0]), None) \
                or next((d for d in existing if folder_keys(d)[1] == keys[1]), None)
            if match:
                target = os.path.join(current, match)
        current = target
    return current


def unique_folder(path: str) -> str:
    """``Titel`` -> ``Titel (2)`` if the folder already holds an audiobook."""
    def taken(p: str) -> bool:
        try:
            return any(f.lower().endswith((".m4b", ".m4a", ".mp3")) for f in os.listdir(p))
        except OSError:
            return False

    if not taken(path):
        return path
    n = 2
    while taken(f"{path} ({n})"):
        n += 1
    return f"{path} ({n})"


PARTIAL = ".hw-part-"          # hidden name of a file still being copied into the library


def move_atomic(src: str, dest: str) -> None:
    """Move a file so that ``dest`` only ever appears complete.

    Same disk: one rename. Another disk (inbox and library on different shares): copy to a hidden name next
    to ``dest`` first, then rename – a crash leaves at most a hidden ``.hw-part-*`` file (removed by
    ``remove_partials``), never a half-copied book that Audiobookshelf or Plex would pick up."""
    try:
        os.rename(src, dest)
        return
    except OSError as e:
        if e.errno != errno.EXDEV:
            raise
    tmp = os.path.join(os.path.dirname(dest), PARTIAL + os.path.basename(dest))
    try:
        shutil.copy2(src, tmp)
        with open(tmp, "rb") as fh:
            os.fsync(fh.fileno())
        os.replace(tmp, dest)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    os.unlink(src)


def remove_partials(folder: str) -> int:
    """Hidden half-copies left by a crash in ``move_atomic`` – in this folder only."""
    n = 0
    try:
        names = os.listdir(folder)
    except OSError:
        return 0
    for name in names:
        if name.startswith(PARTIAL):
            try:
                os.unlink(os.path.join(folder, name))
                n += 1
            except OSError:
                pass
    return n


def place(src_m4b: str, folder: str, filename: str, extras: Optional[Dict[str, str]] = None) -> str:
    """Move the book (and extra files) into the library; returns the final m4b path."""
    os.makedirs(folder, exist_ok=True)
    dest = os.path.join(folder, filename)
    move_atomic(src_m4b, dest)
    for name, src in (extras or {}).items():
        if src and os.path.exists(src):
            shutil.move(src, os.path.join(folder, name))
    return dest


def example(folder_tpl: str, file_tpl: str) -> List[str]:
    """Preview for the settings dialog."""
    samples = [
        # public-domain works (no rights of third parties in the examples)
        {"title": "Eine Studie in Scharlachrot", "authors": ["Arthur Conan Doyle"], "narrators": ["Erika Muster"],
         "series": "Sherlock Holmes", "series_position": "1", "release_date": "1887-11-01"},
        {"title": "Die Verwandlung", "authors": ["Franz Kafka"], "narrators": ["Max Muster"],
         "release_date": "1915-10-01"},
    ]
    out = []
    for b in samples:
        folder, name = destination("", b, folder_tpl, file_tpl)
        out.append(os.path.join(folder, name).lstrip("/"))
    return out
