"""HTTP server (stdlib only) with the security checks every request goes through."""
from __future__ import annotations

import importlib
import ipaddress
import json
import mimetypes
import os
import sys
import threading
import time
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Optional
from urllib.parse import parse_qs, quote, unquote, urlparse

import i18n
from events import Stream
from web import router
from web.router import File, HTTPException, Reply, Req
import applog
import authlog
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
COOKIE = "hw_session"
LOGIN_PAGE = "/login"
# Pages of the app – each address gets the same page shell, the browser shows the right part.
# Who may open them (the API checks every right again): "user" = any account, "admin" = admins only.
APP_PAGES = {"books": "admin", "search": "user", "discover": "user", "requests": "user",
             "account": "user", "settings": "admin"}
SETTINGS_SECTIONS = frozenset({"health", "general", "library", "convert", "downloads", "services", "discover",
                               "notify", "users", "logs"})
# Without login only what the login page itself needs – no app code, no page shell
PUBLIC_STATIC = frozenset({"style.css", "icon.svg", "icon.png", "theme.js", "core.js", "login.js"})


def page_access(path: str) -> Optional[str]:
    """Who may open this app page ("user"/"admin"), None when it is no page of the app."""
    parts = [p for p in path.split("/") if p]
    if not parts or parts[0] not in APP_PAGES:
        return None
    if parts[0] == "settings":
        if len(parts) > 2 or (len(parts) == 2 and parts[1] not in SETTINGS_SECTIONS):
            return None
    elif len(parts) > 1:
        return None
    return APP_PAGES[parts[0]]


