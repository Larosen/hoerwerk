"""Path helpers shared by the handlers."""
from __future__ import annotations

import os

from i18n import t
from web.router import HTTPException


def resolve(app, rel: str) -> str:
    """A path inside /data (symlinks resolved, no way out)."""
    root = app.config.data_root
    path = os.path.realpath(os.path.join(root, (rel or "").strip("/")))
    if os.path.commonpath([path, root]) != root:
        raise HTTPException(400, t("Pfad außerhalb des Datenordners"))
    return path


def relpath(app, path: str) -> str:
    r = os.path.relpath(path, app.config.data_root)
    return "" if r == "." else r


def library_path(app, rel: str) -> str:
    """A book folder inside the library (no way out via .. or symlinks)."""
    root = os.path.realpath(app.config.library)
    path = os.path.realpath(os.path.join(root, (rel or "").strip("/")))
    if path == root or os.path.commonpath([path, root]) != root or not os.path.isdir(path):
        raise HTTPException(404, t("Ordner nicht in der Bibliothek gefunden"))
    return path


def busy(app, folder: str) -> bool:
    """A conversion runs in this folder or below."""
    return any(b == folder or b.startswith(folder + os.sep) for b in app.converter.busy_folders())
