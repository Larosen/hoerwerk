"""Gesucht: the list of wanted audiobooks, searched on the indexers every few hours."""
from __future__ import annotations

from i18n import t
from web.router import HTTPException, Req, route


@route("GET", "/api/wanted")
def wanted_list(req: Req):
    s = req.app.settings()
    return {"items": req.app.wanted.items(), "interval": req.app.wanted.interval_hours(),
            "auto": bool(s.get("wanted_auto")), "indexers": req.app.usenet.configured()["indexers"]}


@route("POST", "/api/wanted")
def wanted_add(req: Req):
    if not isinstance(req.body, dict):
        raise HTTPException(400, t("Ungültige Daten"))
    try:
        item = req.app.wanted.add(req.body)
    except ValueError as e:
        raise HTTPException(400, str(e))
    if req.app.usenet.configured()["indexers"]:
        req.app.wanted.search(item["id"])          # first search right away
    return {"ok": True, "id": item["id"]}


@route("POST", "/api/wanted/search")
def wanted_search(req: Req):
    item = req.app.wanted.search(str(req.get("id") or ""))
    if item is None:
        raise HTTPException(404, t("Nicht gefunden"))
    return item


@route("DELETE", "/api/wanted/{wid}")
def wanted_remove(req: Req, wid: str):
    if not req.app.wanted.remove(wid):
        raise HTTPException(404, t("Nicht gefunden"))
    return {"ok": True}
