"""Upload audiobooks from the browser into the inbox (e.g. ripped from CD)."""
from __future__ import annotations

import uploads
from i18n import t
from web.router import HTTPException, Req, route


def _call(fn, *args, **kw):
    try:
        return fn(*args, **kw)
    except uploads.UploadError as e:
        raise HTTPException(400, str(e))


def _owner(req: Req) -> str:
    return f"u{req.user['id']}"


@route("POST", "/api/upload")
def start(req: Req):
    return {"id": _call(req.app.uploads.start, _owner(req), req.user["name"])}


@route("PUT", "/api/upload/{uid}", raw_body=uploads.MAX_FILE)
def put_file(req: Req, uid: str):
    """One file; its path inside the upload (e.g. "Autor - Titel/CD1/01.mp3") in ?path=."""
    return _call(req.app.uploads.put, uid, req.query.get("path") or "", req.stream, req.length, owner=_owner(req))


@route("POST", "/api/upload/{uid}/finish")
def finish(req: Req, uid: str):
    names = _call(req.app.uploads.finish, uid, owner=_owner(req))
    if not names:
        raise HTTPException(400, t("Keine Dateien hochgeladen"))
    req.app.audit(req, "upload", ", ".join(names))
    return {"ok": True, "names": names}


@route("DELETE", "/api/upload/{uid}")
def cancel(req: Req, uid: str):
    _call(req.app.uploads.cancel, uid, owner=_owner(req))
    return {"ok": True}
