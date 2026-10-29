"""Indexer search -> SABnzbd, with the library cross-check."""
from __future__ import annotations

from typing import List, Optional

import audible
from abs import AbsError
from usenet import UsenetError
from web.router import HTTPException, Req, route


def _owned(entries: List[dict]) -> Optional[dict]:
    if not entries:
        return None
    e = entries[0]
    return {"path": e["path"], "title": e.get("title"), "authors": e.get("authors"),
            "why": e.get("why", "similar"), "count": len(entries)}


@route("GET", "/api/sources")
def sources(req: Req):
    return req.app.usenet.public_config() | {"abs": req.app.abs.public_config()}


@route("PUT", "/api/sources")
def write_sources(req: Req):
    if not isinstance(req.body, dict):
        raise HTTPException(400, "Ungültige Daten")
    if isinstance(req.body.get("abs"), dict):
        req.app.abs.update_config(req.body["abs"])
    return req.app.usenet.update_config(req.body) | {"abs": req.app.abs.public_config()}


@route("POST", "/api/sources/test")
def test_source(req: Req):
    try:
        if req.get("kind") == "abs":
            return {"ok": True, "message": req.app.abs.test()}
        return {"ok": True, "message": req.app.usenet.test(req.get("kind", ""), req.get("id", ""))}
    except (UsenetError, AbsError) as e:
        return {"ok": False, "message": str(e)}
    except Exception as e:  # noqa: BLE001 – unexpected answers (HTML instead of XML …)
        return {"ok": False, "message": f"Unerwartete Antwort: {e}"}


@route("POST", "/api/search")
def search_indexers(req: Req):
    query = (req.get("query") or "").strip()
    if len(query) < 2:
        raise HTTPException(400, "Suchbegriff fehlt")
    extra = [str(x) for x in (req.get("extra") or []) if isinstance(x, str)][:3]
    try:
        res = req.app.usenet.search(query, req.get("language") or "", extra)
    except UsenetError as e:
        raise HTTPException(400, str(e))
    for r in res["results"]:
        r["owned"] = _owned(req.app.library_index.match_release(r["title"]))
    return res


@route("POST", "/api/search/books")
def search_books(req: Req):
    """Look the book up at Audible first – German title and author as the shop lists them."""
    query = (req.get("query") or "").strip()
    if len(query) < 2:
        raise HTTPException(400, "Suchbegriff fehlt")
    region = req.get("region") or req.app.settings()["ab_region"]
    if region not in audible.REGIONS:
        raise HTTPException(400, "Unbekannte Region")
    try:
        hints = {"query": audible.clean_query(query) or query, "asin": audible.find_asin(query)}
        books = req.app.pipeline.providers.search(hints, region)[:8]
    except audible.AudibleError as e:
        raise HTTPException(502, str(e))
    keys = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_position",
            "runtime_min", "release_date", "cover_url", "format_type", "url", "source")
    return {"books": [{**{k: b.get(k) for k in keys}, "owned": _owned(req.app.library_index.find(b))}
                      for b in books], "region": region}


@route("POST", "/api/search/send")
def send_to_sab(req: Req):
    hints = {k: req.get(k) for k in ("title", "author", "asin") if req.get(k)}
    try:
        return req.app.usenet.send(req.get("id", ""), hints)
    except UsenetError as e:
        raise HTTPException(400, str(e))


@route("GET", "/api/downloads")
def downloads(req: Req):
    usenet = req.app.usenet
    if not usenet.configured()["sab"]:
        return {"configured": False, "items": []}
    try:
        return {"configured": True, **usenet.activity()}
    except UsenetError as e:
        return {"configured": True, "error": str(e), "items": []}
