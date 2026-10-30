"""Push notifications via ntfy (https://ntfy.sh or a self-hosted server).

Every event can be switched on or off on its own. Messages are sent in the
background – a slow or unreachable ntfy server never holds up a conversion.
The access token / password stays on the server, never follows a changed
address and never follows a redirect.
"""
from __future__ import annotations

import base64
import json
import re
import threading
import time
from typing import Callable, Dict, List, Optional
from urllib import parse, request
from urllib.error import HTTPError, URLError

import db as dbm
from i18n import t

# event -> (switched on by default, ntfy tags shown as emoji)
EVENTS: Dict[str, tuple] = {
    "filed": (True, ["books"]),
    "review": (True, ["mag"]),
    "duplicate": (True, ["twisted_rightwards_arrows"]),
    "failed": (True, ["warning"]),
    "download_sent": (False, ["inbox_tray"]),
    "download_done": (False, ["white_check_mark"]),
    "download_failed": (True, ["x"]),
    "convert_done": (False, ["headphones"]),
    "convert_failed": (True, ["warning"]),
    "abs_error": (True, ["warning"]),
}
PRIORITIES = (1, 2, 3, 4, 5)        # min, low, default, high, max
SECRETS = ("token", "password")


class NotifyError(Exception):
    pass


def _origin(url: str) -> str:
    p = parse.urlparse(url or "")
    return f"{p.scheme}://{(p.hostname or '').lower()}:{p.port or ''}"


def _base(url: str) -> str:
    url = (url or "").strip().rstrip("/")
    if url and not re.match(r"(?i)^https?://", url):
        url = "https://" + url
    return url


def _valid_url(url: str) -> bool:
    """http(s) with a host name and – if given – a numeric port; nothing else (no user:password@)."""
    try:
        p = parse.urlparse(url)
        p.port  # noqa: B018 – raises ValueError for "host:alert(1)"
    except ValueError:
        return False
    return p.scheme in ("http", "https") and bool(p.hostname) and "@" not in p.netloc and not re.search(r"\s", url)


class _NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):   # credentials must not travel anywhere else
        return None


def _post(url: str, body: dict, headers: dict, timeout: float = 10) -> None:
    req = request.Request(url, data=json.dumps(body, ensure_ascii=False).encode(), method="POST",
                          headers={"Content-Type": "application/json", "User-Agent": "BetterRenamer/4", **headers})
    try:
        with request.build_opener(_NoRedirect).open(req, timeout=timeout) as resp:
            resp.read(64 * 1024)
    except HTTPError as e:
        if e.code in (401, 403):
            raise NotifyError(t("ntfy: Zugang abgelehnt – Token oder Passwort prüfen")) from e
        if e.code == 429:
            raise NotifyError(t("ntfy: zu viele Nachrichten (Limit des Servers)")) from e
        raise NotifyError(f"ntfy: HTTP {e.code}") from e
    except (URLError, OSError) as e:
        raise NotifyError(t("ntfy nicht erreichbar: {reason}", reason=getattr(e, "reason", e))) from e


