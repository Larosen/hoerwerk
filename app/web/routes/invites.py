"""Invitations by link: admins create and manage them, the invited person opens /invite/<token>."""
from __future__ import annotations

import time

import authlog
from config import public_url
from i18n import t
from invites import InviteError
from web.router import HTTPException, Reply, Req, route
from web.routes.auth import _cookie, _me


def _iid(value: str) -> int:
    try:
        return int(value)
    except ValueError:
        raise HTTPException(404, t("Nicht gefunden"))


# -- admin -------------------------------------------------------------------------
def _public_url(req: Req) -> str:
    """Where invitation links point: always the address from the internet, never the LAN one.

    Not set yet, but this very request came through the trusted reverse proxy over HTTPS? Then that is the
    public address – remembered, so links made later from home point there too."""
    url = req.app.settings().get("public_url", "")
    if not url and req.https and req.conn.get("trusted"):
        url = public_url("https://" + req.conn.get("host", ""))
        if url:
            req.app.update_settings({"public_url": url})
    return url


def _with_link(invite: dict, base: str) -> dict:
    return {**invite, "link": f"{base}/invite/{invite['token']}" if base else ""}


@route("GET", "/api/invites")
def list_invites(req: Req):
    base = _public_url(req)
    return {"invites": [_with_link(i, base) for i in req.app.invites.list()], "public_url": base}


@route("POST", "/api/invites")
def create_invite(req: Req):
    base = _public_url(req)
    if not base:
        raise HTTPException(400, t("Erst die öffentliche Adresse eintragen (Einstellungen → Allgemein), z. B. https://hoerwerk.example.de – Einladungslinks zeigen immer dorthin."))
    try:
        invite = req.app.invites.create(req.user["name"], req.get("note") or "", req.get("days", 7),
                                        req.get("uses", 1), req.get("quota"), req.get("perms") or {})
    except (TypeError, ValueError) as e:
        raise HTTPException(400, str(e))
    authlog.event("invite_created", by=req.user["name"], ip=req.ip, invite=invite["id"])
    return _with_link(invite, base)


@route("POST", "/api/invites/{iid}/revoke")
def revoke_invite(req: Req, iid: str):
    try:
        return req.app.invites.revoke(_iid(iid))
    except KeyError:
        raise HTTPException(404, t("Nicht gefunden"))


@route("DELETE", "/api/invites/{iid}")
def delete_invite(req: Req, iid: str):
    try:
        req.app.invites.delete(_iid(iid))
    except KeyError:
        raise HTTPException(404, t("Nicht gefunden"))
    return {"ok": True}


# -- the invited person (no account yet) ------------------------------------------
def _refuse(req: Req, e: Exception, name: str = "") -> HTTPException:
    """A wrong link counts like a wrong password: IP brake and auth.log (so links can't be guessed)."""
    req.app.accounts.failed(req.ip, name)
    authlog.event("login_failed", user=name[:40], ip=req.ip, reason="bad_invite")
    time.sleep(1)
    return HTTPException(404, str(e))


def _not_blocked(req: Req) -> None:
    if req.app.accounts.is_blocked(req.ip):
        authlog.event("login_blocked", ip=req.ip, reason="ip_blocked")
        raise HTTPException(429, t("Zu viele Fehlversuche – bitte in 15 Minuten erneut versuchen"))


@route("GET", "/api/invite/{token}", access="public")
def check_invite(req: Req, token: str):
    _not_blocked(req)
    try:
        invite = req.app.invites.check(token)
    except InviteError as e:
        raise _refuse(req, e)
    return {"ok": True, "note": invite["note"], "expires": invite["expires"]}


@route("POST", "/api/invite/{token}", access="public")
def accept_invite(req: Req, token: str):
    _not_blocked(req)
    name = str(req.get("name") or "")
    try:
        user = req.app.invites.accept(token, name, str(req.get("password") or ""))
    except InviteError as e:
        raise _refuse(req, e, name)
    except ValueError as e:                      # name taken, password too short …: the link stays valid
        raise HTTPException(400, str(e))
    authlog.event("invite_accepted", user=user["name"], ip=req.ip)
    req.app.audit(req, "invite_accepted", user["name"])
    token_ = req.app.accounts.start_session(user["id"], True, req.ip, req.agent)
    authlog.event("login_ok", user=user["name"], ip=req.ip)
    return Reply({"ok": True, "user": _me(user)}, {"Set-Cookie": _cookie(req, token_, True)})
