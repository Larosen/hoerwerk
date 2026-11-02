"""Listening history: link Hörwerk accounts with Audiobookshelf accounts and Plex profiles."""
from __future__ import annotations

import time

import authlog
from abs import AbsError
from i18n import t
from listening import LinkError
from plex import PlexError
from web.router import HTTPException, Req, route


def _kind(value: str) -> str:
    if value not in ("abs", "plex"):
        raise HTTPException(404, t("Nicht gefunden"))
    return value


# -- everyone: their own links --------------------------------------------------------
@route("GET", "/api/me/listening", access="user")
def my_listening(req: Req):
    lst = req.app.listening
    links = lst.links(req.user["id"])
    out = {"available": lst.available(), "links": links, "recent": None}
    if links.get("abs") or links.get("plex"):
        out["recent"] = lst.summary(req.user["id"], time.time() - 30 * 86400)
    return out


@route("POST", "/api/me/listening/abs", access="user")
def link_my_abs(req: Req):
    """Sign in to Audiobookshelf once – wrong passwords count on the same brake as Hörwerk's login."""
    if req.app.accounts.is_blocked(req.ip):
        raise HTTPException(429, t("Zu viele Fehlversuche – bitte in 15 Minuten erneut versuchen"))
    username = str(req.get("username") or "").strip()
    try:
        links = req.app.listening.link_own_abs(req.user["id"], username, str(req.get("password") or ""))
    except LinkError as e:
        raise HTTPException(400, str(e))
    except AbsError as e:
        if "falsch" in str(e) or "wrong" in str(e):
            req.app.accounts.failed(req.ip, req.user["name"])
            authlog.event("login_failed", user=req.user["name"], ip=req.ip, reason="bad_abs_login")
        raise HTTPException(400, str(e))
    req.app.audit(req, "listening_linked", f"abs {links['abs']['name']}")
    return {"links": links}


@route("DELETE", "/api/me/listening/{kind}", access="user")
def unlink_mine(req: Req, kind: str):
    return {"links": req.app.listening.set_link(req.user["id"], _kind(kind), None)}


# -- admins: who is who ------------------------------------------------------------------
@route("GET", "/api/listening/accounts")
def listening_accounts(req: Req):
    lst = req.app.listening
    avail = lst.available()
    out = {"available": avail, "abs": None, "plex": None, "errors": [],
           "users": [{"id": u["id"], "name": u["name"], "links": lst.links(u["id"])} for u in req.app.accounts.list()]}
    if avail["abs"]:
        try:
            out["abs"] = lst.abs_accounts()
        except AbsError as e:
            out["errors"].append(str(e))
    if avail["plex"]:
        try:
            out["plex"] = lst.plex_accounts()
        except PlexError as e:
            out["errors"].append(str(e))
    return out


@route("PUT", "/api/listening/link")
def set_link(req: Req):
    kind = _kind(str(req.get("kind") or ""))
    try:
        uid = int(req.get("user_id"))
    except (TypeError, ValueError):
        raise HTTPException(400, t("Ungültige Daten"))
    target_id = str(req.get("target_id") or "")
    target = None
    if target_id:
        try:
            choices = req.app.listening.abs_accounts() if kind == "abs" else req.app.listening.plex_accounts()
        except (AbsError, PlexError) as e:
            raise HTTPException(502, str(e))
        target = next((c for c in choices if c["id"] == target_id), None)
        if not target:
            raise HTTPException(400, t("Ungültige Daten"))
    try:
        links = req.app.listening.set_link(uid, kind, target)
    except LinkError as e:
        raise HTTPException(400, str(e))
    req.app.audit(req, "listening_link", f"{uid} {kind} {target['name'] if target else '–'}")
    return {"links": links}
