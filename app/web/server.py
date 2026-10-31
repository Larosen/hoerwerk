"""HTTP server (stdlib only) with the security checks every request goes through."""
from __future__ import annotations

import base64
import importlib
import ipaddress
import json
import mimetypes
import os
import secrets
import sys
import time
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Optional
from urllib.parse import parse_qs, unquote, urlparse

import i18n
from events import Stream
from web import router
from web.router import File, HTTPException, Req
import applog
from i18n import t

STATIC_DIR = os.path.realpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "static"))
MAX_BODY = 4 * 1024 * 1024
CSRF_HEADER = "X-Requested-With"
CSRF_VALUE = "Hoerwerk"
SECURITY_HEADERS = {
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "Referrer-Policy": "no-referrer",
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Resource-Policy": "same-origin",
    "Content-Security-Policy": (
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
        "img-src 'self' data: blob: https:; connect-src 'self'; object-src 'none'; "
        "base-uri 'none'; form-action 'self'; frame-ancestors 'none'"),
}
# answered without password/network check: Docker health check, and the logo for the Unraid Docker page
PUBLIC_PATHS = ("/api/health", "/static/icon.png", "/static/icon.svg")


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, app) -> None:
        self.app = app
        super().__init__(address, Handler)

    def handle_error(self, request, client_address) -> None:
        """A client that hangs up (browser tab closed, health check done) is no error worth a traceback."""
        err = sys.exc_info()[1]
        if isinstance(err, (ConnectionError, TimeoutError)):
            return
        applog.error(f"{client_address[0]}: {err!r}\n{traceback.format_exc().rstrip()}", "web")


