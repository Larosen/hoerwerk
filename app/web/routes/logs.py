"""The app log (Protokoll): newest entries, filtered by level."""
from __future__ import annotations

import applog
from i18n import t
from web.router import HTTPException, Req, route


@route("GET", "/api/logs")
def logs(req: Req):
    try:
        after = max(0, int(req.query.get("after") or 0))
    except ValueError:
        raise HTTPException(400, t("Ungültige Daten"))
    level = req.query.get("level") or "info"
    return {"entries": applog.log.since(after, level), "last": applog.log.seq}
