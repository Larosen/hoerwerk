"""Security log: one logfmt line per login event, for people and for CrowdSec.

    time=2026-10-31T17:02:03Z app=hoerwerk event=login_failed user="max" ip=203.0.113.7 reason=bad_password

The lines go to ``/config/logs/auth.log`` (rotated at 10 MB, 5 files kept) and to stdout, so both a
file-reading shipper (Vector, Promtail, the CrowdSec agent) and ``docker logs`` see them. Values are
quoted and stripped of control characters – a user name can't forge a second line.
"""
from __future__ import annotations

import logging
import logging.handlers
import os
import re
import sys
import threading
import time
from typing import Optional

import applog

EVENTS = ("login_ok", "login_failed", "login_blocked", "logout", "setup", "password_changed",
          "user_created", "user_changed", "user_deleted", "sessions_revoked", "forbidden")
_SAFE = re.compile(r"^[A-Za-z0-9_.:/@-]{1,80}$")
_lock = threading.Lock()
_logger: Optional[logging.Logger] = None


def _value(v) -> str:
    text = re.sub(r"[\x00-\x1f\x7f]", "?", str(v))[:200]
    return text if _SAFE.match(text) else '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def line(event: str, ts: Optional[float] = None, **fields) -> str:
    stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() if ts is None else ts))
    parts = [f"time={stamp}", "app=hoerwerk", f"event={event}"]
    parts += [f"{k}={_value(v)}" for k, v in fields.items() if v is not None and v != ""]
    return " ".join(parts)


def setup(config_dir: str) -> None:
    """Open the log file (once). Without a writable folder only stdout is used."""
    global _logger
    with _lock:
        if _logger:
            return
        lg = logging.getLogger("hoerwerk.auth")
        lg.setLevel(logging.INFO)
        lg.propagate = False
        try:
            folder = os.path.join(config_dir, "logs")
            os.makedirs(folder, exist_ok=True)
            fh = logging.handlers.RotatingFileHandler(os.path.join(folder, "auth.log"), maxBytes=10 << 20,
                                                      backupCount=5, encoding="utf-8")
            fh.setFormatter(logging.Formatter("%(message)s"))
            lg.addHandler(fh)
        except OSError as e:
            applog.warn(f"auth.log nicht beschreibbar: {e}", "Anmeldung")
        _logger = lg


def event(name: str, **fields) -> str:
    """Write one event; returns the line (handy for tests)."""
    text = line(name, **fields)
    if _logger:
        _logger.info(text)
    out = sys.__stdout__
    if out:
        try:
            out.write(text + "\n")
            out.flush()
        except (OSError, ValueError):
            pass
    level = "warn" if name in ("login_failed", "login_blocked", "forbidden") else "info"
    applog.log.add(text, level, "Anmeldung", echo=False)
    return text
