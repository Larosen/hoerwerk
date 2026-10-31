"""Aufräumen: leftover text files next to the books that nothing needs any more.

``desc.txt`` and ``reader.txt`` (from beets-audible) only serve Audiobookshelf as a fallback when there is no
``metadata.json``; Plex ignores them, and everything in them is in the M4B tags too. They are only removed from
book folders that have a ``metadata.json`` – there they are surely redundant. Symlinks are never followed or deleted.
"""
from __future__ import annotations

import os
import stat
from typing import List

from abs import METADATA_FILE

LEFTOVERS = ("desc.txt", "reader.txt")


def _regular(path: str) -> bool:
    try:
        return stat.S_ISREG(os.lstat(path).st_mode)
    except OSError:
        return False


def leftovers(root: str) -> List[str]:
    """Paths (relative to the library) of redundant desc.txt/reader.txt files."""
    if not root or not os.path.isdir(root):
        return []
    out: List[str] = []
    for folder, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not d.startswith("."))     # hidden: parked old copies & co.
        if METADATA_FILE not in files or not _regular(os.path.join(folder, METADATA_FILE)):
            continue
        for name in LEFTOVERS:
            path = os.path.join(folder, name)
            if name in files and _regular(path):
                out.append(os.path.relpath(path, root))
    return out


def remove(root: str) -> dict:
    """Delete what ``leftovers`` finds right now (never a list from the browser)."""
    removed, errors = 0, []
    for rel in leftovers(root):
        try:
            os.unlink(os.path.join(root, rel))
            removed += 1
        except OSError as e:
            errors.append(f"{rel}: {e.strerror or e}")
    return {"removed": removed, "errors": errors[:20]}