class Notifier:
    def __init__(self, db: dbm.Database, post: Callable[..., None] = _post,
                 translate: Callable[..., str] = t) -> None:
        self.db = db
        self._post = post
        self.t = translate                   # language of the messages
        self.last_error: Optional[str] = None
        self.sent = 0

    # -- config ------------------------------------------------------------
    def _load(self) -> dict:
        data = self.db.get("ntfy") or {}
        return data if isinstance(data, dict) else {}

    def events(self) -> Dict[str, bool]:
        chosen = self._load().get("events") or {}
        return {k: bool(chosen.get(k, default)) for k, (default, _tags) in EVENTS.items()}

    def configured(self) -> bool:
        c = self._load()
        return bool(c.get("url") and c.get("topic"))

    def wants(self, event: str) -> bool:
        return self.configured() and self.events().get(event, False)

    def public_config(self) -> dict:
        c = self._load()
        return {"url": c.get("url", ""), "topic": c.get("topic", ""), "user": c.get("user", ""),
                "priority": int(c.get("priority") or 3), "app_url": c.get("app_url", ""),
                "token_set": bool(c.get("token")), "password_set": bool(c.get("password")),
                "events": self.events(), "configured": self.configured(), "last_error": self.last_error}

    def update_config(self, new: dict) -> dict:
        old = self._load()
        url = _base(str(new.get("url") or ""))
        if url and not _valid_url(url):
            raise NotifyError(t("Ungültige ntfy-Adresse"))
        topic = str(new.get("topic") or "").strip()
        p = parse.urlparse(url)
        if url and p.path.strip("/") and not topic:
            # "https://ntfy.sh/mytopic" pasted as a whole: split into server and topic
            topic = p.path.strip("/").split("/")[-1]
            url = _base(f"{p.scheme}://{p.netloc}{p.path.rsplit('/', 1)[0] if '/' in p.path.strip('/') else ''}")
        if topic and not re.fullmatch(r"[-_A-Za-z0-9]{1,64}", topic):
            raise NotifyError(t("Topic: nur Buchstaben, Ziffern, - und _ (max. 64 Zeichen)"))
        app_url = str(new.get("app_url") or "").strip()
        if app_url and not _valid_url(app_url):
            raise NotifyError(t("Adresse der Oberfläche muss mit http:// oder https:// beginnen"))
        same_server = _origin(old.get("url", "")) == _origin(url)
        data = {"url": url, "topic": topic, "user": str(new.get("user") or "").strip(), "app_url": app_url,
                "priority": int(new.get("priority") or 3) if int(new.get("priority") or 3) in PRIORITIES else 3,
                "events": {k: bool((new.get("events") or {}).get(k, v)) for k, v in self.events().items()}}
        for key in SECRETS:     # empty field = keep the stored one, but never for another server
            data[key] = str(new.get(key) or "").strip() or (old.get(key, "") if same_server else "")
        self.db.put("ntfy", data)
        return self.public_config()

    # -- sending -----------------------------------------------------------
    def _headers(self, c: dict) -> dict:
        if c.get("token"):
            return {"Authorization": f"Bearer {c['token']}"}
        if c.get("user") and c.get("password"):
            raw = f"{c['user']}:{c['password']}".encode()
            return {"Authorization": "Basic " + base64.b64encode(raw).decode()}
        return {}

    def _send_now(self, event: str, title: str, message: str, priority: Optional[int] = None) -> None:
        c = self._load()
        body = {"topic": c["topic"], "title": title[:250], "message": message[:3500],
                "priority": priority or int(c.get("priority") or 3), "tags": EVENTS.get(event, (False, []))[1]}
        if c.get("app_url"):
            body["click"] = c["app_url"]
        self._post(c["url"], body, self._headers(c))
        self.sent += 1

    def send(self, event: str, title: str, message: str, priority: Optional[int] = None) -> None:
        """Send in the background if this event is switched on."""
        if not self.wants(event):
            return

        def run() -> None:
            try:
                self._send_now(event, title, message, priority)
                self.last_error = None
            except NotifyError as e:
                self.last_error = str(e)
                print(e, flush=True)
        threading.Thread(target=run, daemon=True, name="ntfy").start()

    def test(self) -> str:
        if not self.configured():
            raise NotifyError(t("ntfy ist noch nicht eingerichtet (Adresse und Topic)"))
        self._send_now("filed", t("Better Renamer: Test"), t("Benachrichtigungen funktionieren."), priority=3)
        self.last_error = None
        return t("Testnachricht gesendet")


class DownloadWatcher:
    """Tells about finished/failed SABnzbd downloads that were sent from the search."""

    def __init__(self, usenet, notifier: Notifier, db: dbm.Database, interval: float = 120.0) -> None:
        self.usenet, self.notifier, self.db, self.interval = usenet, notifier, db, interval

    def check_once(self) -> List[str]:
        n = self.notifier
        if not (n.wants("download_done") or n.wants("download_failed")) or not self.usenet.configured()["sab"]:
            return []
        rows = self.db.all("SELECT data FROM grabs WHERE time > ?", (time.time() - 14 * 86400,))
        ours = {dbm.loads(r["data"], {}).get("nzo_id") for r in rows} - {None}
        told = set(self.db.get("ntfy_downloads_told") or [])
        new = []
        for item in self.usenet.activity().get("items", []):
            if not item.get("done") or item.get("id") not in ours or item["id"] in told:
                continue
            failed = item.get("status") == "Failed"
            if failed:
                n.send("download_failed", t("Download fehlgeschlagen"),
                       f"{item.get('name')}\n{item.get('error') or ''}".strip(), priority=4)
            else:
                n.send("download_done", t("Download fertig"), str(item.get("name")))
            told.add(item["id"])
            new.append(item["id"])
        if new:
            self.db.put("ntfy_downloads_told", sorted(told)[-500:])
        return new

    def run(self) -> None:
        while True:
            try:
                self.check_once()
            except Exception as e:  # noqa: BLE001 – SABnzbd away for a moment: try again later
                print(f"download check: {e}", flush=True)
            time.sleep(self.interval)
