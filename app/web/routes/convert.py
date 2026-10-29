"""„Jetzt zu M4B“ in the renamer: conversion jobs for folders in /data."""
from __future__ import annotations

from converter import list_mp3s
from web.router import HTTPException, Req, route
from web.routes.common import relpath, resolve


@route("GET", "/api/jobs")
def jobs(req: Req):
    return req.app.converter.list()


@route("POST", "/api/jobs")
def create_job(req: Req):
    folder = resolve(req.app, req.get("path", ""))
    if not list_mp3s(folder):
        raise HTTPException(400, "Keine MP3-Dateien in diesem Ordner")
    return req.app.converter.enqueue(folder, relpath(req.app, folder), "Manuell").to_dict()


@route("POST", "/api/jobs/{job_id}/cancel")
def cancel_job(req: Req, job_id: str):
    if not req.app.converter.cancel(job_id):
        raise HTTPException(404, "Job nicht gefunden oder bereits beendet")
    return {"ok": True}


@route("DELETE", "/api/jobs")
def clear_jobs(req: Req):
    req.app.converter.clear_finished()
    return {"ok": True}
