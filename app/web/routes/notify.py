"""ntfy notifications: settings (secrets never leave the server) and a test message."""
from __future__ import annotations

from notify import EVENTS, NotifyError
from i18n import t
from web.router import HTTPException, Req, route


@route("GET", "/api/notify")
def read(req: Req):
    return req.app.notifier.public_config() | {"available": list(EVENTS)}


@route("PUT", "/api/notify")
def write(req: Req):
    if not isinstance(req.body, dict):
        raise HTTPException(400, t("Ungültige Daten"))
    try:
        return req.app.notifier.update_config(req.body) | {"available": list(EVENTS)}
    except (NotifyError, ValueError) as e:
        raise HTTPException(400, str(e))


@route("POST", "/api/notify/test")
def test(req: Req):
    try:
        return {"ok": True, "message": req.app.notifier.test()}
    except NotifyError as e:
        return {"ok": False, "message": str(e)}
