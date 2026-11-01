"""The library: stats, rescan, books that are not one M4B yet and their conversion."""
from __future__ import annotations

import os
import threading
from typing import Optional

import applog
import cleanup
import encoder
import tagger
from meta import cache as meta_cache
from library_index import COVER_FILES
from natsort import natural_key
from i18n import t
from web.router import File, HTTPException, Req, route
from web.routes.common import library_path


@route("GET", "/api/library")
def stats(req: Req):
    return req.app.library_index.stats()


@route("GET", "/api/library/books", access="user")
def books(req: Req):
    """Browse the whole library (search over title, author, series, path)."""
    try:
        offset = max(0, int(req.query.get("offset") or 0))
        limit = min(2000, max(1, int(req.query.get("limit") or 100)))
    except ValueError:
        raise HTTPException(400, t("Ungültige Daten"))
    return req.app.library_index.books(req.query.get("q", "")[:200], offset, limit)


@route("GET", "/api/library/cleanup")
def cleanup_preview(req: Req):
    """How many redundant desc.txt/reader.txt files the library holds."""
    found = cleanup.leftovers(req.app.config.library)
    return {"count": len(found), "folders": len({os.path.dirname(p) for p in found}), "examples": found[:5]}


@route("POST", "/api/library/cleanup")
def cleanup_run(req: Req):
    """Delete them and stop writing new ones (they would come back with the next book)."""
    req.app.update_settings({"write_desc": False, "write_reader": False})
    res = cleanup.remove(req.app.config.library)
    applog.info((t("Aufgeräumt: 1 überflüssige Textdatei gelöscht") if res["removed"] == 1
                 else t("Aufgeräumt: {n} überflüssige Textdateien gelöscht", n=res["removed"])), "Bibliothek")
    for err in res["errors"]:
        applog.warn(t("Nicht gelöscht: {error}", error=err), "Bibliothek")
    return res


MAX_COVER = 15 * 1024 * 1024


def _image_type(data: bytes) -> Optional[str]:
    if data[:3] == b"\xff\xd8\xff":
        return "image/jpeg"
    if data[:8] == b"\x89PNG\r\n\x1a\n":
        return "image/png"
    return None


@route("GET", "/api/library/cover", access="user")
def cover(req: Req):
    """The cover of a library book: cover.jpg & co. in its folder, else the picture inside the audio file."""
    folder = library_path(req.app, req.query.get("path", ""))
    candidates = [os.path.join(folder, n) for n in COVER_FILES]   # same order as the library index
    for path in candidates:
        if os.path.isfile(path) and not os.path.islink(path) and os.path.getsize(path) <= MAX_COVER:
            with open(path, "rb") as fh:
                data = fh.read()
            if _image_type(data):
                return File(data, _image_type(data), max_age=86400)
    for rel in encoder.collect_audio(folder)[:1]:
        data = tagger.embedded_cover(os.path.join(folder, rel))
        if data and len(data) <= MAX_COVER and _image_type(data):
            return File(data, _image_type(data), max_age=86400)
    raise HTTPException(404, t("Kein Cover"))


@route("POST", "/api/library/scan")
def scan(req: Req):
    threading.Thread(target=req.app.library_index.scan, daemon=True).start()
    return {"ok": True}


@route("GET", "/api/library/unconverted")
def unconverted(req: Req):
    """Books in the library that are still MP3 & co. instead of one M4B."""
    app = req.app
    active = {j["source"]: j for j in app.pipeline.status()["jobs"]
              if j["status"] not in ("done", "failed", "cancelled")}
    root = os.path.realpath(app.config.library)
    index = app.library_index
    with index.lock:
        entries = [e for e in index.entries.values() if e.get("format") != "m4b"]
    out = []
    for e in sorted(entries, key=lambda e: [natural_key(p) for p in e["path"].split(os.sep)]):
        job = active.get(os.path.join(root, e["path"]))
        out.append({k: e.get(k) for k in ("path", "title", "authors", "series", "series_position", "files", "format", "cover")}
                   | {"job": {"id": job["id"], "status": job["status"]} if job else None})
    return {"books": out, "scanning": index.scanning}


@route("GET", "/api/library/tracks")
def tracks(req: Req):
    """Tracks of an unconverted book with name suggestions for the chapters."""
    folder = library_path(req.app, req.query.get("path", ""))
    out = []
    entry = req.app.library_index.entries.get(os.path.relpath(folder, os.path.realpath(req.app.config.library)), {})
    for rel in encoder.collect_audio(folder):
        info = meta_cache.probe(os.path.join(folder, rel))
        tags = info.get("tags", {})
        out.append({
            "file": rel, "duration": info.get("duration") or 0,
            "auto": encoder.chapter_title(rel, tags, "auto"),
            "filename": encoder.chapter_title(rel, tags, "filename"),
            "tag": (tags.get("title") or "").strip(),
        })
    # "automatisch": cleaned as a whole (repeated book title, years, bare numbers -> "Kapitel N")
    for track, name in zip(out, encoder.chapter_titles([x["auto"] for x in out], entry.get("title") or "")):
        track["auto"] = name
    return {"path": os.path.relpath(folder, os.path.realpath(req.app.config.library)), "tracks": out}


@route("POST", "/api/library/convert")
def convert(req: Req):
    """Queue library books for conversion: [{path, chapter_names?, rename_files?}]."""
    items = req.get("items") or []
    if not isinstance(items, list) or not items:
        raise HTTPException(400, t("Keine Bücher ausgewählt"))
    jobs = []
    for it in items[:200]:
        if not isinstance(it, dict):
            continue
        folder = library_path(req.app, str(it.get("path") or ""))
        names = it.get("chapter_names")
        if names is not None:
            if not isinstance(names, list) or not all(isinstance(n, str) for n in names):
                raise HTTPException(400, t("chapter_names muss eine Liste von Texten sein"))
            if len(names) != len(encoder.collect_audio(folder)):
                raise HTTPException(409, t("Die Tracks haben sich geändert – bitte neu laden"))
            names = [n.strip()[:200] for n in names]
        jobs.append(req.app.pipeline.add(folder, origin="library", chapter_names=names,
                                         rename_files=bool(it.get("rename_files")) and bool(names)).to_dict(full=False))
    return {"jobs": jobs}
