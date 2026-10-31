"""Path helpers shared by the handlers."""
from __future__ import annotations

import os

from i18n import t
from web.router import HTTPException


def library_path(app, rel: str) -> str:
    """A book folder inside the library (no way out via .. or symlinks)."""
    root = os.path.realpath(app.config.library)
    path = os.path.realpath(os.path.join(root, (rel or "").strip("/")))
    if path == root or os.path.commonpath([path, root]) != root or not os.path.isdir(path):
        raise HTTPException(404, t("Ordner nicht in der Bibliothek gefunden"))
    return path

