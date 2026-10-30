"""Renamer: browse /data, file details, preview, rename, history, undo, presets."""
from __future__ import annotations

import os
from datetime import datetime
from typing import List

import planner
from config import RENAME_OPTION_KEYS
from converter import list_mp3s
from meta import cache as meta_cache, kind_of
from rules import natural_key
from i18n import t
from web.router import HTTPException, Req, route
from web.routes.common import busy, relpath, resolve


def _entry(rel: str, st: os.stat_result, is_dir: bool) -> dict:
    return {"name": rel, "size": 0 if is_dir else st.st_size, "mtime": st.st_mtime,
            "dir": is_dir, "kind": "folder" if is_dir else kind_of(rel)}


@route("GET", "/api/browse")
def browse(req: Req):
    app = req.app
    folder = resolve(app, req.query.get("path", ""))
    if not os.path.isdir(folder):
        raise HTTPException(404, t("Ordner nicht gefunden"))
    visible = lambda name: app.config.show_hidden or not name.startswith(".")  # noqa: E731
    recursive = req.query.get("recursive") == "1"
    include_dirs = req.query.get("dirs") == "1"
    dirs, items, truncated = [], [], False
    try:
        entries = sorted(os.scandir(folder), key=lambda e: natural_key(e.name))
    except PermissionError:
        raise HTTPException(403, t("Keine Berechtigung für diesen Ordner"))
    for e in entries:
        if visible(e.name):
            try:
                if e.is_dir():
                    dirs.append({"name": e.name})
            except OSError:
                pass

    def walk(rel: str) -> None:
        nonlocal truncated
        try:
            scan = sorted(os.scandir(os.path.join(folder, rel)), key=lambda e: natural_key(e.name))
        except OSError:
            return
        subdirs = []
        for e in scan:
            if not visible(e.name) or e.name.startswith(".rn-") or e.name.startswith(".rb-"):
                continue
            if len(items) >= app.config.max_items:
                truncated = True
                return
            r = f"{rel}/{e.name}" if rel else e.name
            try:
                is_dir = e.is_dir(follow_symlinks=False)   # don't wander out through symlinked folders
                st = e.stat()
            except OSError:
                continue
            if is_dir:
                subdirs.append(r)
                if include_dirs:
                    items.append(_entry(r, st, True))
            elif e.is_file():
                items.append(_entry(r, st, False))
        if recursive:
            for r in subdirs:
                walk(r)

    walk("")
    rel = relpath(app, folder)
    return {
        "path": rel,
        "parent": None if not rel else relpath(app, os.path.dirname(folder)),
        "dirs": dirs,
        "files": items,
        "truncated": truncated,
        "mp3_count": len(list_mp3s(folder)),
        "busy": folder in app.converter.busy_folders(),
    }


@route("GET", "/api/meta")
def file_meta(req: Req):
    """All template placeholders and their values for one file."""
    folder = resolve(req.app, req.query.get("path", ""))
    full = resolve(req.app, os.path.join(relpath(req.app, folder), req.query.get("file", "")))
    if not os.path.isfile(full):
        raise HTTPException(404, t("Datei nicht gefunden"))
    st = os.stat(full)
    m = meta_cache.meta(full)
    out = {k: (v.strftime("%Y-%m-%d %H:%M:%S") if hasattr(v, "strftime") else v)
           for k, v in m.items()}
    taken = out.pop("_taken", None)
    return {
        "kind": kind_of(full),
        "meta": out,
        "dates": {
            "modified": datetime.fromtimestamp(st.st_mtime).strftime("%Y-%m-%d %H:%M:%S"),
            "created": datetime.fromtimestamp(getattr(st, "st_birthtime", None) or st.st_ctime).strftime("%Y-%m-%d %H:%M:%S"),
            "taken": taken,
        },
    }


