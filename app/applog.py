"""The app's log: what happened, for the UI (Protokoll) and for ``docker logs``.

Every entry goes to stdout as before (so ``docker logs`` keeps working) and into a
ring buffer the web UI reads. Book and conversion steps, warnings and errors all
end up here; stray output on stderr (tracebacks) is captured as errors.
"""
from __future__ import annotations

import sys
import threading
import time
from collections import deque
from typing import Deque, List, Optional, TextIO

LEVELS = ("info", "warn", "error")


class AppLog:
    def __init__(self, size: int = 3000, out: Optional[TextIO] = None) -> None:
        self.entries: Deque[dict] = deque(maxlen=size)
        self.seq = 0
        self.lock = threading.Lock()
        self.out = out                      # None: the real stdout of the process

    def add(self, text: str, level: str = "info", source: str = "", echo: bool = True) -> dict:
        text = str(text).rstrip()
        level = level if level in LEVELS else "info"
        with self.lock:
            self.seq += 1
            entry = {"id": self.seq, "time": time.time(), "level": level, "source": source, "text": text}
            self.entries.append(entry)
        if echo:
            out = self.out or sys.__stdout__
            if out:
                stamp = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(entry["time"]))
                tag = f" [{source}]" if source else ""
                try:
                    out.write(f"{stamp} {level.upper():5}{tag} {text}\n")
                    out.flush()
                except (OSError, ValueError):
                    pass
        return entry

    def since(self, after: int = 0, level: str = "info", limit: int = 1000) -> List[dict]:
        """Entries newer than ``after`` with at least ``level`` (info < warn < error)."""
        floor = LEVELS.index(level) if level in LEVELS else 0
        with self.lock:
            out = [e for e in self.entries if e["id"] > after and LEVELS.index(e["level"]) >= floor]
        return out[-limit:]

    def text(self) -> str:
        with self.lock:
            entries = list(self.entries)
        return "".join(
            f"{time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(e['time']))} {e['level'].upper():5}"
            f"{' [' + e['source'] + ']' if e['source'] else ''} {e['text']}\n" for e in entries)


log = AppLog()


def info(text: str, source: str = "") -> None:
    log.add(text, "info", source)


def warn(text: str, source: str = "") -> None:
    log.add(text, "warn", source)


def error(text: str, source: str = "") -> None:
    log.add(text, "error", source)


class _Stderr:
    """Keeps writing to the real stderr and records every complete line as an error."""

    def __init__(self, real: TextIO, target: AppLog) -> None:
        self.real = real
        self.target = target
        self.buf = ""

    def write(self, s: str) -> int:
        try:
            self.real.write(s)
        except (OSError, ValueError):
            pass
        self.buf += s
        while "\n" in self.buf:
            line, self.buf = self.buf.split("\n", 1)
            if line.strip():
                self.target.add(line, "error", echo=False)
        return len(s)

    def flush(self) -> None:
        try:
            self.real.flush()
        except (OSError, ValueError):
            pass

    def __getattr__(self, name):
        return getattr(self.real, name)


def capture_stderr() -> None:
    if not isinstance(sys.stderr, _Stderr):
        sys.stderr = _Stderr(sys.stderr, log)   # type: ignore[assignment]
