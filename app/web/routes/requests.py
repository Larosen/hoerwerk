"""Book requests: ask for a book, approve/decline (admin), follow it, interactive release search."""
from __future__ import annotations

import authlog
import bookrequests as br
from i18n import t
from web.router import HTTPException, Req, route


def _id(value: str) -> int:
    try:
        return int(value)
    except ValueError:
        raise HTTPException(404, t("Nicht gefunden"))


def _visible(req: Req, rid: str) -> dict:
    """The request – only if this account may see it (others get 404, not 403: no guessing of ids)."""
    item = req.app.requests.get(_id(rid))
    if not item or not req.app.requests.can_see(req.user, item):
        raise HTTPException(404, t("Nicht gefunden"))
    return item


def _call(fn, *args, **kw):
    try:
        return fn(*args, **kw)
    except br.RequestError as e:
        raise HTTPException(e.status, str(e))


def _out(req: Req, item: dict) -> dict:
    return br.summary(item, req.user, req.app.requests.sees_all(req.user))


@route("GET", "/api/requests", access="user")
def list_requests(req: Req):
    rq = req.app.requests
    # whoever may see all requests (admins, or the right) always sees all – no switch that could hide approvals
    return {"items": [_out(req, r) for r in rq.list(req.user, rq.sees_all(req.user))], "quota": rq.quota(req.user),
            "sees_all": rq.sees_all(req.user), "may_search": rq.may_search(req.user),
            "pending": rq.pending_count() if req.is_admin else None,
            "interval": rq.interval_hours()}


@route("GET", "/api/requests/keys", access="user")
def request_keys(req: Req):
    """Which books are requested already (status by book key) – for marks on cards."""
    return {"keys": req.app.requests.status_by_key()}


@route("POST", "/api/requests", access="user")
def create_request(req: Req):
    if not isinstance(req.get("book"), dict):
        raise HTTPException(400, t("Ungültige Daten"))
    item = _call(req.app.requests.create, req.user, req.get("book"))
    return _out(req, item)


@route("POST", "/api/requests/series", access="user")
def create_series_request(req: Req):
    """A whole series (or the chosen volumes): one request, counted once, approved together."""
    res = _call(req.app.requests.create_series, req.user, req.get("series"), req.get("books"))
    return {"group": res["group"], "status": res["status"], "skipped": res["skipped"],
            "items": [_out(req, r) for r in res["items"]]}


@route("GET", "/api/requests/{rid}", access="user")
def one_request(req: Req, rid: str):
    return _out(req, _visible(req, rid))


@route("DELETE", "/api/requests/{rid}", access="user")
def withdraw_request(req: Req, rid: str):
    item = _visible(req, rid)
    whole = req.query.get("group") == "1"
    n = _call(req.app.requests.withdraw, item["id"], req.user, whole)
    if req.is_admin and item["user_id"] != req.user["id"]:
        req.app.audit(req, "request_deleted", f"{item['id']} {item['book'].get('title')}" + (f" (+{n - 1})" if n > 1 else ""))
    return {"ok": True, "deleted": n}


@route("POST", "/api/requests/{rid}/approve")
def approve_request(req: Req, rid: str):
    item = _call(req.app.requests.approve, _id(rid), req.user)
    req.app.audit(req, "request_approved", f"{item['id']} {item['book'].get('title')}")
    return _out(req, item)


@route("POST", "/api/requests/{rid}/decline")
def decline_request(req: Req, rid: str):
    item = _call(req.app.requests.decline, _id(rid), req.user, str(req.get("reason") or ""))
    req.app.audit(req, "request_declined", f"{item['id']} {item['book'].get('title')}")
    return _out(req, item)


@route("POST", "/api/requests/{rid}/retry")
def retry_request(req: Req, rid: str):
    return _out(req, _call(req.app.requests.retry, _id(rid)))


# -- interactive search (admins, and accounts with the right) ------------------
def _searcher(req: Req, rid: str) -> dict:
    item = _visible(req, rid)
    if not req.app.requests.may_search(req.user):
        authlog.event("forbidden", user=req.user["name"], ip=req.ip, path=f"/api/requests/{rid}/releases")
        raise HTTPException(403, t("Dafür fehlen die Rechte"))
    return item


@route("POST", "/api/requests/{rid}/releases", access="user")
def request_releases(req: Req, rid: str):
    item = _searcher(req, rid)
    res = _call(req.app.requests.releases, item["id"], str(req.get("query") or ""))
    out = [{**{k: v for k, v in r.items() if k != "link"}} for r in res["results"]]
    return {"query": res["query"], "queries": res["queries"], "errors": res["errors"], "results": out}


@route("POST", "/api/requests/{rid}/grab", access="user")
def request_grab(req: Req, rid: str):
    item = _searcher(req, rid)
    res = _call(req.app.requests.grab, item["id"], str(req.get("release_id") or ""), req.user,
                {"title": str(req.get("title") or "")})
    req.app.audit(req, "request_grab", f"{item['id']} {res.get('release')}")
    return _out(req, res)


@route("POST", "/api/requests/{rid}/block", access="user")
def request_block(req: Req, rid: str):
    item = _searcher(req, rid)
    return _out(req, _call(req.app.requests.block, item["id"], str(req.get("title") or "")[:300]))


# -- statistics per account (admins) ----------------------------------------------
@route("GET", "/api/stats")
def stats(req: Req):
    try:
        return req.app.stats.summary(str(req.query.get("period") or "365"))
    except ValueError as e:
        raise HTTPException(400, str(e))
