"""Entdecken: suggested audiobooks (for the library, by genre, from book reviews)."""
from __future__ import annotations

import bookrequests as br
import discover
from i18n import t
from web.router import HTTPException, Req, route


def _section(req: Req, value: str) -> str:
    if value not in discover.SECTIONS:
        raise HTTPException(400, t("Ungültige Daten"))
    return value


@route("GET", "/api/discover", access="user")
def suggestions(req: Req):
    section = _section(req, req.query.get("section") or "foryou")
    genre = (req.query.get("genre") or "").strip()
    if section == "genre" and not (genre.isdigit() and req.app.discover.known_genre(genre)):
        return {"items": [], "updated": None, "error": None, "running": False}
    res = req.app.discover.get(section, genre)
    requested = req.app.requests.status_by_key()
    res["items"] = [{**i, "requested": requested.get(br.book_key(i))} for i in res["items"]]
    return res


@route("GET", "/api/discover/genres", access="user")
def genres(req: Req):
    return {"genres": req.app.discover.genres()}


@route("POST", "/api/discover/refresh")
def refresh(req: Req):
    section = _section(req, req.get("section") or "foryou")
    genre = str(req.get("genre") or "").strip()
    if section == "genre" and not (genre.isdigit() and req.app.discover.known_genre(genre)):
        raise HTTPException(400, t("Ungültige Daten"))
    req.app.discover.refresh(section, genre)
    return {"ok": True}
