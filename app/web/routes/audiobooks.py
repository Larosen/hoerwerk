"""Audiobook pipeline: status, review (search, link, choose, own data), duplicates, retry."""
from __future__ import annotations

import os

import audible
import library
import links
from i18n import t
from web.router import HTTPException, Req, route
from web.routes.common import resolve


def _job(req: Req, job_id: str):
    try:
        return req.app.pipeline.get(job_id)
    except KeyError:
        raise HTTPException(404, t("Hörbuch nicht gefunden"))


def _region(req: Req) -> str:
    region = req.get("region") or req.app.settings()["ab_region"]
    if region not in audible.REGIONS:
        raise HTTPException(400, t("Unbekannte Region"))
    return region


@route("GET", "/api/audiobooks")
def status(req: Req):
    return req.app.pipeline.status() | {"queues": req.app.engine.stats()}


@route("GET", "/api/audiobooks/paths")
def paths(req: Req):
    s = req.app.settings()
    return {"examples": library.example(req.query.get("folder") or s["ab_folder_template"],
                                        req.query.get("file") or s["ab_file_template"]),
            "regions": list(audible.REGIONS)}


@route("POST", "/api/audiobooks/scan")
def scan(req: Req):
    req.app.pipeline.scan_now()
    return {"ok": True}


@route("POST", "/api/audiobooks/import")
def import_(req: Req):
    path = resolve(req.app, req.get("path", ""))
    if not os.path.exists(path):
        raise HTTPException(404, t("Nicht gefunden"))
    return req.app.pipeline.add(path, origin="manual").to_dict(full=False)


@route("GET", "/api/audiobooks/{job_id}")
def detail(req: Req, job_id: str):
    job = _job(req, job_id)
    return job.to_dict(full=False) | {"log": job.log}


@route("POST", "/api/audiobooks/{job_id}/search")
def search(req: Req, job_id: str):
    _job(req, job_id)
    query = (req.get("query") or "").strip()
    if not query:
        raise HTTPException(400, t("Suchbegriff fehlt"))
    region = _region(req)
    try:
        return req.app.pipeline.search(job_id, query, region).to_dict(full=False)
    except links.LinkError as e:
        raise HTTPException(400, str(e))
    except audible.AudibleError as e:
        raise HTTPException(502, str(e))


@route("POST", "/api/audiobooks/{job_id}/link")
def link(req: Req, job_id: str):
    """Fetch book data from a shop link; returns the job and the new candidate."""
    _job(req, job_id)
    url = (req.get("url") or "").strip()
    if not links.is_link(url):
        raise HTTPException(400, t("Bitte einen Link von Audible, BookBeat oder Thalia einfügen"))
    try:
        job, _book = req.app.pipeline.add_link(job_id, url)
    except links.LinkError as e:
        raise HTTPException(400, str(e))
    except audible.AudibleError as e:
        raise HTTPException(502, str(e))
    data = job.to_dict(full=False)
    return {"job": data, "candidate": data["candidates"][0]}


@route("POST", "/api/audiobooks/{job_id}/duplicate")
def duplicate(req: Req, job_id: str):
    _job(req, job_id)
    try:
        return req.app.pipeline.resolve_duplicate(job_id, req.get("action", "")).to_dict(full=False)
    except ValueError as e:
        raise HTTPException(400, str(e))
    except OSError as e:
        raise HTTPException(500, t("Konnte die alte Fassung nicht verschieben: {error}", error=e))


@route("POST", "/api/audiobooks/{job_id}/reopen")
def reopen(req: Req, job_id: str):
    _job(req, job_id)
    try:
        return req.app.pipeline.reopen(job_id).to_dict(full=False)
    except ValueError as e:
        raise HTTPException(409, str(e))
    except OSError as e:
        raise HTTPException(500, t("Konnte nicht aus der Bibliothek holen: {error}", error=e))


@route("POST", "/api/audiobooks/{job_id}/choose")
def choose(req: Req, job_id: str):
    _job(req, job_id)
    cand_id = str(req.get("id") or "").strip()
    asin = (req.get("asin") or "").strip().upper()
    if not cand_id and not audible.ASIN_RX.fullmatch(asin):
        raise HTTPException(400, t("Ungültige ASIN"))
    try:
        return req.app.pipeline.choose(job_id, cand_id, asin,
                                       req.get("region") or req.app.settings()["ab_region"]).to_dict(full=False)
    except KeyError:
        raise HTTPException(404, t("Treffer nicht gefunden – bitte neu suchen"))
    except audible.AudibleError as e:
        raise HTTPException(502, str(e))


@route("POST", "/api/audiobooks/{job_id}/manual")
def manual(req: Req, job_id: str):
    _job(req, job_id)
    data = req.body if isinstance(req.body, dict) else {}
    return req.app.pipeline.manual(job_id, data, str(data.get("base_id") or "")).to_dict(full=False)


@route("POST", "/api/audiobooks/{job_id}/retry")
def retry(req: Req, job_id: str):
    _job(req, job_id)
    try:
        return req.app.pipeline.retry(job_id).to_dict(full=False)
    except OSError as e:
        raise HTTPException(500, t("Konnte nicht zurück in den Eingang verschieben: {error}", error=e))


@route("POST", "/api/audiobooks/{job_id}/cancel")
def cancel(req: Req, job_id: str):
    _job(req, job_id)
    return req.app.pipeline.cancel(job_id).to_dict(full=False)


@route("DELETE", "/api/audiobooks/{job_id}")
def remove(req: Req, job_id: str):
    _job(req, job_id)
    try:
        req.app.pipeline.remove(job_id)
    except ValueError as e:
        raise HTTPException(409, str(e))
    return {"ok": True}


@route("DELETE", "/api/audiobooks")
def clear(req: Req):
    req.app.pipeline.clear_finished()
    return {"ok": True}
