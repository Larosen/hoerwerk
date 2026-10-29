"""Tiny router: ``@route("GET", "/api/x/{id}")`` registers a handler ``fn(req, id=...)``."""
from __future__ import annotations

import re
from typing import Dict, List, Optional, Tuple


class HTTPException(Exception):
    def __init__(self, status: int, detail: str) -> None:
        super().__init__(detail)
        self.status_code, self.detail = status, detail


class Req:
    """What a handler gets: the app (services), parsed JSON body and query string."""
    def __init__(self, app, body, query: Dict[str, str]) -> None:
        self.app = app
        self.body = body if body is not None else {}
        self.query = query

    def get(self, key: str, default=None):
        return self.body.get(key, default) if isinstance(self.body, dict) else default


ROUTES: List[Tuple[str, "re.Pattern[str]", object]] = []


def route(method: str, pattern: str):
    rx = re.compile("^" + re.sub(r"\{(\w+)\}", r"(?P<\1>[^/]+)", pattern) + "$")

    def deco(fn):
        ROUTES.append((method, rx, fn))
        return fn
    return deco


def match(method: str, path: str) -> Optional[tuple]:
    for m, rx, fn in ROUTES:
        hit = rx.match(path)
        if hit and m == method:
            return fn, hit.groupdict()
    return None