# always allowed as Host: a DNS-rebinding page arrives under its own name, never as these
LOOPBACK_NAMES = frozenset({"localhost", "127.0.0.1", "::1"})


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, app) -> None:
        self.app = app
        # proxies in the LAN/Docker network that send X-Forwarded-For but are not in TRUSTED_PROXIES
        # (shown in the health check, logged once an hour each) – e.g. Newt after it got a new IP
        if app is not None:
            app.untrusted_proxies = {}
        self._untrusted_lock = threading.Lock()
        super().__init__(address, Handler)

    def note_untrusted(self, peer: str) -> None:
        now = time.time()
        with self._untrusted_lock:
            seen = self.app.untrusted_proxies
            last_logged = seen.get(peer, {}).get("logged", 0)
            if len(seen) > 50 and peer not in seen:
                return
            seen[peer] = {"time": now, "logged": last_logged}
            if now - last_logged < 3600:
                return
            seen[peer]["logged"] = now
        applog.warn(t("{peer} schickt X-Forwarded-For, steht aber nicht in TRUSTED_PROXIES – alle Besucher über diesen Proxy erscheinen mit seiner Adresse (Anmelde-Sperre, CrowdSec). TRUSTED_PROXIES={peer} setzen.", peer=peer), "Anmeldung")  # noqa: E501

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
        if self.close_connection:
            # without this a reverse proxy reuses the connection we are about to close -> 502 Bad Gateway
            self.send_header("Connection", "close")
        self.end_headers()

    def _send(self, status: int, body: bytes, ctype: str, extra: Optional[dict] = None) -> None:
        self._headers(status, ctype, extra, len(body))
        if self.command != "HEAD":
            self.wfile.write(body)

    def _json(self, status: int, data) -> None:
        self._send(status, json.dumps(data, ensure_ascii=False).encode(), "application/json; charset=utf-8")

    # -- who is asking ------------------------------------------------------
    @staticmethod
    def _addr(text: str):
        try:
            ip = ipaddress.ip_address(text.strip().strip("[]").split("%")[0])
        except ValueError:
            return None
        return ip.ipv4_mapped if getattr(ip, "ipv4_mapped", None) else ip

    def _trusted(self, ip) -> bool:
        return ip is not None and any(ip in net for net in self.config.trusted_proxies)

    def client_ip(self) -> str:
        """The real client: behind a trusted proxy the right-most X-Forwarded-For entry it did not add itself."""
        peer = self._addr(self.client_address[0])
        if not self._trusted(peer):
            # a proxy in the own network that is not trusted: the visitors' real addresses get lost – say so
            if peer is not None and not peer.is_global and not peer.is_loopback and self._forwarded():
                self.server.note_untrusted(str(peer))
            return str(peer) if peer else self.client_address[0]
        chain = [self._addr(p) for p in (self.headers.get("X-Forwarded-For") or "").split(",") if p.strip()]
        real = self._addr(self.headers.get("X-Real-IP") or "")
        for ip in reversed(chain):
            if ip is None:
                break
            if not self._trusted(ip):
                return str(ip)
        return str(real or (chain[0] if chain and chain[0] else peer))

    def _forwarded(self) -> bool:
        return any(self.headers.get(h) for h in ("X-Forwarded-For", "Forwarded", "X-Real-IP"))

    def conn_info(self) -> dict:
        """How this request arrived – for the health check and "So sieht dich Hörwerk"."""
        peer = self._addr(self.client_address[0])
        return {"peer": str(peer or self.client_address[0]), "trusted": self._trusted(peer),
                "proxied": self._forwarded(), "host": (self.headers.get("Host") or "")[:200],
                "forwarded_for": (self.headers.get("X-Forwarded-For") or "")[:300],
                "forwarded_proto": (self.headers.get("X-Forwarded-Proto") or "")[:20]}

    def is_https(self) -> bool:
        if self.config.cookie_secure in ("true", "false"):
            return self.config.cookie_secure == "true"
        proto = (self.headers.get("X-Forwarded-Proto") or "").split(",")[0].strip().lower()
        return proto == "https" and self._trusted(self._addr(self.client_address[0]))

    def _token(self) -> str:
        for part in (self.headers.get("Cookie") or "").split(";"):
            k, _, v = part.strip().partition("=")
            if k == COOKIE:
                return v.strip()
        return ""

    def _user(self, token: str) -> Optional[dict]:
        return self.server.app.accounts.session_user(token, self.client_ip()) if token else None

    def _host_ok(self) -> bool:
        allowed = self.config.allowed_hosts
        if not allowed:
            return True
        host = (self.headers.get("Host") or "").lower()
        host = host.rsplit(":", 1)[0] if not host.endswith("]") else host
        return host.strip("[]") in allowed | LOOPBACK_NAMES

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

    def _redirect(self, where: str) -> None:
        self._send(303, b"", "text/plain", {"Location": where})

    def _dispatch(self) -> None:
        url = urlparse(self.path)
        # Early refusals leave a request body unread: such a connection is not reused (and says so, see
        # _headers). Requests without a body stay open for the next one – proxies like Traefik reuse them.
        self.close_connection = (self.headers.get("Content-Length") or "0").strip() != "0" \
            or bool(self.headers.get("Transfer-Encoding"))
        # the health check answers under any name: Docker asks as 127.0.0.1, Pangolin/Newt under the target name
        if not self._host_ok() and not (url.path == "/api/health" and self.command in ("GET", "HEAD")):
            return self._send(421, b"Unbekannter Host", "text/plain")
        token = self._token()
        reading = self.command in ("GET", "HEAD")
        if reading and url.path == "/i18n.js":
            return self._i18n()
        if reading and url.path == "/me.js":
            user = self._user(token)
            me = {k: user.get(k) for k in ("id", "name", "role", "perms", "quota", "totp")} if user else None
            return self._send(200, f"const ME = {json.dumps(me, ensure_ascii=False)};\n".encode(),
                              "text/javascript; charset=utf-8")
        if reading and url.path.startswith("/static/"):
            if url.path[len("/static/"):] not in PUBLIC_STATIC and self._user(token) is None:
                return self._json(401, {"error": t("Bitte anmelden"), "login": True})
            return self._static(url.path)
        if reading and url.path in ("/", LOGIN_PAGE):
            signed_in = self._user(token) is not None
            if url.path == "/" and not signed_in:
                return self._redirect(LOGIN_PAGE)
            if url.path == LOGIN_PAGE and signed_in:
                return self._redirect("/")
            return self._static("/" if url.path == "/" else "/static/login.html")
        access = page_access(url.path) if reading else None
        if access:
            user = self._user(token)
            if user is None:                       # back to this page after signing in
                return self._redirect(f"{LOGIN_PAGE}?next={quote(url.path, safe='/')}")
            if access == "admin" and user["role"] != "admin":
                return self._redirect("/")
            return self._static("/")
        if self.command not in ("GET", "HEAD") and not self._same_origin():
            return self._json(403, {"error": t("Anfrage von fremder Seite abgelehnt")})
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            return self._json(400, {"error": t("Ungültige Länge")})
        query = {k: v[0] for k, v in parse_qs(url.query).items()}
        found = router.match(self.command, url.path)
        if not found:
            return self._json(404, {"error": t("Nicht gefunden")})
        fn, params, access = found
        # who is asking is checked before a single byte of the body is read (uploads can be large)
        user = self._user(token) if access != "public" or token else None
        ip = self.client_ip()
        if access != "public" and user is None:
            return self._json(401, {"error": t("Bitte anmelden"), "login": True})
        if access == "admin" and user["role"] != "admin":
            authlog.event("forbidden", user=user["name"], ip=ip, path=url.path)
            return self._json(403, {"error": t("Dafür fehlen die Rechte")})
        if access == "admin" and not user.get("totp") and self.server.app.settings().get("admin_totp_required"):
            return self._json(403, {"error": t("Admins brauchen einen Zweitfaktor – bitte unter Konto einrichten"),
                                    "totp_setup": True})
        raw = getattr(fn, "raw_body", 0)
        if length > (raw or MAX_BODY):
            return self._json(413, {"error": t("Anfrage zu groß")})
        if raw and not self.headers.get("Content-Length"):
            return self._json(411, {"error": t("Ungültige Länge")})
        self.close_connection = False
        body = None
        if length and not raw:
            try:
                body = json.loads(self.rfile.read(length))
            except (json.JSONDecodeError, UnicodeDecodeError):
                return self._json(400, {"error": t("Ungültiges JSON")})
        req = Req(self.server.app, body, query, user=user, ip=ip, https=self.is_https(), token=token,
                  agent=self.headers.get("User-Agent") or "",
                  proxied=self._forwarded(), conn=self.conn_info())
        if raw:
            req.stream, req.length = self.rfile, length          # the handler reads the body itself
            self.close_connection = True                          # never reuse: an aborted upload leaves bytes behind
        try:
            result = fn(req, **params)
            if access == "admin" and self.command not in ("GET", "HEAD"):
                self.server.app.audit(req, f"{self.command} {url.path}")
            if isinstance(result, Reply):
                return self._send(200, json.dumps(result.data, ensure_ascii=False).encode(),
                                  "application/json; charset=utf-8", result.headers)
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
