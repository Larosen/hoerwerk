"""Settings and health."""
from __future__ import annotations

import functools
import subprocess

import audible
import sandbox
from config import CHOICES, VERSION
from web.router import HTTPException, Req, route


@route("GET", "/api/settings")
def read_settings(req: Req):
    return req.app.settings()


@route("PUT", "/api/settings")
def write_settings(req: Req):
    data = req.body if isinstance(req.body, dict) else {}
    defaults = req.app.defaults
    clean = {}
    for k, v in data.items():
        if k not in defaults:
            continue
        kind = type(defaults[k])
        try:
            clean[k] = v if kind is str or isinstance(v, kind) and kind is not int else kind(v)
        except (TypeError, ValueError):
            raise HTTPException(400, f"Ungültiger Wert für {k}")
    for k, allowed in {**CHOICES, "ab_region": tuple(audible.REGIONS)}.items():
        if clean.get(k) not in (None,) + allowed:
            raise HTTPException(400, f"Ungültiger Wert für {k}")
    return req.app.update_settings(clean)


@route("GET", "/api/events")
def events(req: Req):
    """Live updates (Server-Sent Events) for the topics the page shows."""
    bus = req.app.events
    wanted = (req.query.get("topics") or ",".join(bus.providers)).split(",")
    topics = [t for t in wanted if t in bus.providers]
    if not topics:
        raise HTTPException(400, "Unbekanntes Thema")
    try:
        return bus.stream(topics)
    except OverflowError as e:
        raise HTTPException(503, str(e))


@functools.lru_cache(maxsize=1)
def _ffmpeg_version() -> str:
    try:
        out = subprocess.run(sandbox.ffmpeg(["-hide_banner", "-version"]), capture_output=True, text=True,
                             timeout=20).stdout
    except (OSError, subprocess.SubprocessError):
        return "nicht gefunden"
    first = (out.splitlines() or [""])[0]
    return first.replace("ffmpeg version ", "").split(" Copyright")[0] or "nicht gefunden"


@route("GET", "/api/system")
def system(req: Req):
    """What runs here: versions and whether ffmpeg is jailed (shown in the settings)."""
    return {"version": VERSION, "ffmpeg": _ffmpeg_version(), "sandbox": sandbox.status(),
            "queues": req.app.engine.stats()}


@route("GET", "/api/health")
def health(req: Req):
    return {"ok": True, "version": VERSION}
