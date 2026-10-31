"""Entdecken: suggested audiobooks (for the library, by genre, from book reviews)."""
from __future__ import annotations

import discover
from i18n import t
from web.router import HTTPException, Req, route


def _section(req: Req, value: str) -> str:
    if value not in discover.SECTIONS:
        raise HTTPException(400, t("Ungültige Daten"))
    return value


@route("GET", "/api/discover")
def suggestions(req: Req):
    section = _section(req, req.query.get("section") or "foryou")
    genre = (req.query.get("genre") or "").strip()
    if section == "genre" and not genre.isdigit():
        return {"items": [], "updated": None, "error": None, "running": False}
    return req.app.discover.get(section, genre)


@route("GET", "/api/discover/genres")
def genres(req: Req):
    return {"genres": req.app.discover.genres()}


@route("POST", "/api/discover/refresh")
def refresh(req: Req):
    section = _section(req, req.get("section") or "foryou")
    genre = str(req.get("genre") or "").strip()
    if section == "genre" and not genre.isdigit():
        raise HTTPException(400, t("Ungültige Daten"))
    req.app.discover.refresh(section, genre)
    return {"ok": True}
