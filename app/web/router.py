"""Tiny router: ``@route("GET", "/api/x/{id}")`` registers a handler ``fn(req, id=...)``."""
from __future__ import annotations

import re
from typing import Dict, List, Optional, Tuple


class HTTPException(Exception):
    def __init__(self, status: int, detail: str) -> None:
        super().__init__(detail)
        self.status_code, self.detail = status, detail


class File:
    """A handler answer that is not JSON: bytes with their type (e.g. a cover image)."""
    def __init__(self, body: bytes, ctype: str, max_age: int = 0) -> None:
        self.body, self.ctype, self.max_age = body, ctype, max_age


class Reply:
    """A JSON answer with extra headers (e.g. the session cookie at login)."""
    def __init__(self, data, headers: Optional[Dict[str, str]] = None) -> None:
        self.data, self.headers = data, headers or {}


class Req:
    """What a handler gets: the app (services), parsed JSON body, query string, who asks and from where."""
    def __init__(self, app, body, query: Dict[str, str], user: Optional[dict] = None, ip: str = "",
                 https: bool = False, token: str = "", agent: str = "", proxied: bool = False) -> None:
        self.app = app
        self.body = body if body is not None else {}
        self.query = query
        self.user = user
        self.ip = ip
        self.https = https
        self.token = token
        self.agent = agent
        self.proxied = proxied          # came through a reverse proxy (any forwarding header)
        self.stream = None              # raw-body routes (uploads): the socket to read ``length`` bytes from
        self.length = 0

    @property
    def is_admin(self) -> bool:
        return bool(self.user and self.user.get("role") == "admin")

    def get(self, key: str, default=None):
        return self.body.get(key, default) if isinstance(self.body, dict) else default


# Who may call a route: "public" (no login), "user" (any signed-in account), "admin" (the default)
ACCESS = ("public", "user", "admin")
ROUTES: List[Tuple[str, "re.Pattern[str]", object, str]] = []


def route(method: str, pattern: str, access: str = "admin", raw_body: int = 0):
    """``raw_body``: the handler streams the body itself (``req.stream``/``req.length``), up to this many bytes."""
    if access not in ACCESS:
        raise ValueError(access)
    rx = re.compile("^" + re.sub(r"\{(\w+)\}", r"(?P<\1>[^/]+)", pattern) + "$")

    def deco(fn):
        fn.raw_body = raw_body
        ROUTES.append((method, rx, fn, access))
        return fn
    return deco


def match(method: str, path: str) -> Optional[tuple]:
    """(handler, path parameters, access level) or None."""
    for m, rx, fn, access in ROUTES:
        hit = rx.match(path)
        if hit and m == method:
            return fn, hit.groupdict(), access
    return None