def _plan_from(req: Req):
    folder = resolve(req.app, req.get("path", ""))
    items = req.get("files") or []
    if not isinstance(items, list) or not all(isinstance(i, str) for i in items):
        raise HTTPException(400, t("files muss eine Liste von Dateinamen sein"))
    settings = req.app.settings()
    options = {**{k: settings[k] for k in RENAME_OPTION_KEYS}, **(req.get("options") or {})}
    try:
        return folder, planner.compute_plan(folder, items, req.get("rules") or [], options)
    except planner.PlanError as e:
        raise HTTPException(400, str(e))


@route("POST", "/api/preview")
def preview(req: Req):
    _, plan = _plan_from(req)
    return {
        "items": plan,
        "changed": sum(1 for p in plan if p["changed"]),
        "errors": sum(1 for p in plan if p["error"]),
    }


@route("POST", "/api/rename")
def rename(req: Req):
    app = req.app
    if busy(app, resolve(app, req.get("path", ""))):
        raise HTTPException(409, t("In diesem Ordner läuft gerade eine Konvertierung"))
    folder, plan = _plan_from(req)
    bad = [p for p in plan if p["error"]]
    if bad:
        raise HTTPException(400, t("{n} Konflikt(e) – bitte Vorschau prüfen", n=len(bad)))
    phases = planner.build_phases(plan)
    created: List[str] = []
    if phases:
        try:
            created = planner.execute(folder, phases)
        except OSError as e:
            raise HTTPException(500, t("Umbenennen fehlgeschlagen, alles zurückgesetzt: {error}", error=e))
        app.history.add(relpath(app, folder), phases, created)

    # Automatic MP3 -> M4B for every folder whose MP3s were part of the batch
    jobs = []
    convert = req.get("convert")
    if convert is None:
        convert = app.settings()["auto_convert"]
    if convert:
        dirs = []
        for p in plan:
            if not p["dir"] and p["new"].lower().endswith(".mp3"):
                d = os.path.join(folder, os.path.dirname(p["new"]))
                if d not in dirs:
                    dirs.append(d)
        for d in dirs:
            if list_mp3s(d):
                jobs.append(app.converter.enqueue(d, relpath(app, d), t("Nach Umbenennen")).to_dict())
    return {"renamed": sum(1 for p in plan if p["changed"]), "jobs": jobs,
            "job": jobs[0] if jobs else None}


def _phases_of(entry: dict) -> List[list]:
    if "phases" in entry:
        return entry["phases"]
    return [entry.get("pairs", [])]  # history written by version 1


@route("GET", "/api/history")
def history(req: Req):
    out = []
    for h in req.app.history.list():
        phases = _phases_of(h)
        out.append({"id": h.get("id"), "time": h["time"], "folder": h["folder"],
                    "count": h.get("count") or sum(len(p) for p in phases),
                    "sample": h.get("sample") or [list(x) for x in phases[0][:3]] if phases else []})
    return out


@route("POST", "/api/undo")
def undo(req: Req):
    app = req.app
    with app.history.lock:
        last = app.history.last()
        if not last:
            raise HTTPException(404, t("Nichts rückgängig zu machen"))
        folder = resolve(app, last["folder"])
        if busy(app, folder):
            raise HTTPException(409, t("In diesem Ordner läuft gerade eine Konvertierung"))
        phases = [[tuple(p) for p in ph] for ph in _phases_of(last)]
        try:
            n = planner.undo(folder, phases, last.get("created") or [])
        except planner.PlanError as e:
            raise HTTPException(409, str(e))
        except OSError as e:
            raise HTTPException(500, t("Rückgängig fehlgeschlagen: {error}", error=e))
        app.history.remove(last["id"])
    return {"restored": n, "folder": last["folder"]}


@route("GET", "/api/presets")
def read_presets(req: Req):
    return req.app.presets.get()


@route("PUT", "/api/presets")
def write_presets(req: Req):
    data = req.body if isinstance(req.body, dict) else {}
    if not all(isinstance(v, list) for v in data.values()):
        raise HTTPException(400, t("Voreinstellungen müssen Regel-Listen sein"))
    return req.app.presets.replace(data)