class Handler(BaseHTTPRequestHandler):
    server_version = "Hoerwerk"
    protocol_version = "HTTP/1.1"
    server: Server

    @property
    def config(self):
        return self.server.app.config

    def log_request(self, code="-", size="-"):  # quieter: only failed requests
        if str(code)[:1] in "45":
            super().log_request(code, size)

    def log_message(self, format, *args):  # noqa: A002 – signature of BaseHTTPRequestHandler
        applog.warn(f"{self.address_string()} {format % args}", "web")

    def _headers(self, status: int, ctype: str, extra: Optional[dict] = None, length: Optional[int] = None) -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        if length is not None:
            self.send_header("Content-Length", str(length))
        if "Cache-Control" not in (extra or {}):
            self.send_header("Cache-Control", "no-store")
        for k, v in SECURITY_HEADERS.items():
            self.send_header(k, v)
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()

    def _send(self, status: int, body: bytes, ctype: str, extra: Optional[dict] = None) -> None:
        self._headers(status, ctype, extra, len(body))
        if self.command != "HEAD":
            self.wfile.write(body)

    def _json(self, status: int, data) -> None:
        self._send(status, json.dumps(data, ensure_ascii=False).encode(), "application/json; charset=utf-8")

    # -- checks ------------------------------------------------------------
    def _authorized(self) -> bool:
        cfg = self.config
        if not cfg.auth:
            return True
        header = self.headers.get("Authorization", "")
        if header.lower().startswith("basic "):
            try:
                user, _, pw = base64.b64decode(header[6:]).decode().partition(":")
                return secrets.compare_digest(user, cfg.auth_user) and secrets.compare_digest(pw, cfg.auth_pass)
            except Exception:  # noqa: BLE001
                return False
        return False

    def _client_ok(self) -> bool:
        if self.config.auth or self.config.allow_public:
            return True
        try:
            ip = ipaddress.ip_address(self.client_address[0].split("%")[0])
        except ValueError:
            return False
        if getattr(ip, "ipv4_mapped", None):
            ip = ip.ipv4_mapped
        return not ip.is_global

    def _host_ok(self) -> bool:
        allowed = self.config.allowed_hosts
        if not allowed:
            return True
        host = (self.headers.get("Host") or "").lower()
        host = host.rsplit(":", 1)[0] if not host.endswith("]") else host
        return host.strip("[]") in allowed

    def _same_origin(self) -> bool:
        """Changing requests must come from our own page (CSRF protection).

        Browsers only add custom headers after a CORS preflight, which this server never
        approves – so a foreign web page can't send them. Origin/Sec-Fetch-Site are
        checked as well where the browser provides them.
        """
        if self.headers.get(CSRF_HEADER) != CSRF_VALUE:
            return False
        site = self.headers.get("Sec-Fetch-Site")
        if site and site not in ("same-origin", "none"):
            return False
        origin = self.headers.get("Origin")
        if origin and origin != "null":
            return urlparse(origin).netloc.lower() == (self.headers.get("Host") or "").lower()
        return True

    # -- responses ---------------------------------------------------------
    def _static(self, path: str) -> None:
        name = "index.html" if path == "/" else path[len("/static/"):]
        full = os.path.realpath(os.path.join(STATIC_DIR, unquote(name)))
        if os.path.commonpath([full, STATIC_DIR]) != STATIC_DIR or not os.path.isfile(full):
            return self._json(404, {"error": t("Nicht gefunden")})
        ctype = mimetypes.guess_type(full)[0] or "application/octet-stream"
        if ctype.startswith("text/") or ctype.endswith("javascript"):
            ctype += "; charset=utf-8"
        with open(full, "rb") as fh:
            self._send(200, fh.read(), ctype)

    def _i18n(self) -> None:
        """Language and catalog for the page (a script, so it is there before anything renders)."""
        lang = i18n.language()
        data = json.dumps({"lang": lang, "text": i18n.catalog(lang)}, ensure_ascii=False)
        self._send(200, f"const I18N = {data};\n".encode(), "text/javascript; charset=utf-8")

    def _stream(self, stream: Stream) -> None:
        """Server-Sent Events: the connection stays open and carries every change."""
        self.close_connection = True
        try:
            self._headers(200, "text/event-stream; charset=utf-8", {"X-Accel-Buffering": "no"})
            for chunk in stream:
                self.wfile.write(chunk)
                self.wfile.flush()
        except OSError:          # browser tab closed
            pass
        finally:
            stream.close()

    def _dispatch(self) -> None:
        url = urlparse(self.path)
        # Early refusals leave the request body unread: don't reuse such a connection
        self.close_connection = True
        if not self._host_ok():
            return self._send(421, b"Unbekannter Host", "text/plain")
        public = url.path in PUBLIC_PATHS
        if not self._client_ok() and not public:
            return self._send(403, t("Zugriff von öffentlichen Adressen nur mit Passwort (AUTH_USER/AUTH_PASS). Bei IPv6 im Heimnetz ggf. ALLOW_PUBLIC=true setzen.").encode(),  # noqa: E501
                              "text/plain; charset=utf-8")
        if not self._authorized() and not public:
            time.sleep(1)   # slows down password guessing
            return self._send(401, b"", "text/plain", {"WWW-Authenticate": 'Basic realm="Hoerwerk"'})
        if self.command in ("GET", "HEAD") and url.path == "/i18n.js":
            return self._i18n()
        if self.command in ("GET", "HEAD") and (url.path == "/" or url.path.startswith("/static/")):
            return self._static(url.path)
        if self.command not in ("GET", "HEAD") and not self._same_origin():
            return self._json(403, {"error": t("Anfrage von fremder Seite abgelehnt")})
        body = None
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            return self._json(400, {"error": t("Ungültige Länge")})
        if length > MAX_BODY:
            return self._json(413, {"error": t("Anfrage zu groß")})
        self.close_connection = False
        if length:
            try:
                body = json.loads(self.rfile.read(length))
            except (json.JSONDecodeError, UnicodeDecodeError):
                return self._json(400, {"error": t("Ungültiges JSON")})
        query = {k: v[0] for k, v in parse_qs(url.query).items()}
        found = router.match(self.command, url.path)
        if not found:
            return self._json(404, {"error": t("Nicht gefunden")})
        fn, params = found
        try:
            result = fn(Req(self.server.app, body, query), **params)
            if isinstance(result, Stream):
                return self._stream(result)
            if isinstance(result, File):
                cache = f"private, max-age={result.max_age}" if result.max_age else "no-store"
                return self._send(200, result.body, result.ctype, {"Cache-Control": cache})
            return self._json(200, result)
        except HTTPException as e:
            return self._json(e.status_code, {"error": e.detail})
        except Exception as e:  # noqa: BLE001
            self.log_error("Fehler in %s: %r", url.path, e)
            return self._json(500, {"error": t("Interner Fehler: {error}", error=e)})

    do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = _dispatch


def make_server(app, host: str = "0.0.0.0", port: Optional[int] = None) -> Server:
    importlib.import_module("web.routes")   # registers the API handlers
    return Server((host, app.config.port if port is None else port), app)
