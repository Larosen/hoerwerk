"""Black-box harness for the API contract: starts a Hörwerk server (any implementation) and talks HTTP to it.

Nothing here imports the application. The server is started as a process with the same environment the
Docker image uses; the tests only see HTTP answers and the folders (inbox, library, config).

    HW_SERVER_CMD   command that starts the server (default: python3 app/main.py) – e.g. the Rust binary
    HW_URL          use an already running server instead (folders: HW_ROOT)
"""
from __future__ import annotations

import http.client
import json
import os
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
from typing import Optional

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ADMIN = {"name": "admin", "password": "Vertrag-Admin-Kaffee-7"}
OWN = {"X-Requested-With": "Hoerwerk"}


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Server:
    """One server with throwaway folders. ``env`` adds/overrides variables (e.g. a prepared CONFIG_DIR)."""

    def __init__(self, env: Optional[dict] = None, config_dir: Optional[str] = None) -> None:
        self.dir = tempfile.mkdtemp(prefix="hw-contract-")
        self.config = config_dir or os.path.join(self.dir, "config")
        self.inbox = os.path.join(self.dir, "inbox")
        self.library = os.path.join(self.dir, "library")
        for d in (self.config, self.inbox, self.library):
            os.makedirs(d, exist_ok=True)
        self.port = free_port()
        self.env = {
            **os.environ, "PORT": str(self.port), "CONFIG_DIR": self.config, "INBOX_DIR": self.inbox,
            "LIBRARY_DIR": self.library, "AUTH_USER": ADMIN["name"], "AUTH_PASS": ADMIN["password"],
            # offline and deterministic: books are filed with their own tags, no shop lookups
            "MATCH_MODE": "off", "METADATA_FALLBACK": "false", "SETTLE_SECONDS": "0", "SCAN_INTERVAL": "1",
            "DUPLICATES": "ask", "APP_LANGUAGE": "de", **(env or {}),
        }
        self.proc: Optional[subprocess.Popen] = None
        self.log = os.path.join(self.dir, "server.log")

    def start(self) -> "Server":
        cmd = shlex.split(os.environ.get("HW_SERVER_CMD") or f"{shlex.quote(sys.executable)} app/main.py")
        self._out = open(self.log, "w")
        self.proc = subprocess.Popen(cmd, cwd=ROOT, env=self.env, stdout=self._out, stderr=subprocess.STDOUT)
        for _ in range(300):
            if self.proc.poll() is not None:
                raise RuntimeError("server ended:\n" + self.output())
            try:
                if self.request("GET", "/api/health")[0] == 200:
                    return self
            except OSError:
                pass
            time.sleep(0.1)
        raise RuntimeError("server did not start:\n" + self.output())

    def output(self) -> str:
        try:
            with open(self.log, encoding="utf-8", errors="replace") as fh:
                return fh.read()[-4000:]
        except OSError:
            return ""

    def stop(self, sig: int = signal.SIGTERM, timeout: float = 10) -> int:
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(sig)
            try:
                self.proc.wait(timeout)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        if getattr(self, "_out", None):
            self._out.close()
        return self.proc.returncode if self.proc else 0

    def cleanup(self) -> None:
        self.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    # -- HTTP ------------------------------------------------------------------
    def request(self, method: str, path: str, body=None, headers: Optional[dict] = None, cookie: str = "",
                raw: Optional[bytes] = None, host: Optional[str] = None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        h = {"Host": host if host is not None else f"127.0.0.1:{self.port}"}
        if body is not None:
            h["Content-Type"] = "application/json"
        if cookie:
            h["Cookie"] = cookie
        h.update(headers or {})
        c.request(method, path, body=data, headers=h)
        r = c.getresponse()
        out = (r.status, {k.lower(): v for k, v in r.getheaders()}, r.read())
        c.close()
        return out


class Client:
    """A signed-in browser: keeps its session cookie, sends the page's own header with every change."""

    def __init__(self, server: Server, name: str = "", password: str = "") -> None:
        self.s = server
        self.cookie = ""
        if name:
            self.login(name, password)

    def login(self, name: str, password: str) -> dict:
        status, headers, body = self.s.request("POST", "/api/auth/login", {"name": name, "password": password},
                                               headers=OWN)
        if status != 200:
            raise AssertionError(f"login {name}: {status} {body[:200]!r}")
        self.cookie = headers["set-cookie"].split(";")[0]
        return json.loads(body)

    def call(self, method: str, path: str, body=None, expect: Optional[int] = 200, **kw):
        headers = {**OWN, **kw.pop("headers", {})} if method != "GET" else kw.pop("headers", {})
        status, hdrs, raw = self.s.request(method, path, body, headers=headers, cookie=self.cookie, **kw)
        data = json.loads(raw) if raw and hdrs.get("content-type", "").startswith("application/json") else raw
        if expect is not None and status != expect:
            raise AssertionError(f"{method} {path}: {status} (expected {expect}) {raw[:300]!r}")
        return data if expect is not None else (status, data)

    def get(self, path: str, **kw):
        return self.call("GET", path, **kw)

    def wait(self, check, seconds: float = 60, step: float = 0.3, what: str = ""):
        end = time.time() + seconds
        last = None
        while time.time() < end:
            last = check()
            if last:
                return last
            time.sleep(step)
        raise AssertionError(f"timed out waiting for {what or check}: last={last!r}")


def tone(path: str, seconds: float = 1.5, tags: Optional[dict] = None) -> None:
    """A short MP3 with tags (system ffmpeg)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    meta = [x for k, v in (tags or {}).items() for x in ("-metadata", f"{k}={v}")]
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", f"sine=f=440:d={seconds}",
                    "-ar", "44100", "-ac", "2", *meta, path], check=True)


def deliver(inbox: str, name: str, files: dict) -> str:
    """A finished download: built beside the inbox, then moved in with one rename (like SABnzbd)."""
    tmp = os.path.join(os.path.dirname(inbox), ".incoming", name)
    for file, tags in files.items():
        tone(os.path.join(tmp, file), tags=tags)
    dest = os.path.join(inbox, name)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    os.rename(tmp, dest)
    return dest


def quote(path: str) -> str:
    return urllib.parse.quote(path, safe="")
