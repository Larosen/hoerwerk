"""Login, logout, first setup, own account; user management for admins."""
from __future__ import annotations

import ipaddress
import time

import authlog
from accounts import AuthError, SESSION_LONG
from i18n import t
from web.router import HTTPException, Reply, Req, route

COOKIE = "hw_session"


def _cookie(req: Req, token: str, persistent: bool) -> str:
    parts = [f"{COOKIE}={token}", "Path=/", "HttpOnly", "SameSite=Lax"]
    if req.https:
        parts.append("Secure")
    if persistent:
        parts.append(f"Max-Age={SESSION_LONG}")
    return "; ".join(parts)


def _gone_cookie(req: Req) -> str:
    return f"{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0" + ("; Secure" if req.https else "")


def _me(user: dict) -> dict:
    return {k: user[k] for k in ("id", "name", "role", "perms", "quota")}


def _home(req: Req) -> bool:
    """Directly from the home network – not through a reverse proxy (which may carry anyone from the internet)."""
    if req.proxied:
        return False
    try:
        return not ipaddress.ip_address(req.ip).is_global
    except ValueError:
        return False


@route("GET", "/api/auth/status", access="public")
def status(req: Req):
    """For the login page: is there an account yet (else: first setup), and does setup need the code."""
    setup = req.app.accounts.count() == 0
    user = req.app.accounts.session_user(req.token) if req.token else None
    return {"setup": setup, "setup_code": setup and not _home(req), "user": _me(user) if user else None}


@route("POST", "/api/auth/setup", access="public")
def setup(req: Req):
    """Create the first admin – only while there is no account at all."""
    accounts = req.app.accounts
    if accounts.count():
        raise HTTPException(409, t("Es gibt schon ein Konto – bitte anmelden"))
    if not _home(req) and str(req.get("code") or "").strip().upper() != req.app.setup_code:
        authlog.event("login_failed", user=req.get("name") or "", ip=req.ip, reason="bad_setup_code")
        time.sleep(1)
        raise HTTPException(403, t("Einrichtungs-Code falsch (steht im Container-Log)"))
    try:
        user = accounts.create(str(req.get("name") or ""), str(req.get("password") or ""), role="admin")
    except ValueError as e:
        raise HTTPException(400, str(e))
    authlog.event("setup", user=user["name"], ip=req.ip)
    token = accounts.start_session(user["id"], True, req.ip, req.agent)
    return Reply({"ok": True, "user": _me(user)}, {"Set-Cookie": _cookie(req, token, True)})


@route("POST", "/api/auth/login", access="public")
def login(req: Req):
    accounts = req.app.accounts
    name = str(req.get("name") or "")
    time.sleep(accounts.delay_for(name))
    try:
        user = accounts.authenticate(name, str(req.get("password") or ""), req.ip)
    except AuthError as e:
        authlog.event("login_blocked" if e.status == 429 else "login_failed", user=name[:40], ip=req.ip,
                      reason=e.reason)
        raise HTTPException(e.status, str(e))
    persistent = bool(req.get("remember"))
    token = accounts.start_session(user["id"], persistent, req.ip, req.agent)
    authlog.event("login_ok", user=user["name"], ip=req.ip)
    return Reply({"ok": True, "user": _me(user)}, {"Set-Cookie": _cookie(req, token, persistent)})


@route("POST", "/api/auth/logout", access="user")
def logout(req: Req):
    req.app.accounts.end_session(req.token)
    authlog.event("logout", user=req.user["name"], ip=req.ip)
    return Reply({"ok": True}, {"Set-Cookie": _gone_cookie(req)})


@route("GET", "/api/auth/me", access="user")
def me(req: Req):
    return {**_me(req.user), "sessions": req.app.accounts.sessions(req.user["id"]), "session": req.user["session"]}


@route("POST", "/api/auth/password", access="user")
def change_password(req: Req):
    accounts = req.app.accounts
    try:
        accounts.authenticate(req.user["name"], str(req.get("old") or ""), req.ip)
    except AuthError as e:
        authlog.event("login_failed", user=req.user["name"], ip=req.ip, reason="bad_old_password")
        raise HTTPException(e.status if e.status == 429 else 403, t("Das bisherige Passwort stimmt nicht"))
    try:
        accounts.set_password(req.user["id"], str(req.get("new") or ""))
    except ValueError as e:
        raise HTTPException(400, str(e))
    authlog.event("password_changed", user=req.user["name"], ip=req.ip)
    token = accounts.start_session(req.user["id"], True, req.ip, req.agent)     # stay signed in here
    return Reply({"ok": True}, {"Set-Cookie": _cookie(req, token, True)})


@route("POST", "/api/auth/sessions/end", access="user")
def end_sessions(req: Req):
    """End one session (``id``) or all others (no id)."""
    sid = str(req.get("id") or "")
    accounts = req.app.accounts
    n = accounts.end_session_id(req.user["id"], sid) if sid else accounts.end_sessions(req.user["id"],
                                                                                     keep=req.user["session"])
    authlog.event("sessions_revoked", user=req.user["name"], ip=req.ip, count=int(n))
    return {"ok": True, "ended": int(n)}


# -- user management (admin) ---------------------------------------------------
def _uid(value: str) -> int:
    try:
        return int(value)
    except ValueError:
        raise HTTPException(404, t("Nicht gefunden"))


@route("GET", "/api/users")
def users(req: Req):
    return {"users": req.app.accounts.list(), "default_quota": req.app.settings().get("request_quota", 5)}


@route("POST", "/api/users")
def create_user(req: Req):
    try:
        user = req.app.accounts.create(str(req.get("name") or ""), str(req.get("password") or ""),
                                       str(req.get("role") or "user"), req.get("perms") or {}, req.get("quota"))
    except (ValueError, TypeError) as e:
        raise HTTPException(400, str(e))
    authlog.event("user_created", user=user["name"], by=req.user["name"], ip=req.ip, role=user["role"])
    return user


@route("PUT", "/api/users/{uid}")
def change_user(req: Req, uid: str):
    accounts = req.app.accounts
    n = _uid(uid)
    changes = {k: req.get(k) for k in ("name", "role", "perms", "quota", "disabled") if k in req.body}
    try:
        user = accounts.update(n, changes)
        if req.get("password"):
            accounts.set_password(n, str(req.get("password")))
    except KeyError:
        raise HTTPException(404, t("Nicht gefunden"))
    except (ValueError, TypeError) as e:
        raise HTTPException(400, str(e))
    authlog.event("user_changed", user=user["name"], by=req.user["name"], ip=req.ip,
                  fields=",".join(sorted(changes) + (["password"] if req.get("password") else [])))
    return user


@route("DELETE", "/api/users/{uid}")
def delete_user(req: Req, uid: str):
    accounts = req.app.accounts
    n = _uid(uid)
    user = accounts.get(n)
    if not user:
        raise HTTPException(404, t("Nicht gefunden"))
    if n == req.user["id"]:
        raise HTTPException(400, t("Das eigene Konto kann nicht gelöscht werden"))
    try:
        accounts.delete(n)
    except ValueError as e:
        raise HTTPException(400, str(e))
    authlog.event("user_deleted", user=user["name"], by=req.user["name"], ip=req.ip)
    return {"ok": True}
