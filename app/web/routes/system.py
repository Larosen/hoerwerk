"""Settings and health."""
from __future__ import annotations

import functools
import os
import shutil
import subprocess
import time

import audible
import i18n
import sandbox
from config import CHOICES, VERSION, public_url
from web import hosts
from i18n import t
from web.router import HTTPException, Req, route


@route("GET", "/api/settings", access="user")
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
            raise HTTPException(400, t("Ungültiger Wert für {key}", key=k))
    if "discover_trusted_authors" in clean:
        clean["discover_trusted_authors"] = str(clean["discover_trusted_authors"] or "")[:4000]
    if "public_url" in clean:
        url = public_url(clean["public_url"])
        if clean["public_url"].strip() and not url:
            raise HTTPException(400, t("Öffentliche Adresse: bitte so angeben: https://hoerwerk.example.de"))
        clean["public_url"] = url
    for k, allowed in {**CHOICES, "ab_region": tuple(audible.REGIONS)}.items():
        if clean.get(k) not in (None,) + allowed:
            raise HTTPException(400, t("Ungültiger Wert für {key}", key=k))
    return req.app.update_settings(clean)


@route("GET", "/api/events")
def events(req: Req):
    """Live updates (Server-Sent Events) for the topics the page shows."""
    bus = req.app.events
    wanted = (req.query.get("topics") or ",".join(bus.providers)).split(",")
    topics = [t for t in wanted if t in bus.providers]
    if not topics:
        raise HTTPException(400, t("Unbekanntes Thema"))
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
        return t("nicht gefunden")
    first = (out.splitlines() or [""])[0]
    return first.replace("ffmpeg version ", "").split(" Copyright")[0] or t("nicht gefunden")


@route("GET", "/api/system")
def system(req: Req):
    """What runs here: versions and whether ffmpeg is jailed (shown in the settings)."""
    return {"version": VERSION, "ffmpeg": _ffmpeg_version(), "sandbox": sandbox.status(),
            "queues": req.app.engine.stats()}


@route("GET", "/api/health", access="public")
def health(req: Req):
    """For Docker, Pangolin & co.: alive. The version only for signed-in users (no help for attackers)."""
    return {"ok": True, "version": VERSION} if req.user else {"ok": True}


STARTED = time.time()


def _check(group: str, status: str, title: str, detail: str = "", hint: str = "", id: str = "") -> dict:
    return {"id": id or title, "group": group, "status": status, "title": title, "detail": detail, "hint": hint}


def _dir_check(name: str, path: str, need_write: bool = True) -> list:
    """Folder there, writable, enough space – one line per folder."""
    if not os.path.isdir(path):
        return [_check("storage", "error", t("{name}: {path} fehlt", name=name, path=path),
                       t("Ordner im Docker-Template einbinden."), id="dir-" + name)]
    try:
        gb = shutil.disk_usage(path).free / 1e9
    except OSError:
        gb = -1
    free = t("{gb} GB frei", gb=(f"{gb:.1f}".replace(".", ",") if i18n.language() == "de" else f"{gb:.1f}")) if gb >= 0 else ""
    if need_write and not os.access(path, os.W_OK):
        return [_check("storage", "error", t("{name}: {path} nicht beschreibbar", name=name, path=path),
                       t("Rechte prüfen (PUID/PGID, läuft als UID {uid}).", uid=os.getuid()), id="dir-" + name)]
    if 0 <= gb < 5:
        return [_check("storage", "warn", t("{name}: nur noch {free}", name=name, free=free), path,
                       t("Beim Umwandeln braucht ein Hörbuch kurzzeitig etwa seine doppelte Größe."), id="dir-" + name)]
    return [_check("storage", "ok", f"{name}: {free}", path, id="dir-" + name)]


@route("GET", "/api/diagnostics")
def diagnostics(req: Req):
    """The health check page: is everything set up right? Each check: group, ok/warn/error/info, what to do."""
    app, cfg, conn = req.app, req.app.config, req.conn
    checks = []
    add = lambda *a, **k: checks.append(_check(*a, **k))  # noqa: E731

    # -- connection & proxy (this very request) --
    add("connection", "info", t("Hörwerk sieht dich als {ip}", ip=req.ip), id="ip",
        detail=t("Über {peer}", peer=conn.get("peer", "")) if conn.get("peer") != req.ip else "")
    if conn.get("proxied"):
        if conn.get("trusted"):
            add("connection", "ok", t("Reverse-Proxy {peer} ist vertrauenswürdig", peer=conn.get("peer")), id="proxy",
                detail=t("X-Forwarded-For: {xff}", xff=conn.get("forwarded_for") or "–"))
        else:
            add("connection", "error", t("Reverse-Proxy {peer} steht nicht in TRUSTED_PROXIES", peer=conn.get("peer")), id="proxy",
                detail=t("Die echte Adresse der Besucher geht verloren: Anmelde-Sperre und CrowdSec treffen alle, die über diesen Proxy kommen."),
                hint=f"TRUSTED_PROXIES={conn.get('peer')}")
    elif cfg.trusted_proxies:
        add("connection", "ok", t("Direkt verbunden (ohne Proxy)"), id="proxy",
            detail=t("Vertrauenswürdige Proxys: {nets}", nets=", ".join(str(n) for n in cfg.trusted_proxies)))
    else:
        add("connection", "info", t("Direkt verbunden, kein Reverse-Proxy eingerichtet"), id="proxy",
            detail=t("Für den Zugriff aus dem Internet: Reverse-Proxy mit HTTPS davor und TRUSTED_PROXIES setzen."))
    for peer, info in sorted((getattr(app, "untrusted_proxies", {}) or {}).items()):
        if peer != conn.get("peer"):
            add("connection", "warn", t("{peer} schickte X-Forwarded-For, ist aber nicht vertrauenswürdig", peer=peer),
                detail=t("Zuletzt {ago}. Wenn das dein Proxy ist (z. B. Newt nach einem Neustart mit neuer IP): eintragen.",
                         ago=time.strftime("%d.%m. %H:%M", time.localtime(info["time"]))),
                hint=f"TRUSTED_PROXIES={peer}", id="untrusted-" + peer)
    if conn.get("proxied"):
        if req.https:
            add("connection", "ok", t("HTTPS erkannt – das Anmelde-Cookie ist nur über HTTPS gültig"), id="https")
        else:
            add("connection", "warn", t("Der Proxy meldet kein HTTPS"), id="https",
                detail=t("Ohne „X-Forwarded-Proto: https“ vom vertrauenswürdigen Proxy wird das Anmelde-Cookie ohne „Secure“ gesetzt."),
                hint="COOKIE_SECURE=true")
    pub = app.settings().get("public_url", "")
    if pub:
        import ipaddress
        from urllib.parse import urlsplit
        pub_host = urlsplit(pub).hostname or ""
        try:
            local = not ipaddress.ip_address(pub_host).is_global
        except ValueError:
            local = pub_host in ("localhost", "tower") or "." not in pub_host
        add("connection", "warn" if local or pub.startswith("http:") else "ok",
            t("Öffentliche Adresse: {url}", url=pub), id="public_url",
            detail=t("Das ist keine Adresse aus dem Internet – Einladungslinks funktionieren so nur zu Hause.") if local
            else t("Ohne HTTPS gehen Passwörter unverschlüsselt durchs Netz.") if pub.startswith("http:") else
            t("Einladungslinks zeigen hierher."))
    else:
        add("connection", "warn", t("Keine öffentliche Adresse eingetragen – Einladungslinks gehen erst danach"), id="public_url",
            detail=t("Einstellungen → Allgemein, oder Hörwerk einmal über den Reverse-Proxy öffnen."),
            hint="https://hoerwerk.example.de")
    host = hosts.host_name(conn.get("host", ""))
    if "*" in cfg.allowed_hosts:
        add("connection", "warn", t("ALLOWED_HOSTS=* – Hörwerk antwortet unter jedem Namen"), id="hosts",
            detail=t("Kein Schutz gegen DNS-Rebinding. Besser die eigenen Namen eintragen."), hint=f"ALLOWED_HOSTS={host}")
    elif cfg.allowed_hosts:
        add("connection", "ok", t("Erlaubte Namen: {hosts}", hosts=", ".join(sorted(cfg.allowed_hosts))), id="hosts",
            detail=t("Aufgerufen als {host}", host=host))
    else:
        add("connection", "ok", t("Hörwerk antwortet unter IP-Adressen, Namen aus dem Heimnetz und der öffentlichen Adresse"),
            id="hosts", detail=t("Schutz gegen DNS-Rebinding. Weitere Namen: ALLOWED_HOSTS."))
    for name, info in sorted((getattr(app, "refused_hosts", None) or {}).items(), key=lambda kv: -kv[1]["time"])[:5]:
        add("connection", "warn", t("Aufruf als „{host}“ abgelehnt", host=name), id="refused-" + name,
            detail=t("Zuletzt {ago}. Wenn das deine Adresse ist: als öffentliche Adresse eintragen oder ALLOWED_HOSTS setzen.",
                     ago=time.strftime("%d.%m. %H:%M", time.localtime(info["time"]))),
            hint=f"ALLOWED_HOSTS={name}")

    if app.start_workers and not app.library_ready.is_set():
        add("connection", "error" if app.library_error else "warn",
            t("Bibliothek wird eingelesen – Bücher werden danach verarbeitet") if not app.library_error
            else t("Bibliothek nicht lesbar – es wird nichts verarbeitet"), id="library-ready",
            detail=app.library_error or t("Erst wenn Hörwerk die Bibliothek kennt, prüft es neue Bücher auf Doppelte."))

    if getattr(app, "db_backup_error", ""):
        add("storage", "error", t("Tägliche Datenbank-Sicherung fehlgeschlagen"), id="db-backup",
            detail=t("{error} – neuer Versuch jede Stunde.", error=app.db_backup_error))

    # -- security --
    users = app.accounts.list()
    no_totp = [u["name"] for u in users if u.get("role") == "admin" and not u.get("totp") and not u.get("disabled")]
    if no_totp:
        add("security", "warn", t("Admins ohne Zweitfaktor: {names}", names=", ".join(no_totp)), id="totp",
            detail=t("Unter Konto → Zweitfaktor einrichten; danach in Entdecken & Anfragen „Admins brauchen einen Zweitfaktor“ einschalten."))
    else:
        add("security", "ok", t("Alle Admins haben einen Zweitfaktor"), id="totp")
    if not app.settings().get("admin_totp_required"):
        add("security", "info", t("Zweitfaktor für Admins ist nicht Pflicht"), id="totp_required")
    log = os.path.join(cfg.config_dir, "logs", "auth.log")
    if os.path.isdir(os.path.dirname(log)) and os.access(os.path.dirname(log), os.W_OK):
        add("security", "ok", t("Anmelde-Protokoll für CrowdSec: {path}", path=log), id="authlog")
    else:
        add("security", "error", t("Anmelde-Protokoll nicht schreibbar: {path}", path=log), id="authlog")
    blocked = app.accounts.blocked_count()
    add("security", "warn" if blocked else "ok", (t("1 Adresse gerade wegen Fehlversuchen gesperrt") if blocked == 1
                                                 else t("{n} Adressen gerade wegen Fehlversuchen gesperrt", n=blocked)) if blocked
        else t("Keine Adresse wegen Fehlversuchen gesperrt"), id="blocked")

    # -- storage --
    checks += _dir_check(t("Eingang"), cfg.inbox)
    checks += _dir_check(t("Bibliothek"), cfg.library)
    checks += _dir_check(t("Konfiguration"), cfg.config_dir)

    # -- services (the page tests the connected ones itself, one by one) --
    src = app.usenet.public_config()
    abs_cfg, plex_cfg, ntfy = app.abs.public_config(), app.plex.public_config(), app.notifier.public_config()
    if not src["indexers"]:
        add("services", "info", t("Kein Indexer eingerichtet – Anfragen werden nicht automatisch besorgt"), id="indexers")
    if not src["sab"].get("apikey_set"):
        add("services", "info", t("SABnzbd nicht eingerichtet"), id="sab")
    if ntfy["configured"]:
        add("services", "error" if ntfy["last_error"] else "ok",
            t("ntfy: {error}", error=ntfy["last_error"]) if ntfy["last_error"] else t("ntfy eingerichtet ({topic})", topic=ntfy["topic"]),
            id="ntfy")
    else:
        add("services", "info", t("ntfy nicht eingerichtet – keine Benachrichtigungen aufs Handy"), id="ntfy")
    try:
        audible.AudibleClient().genres(app.settings().get("ab_region", "de"))
        add("services", "ok", t("Audible-Shop erreichbar"), id="audible")
    except Exception as e:  # noqa: BLE001 – shown, not raised
        add("services", "error", t("Audible-Shop nicht erreichbar: {error}", error=e), id="audible",
            detail=t("Ohne Audible keine Erkennung, Tags und Vorschläge – Internetzugang des Containers prüfen."))

    # -- system --
    sb = sandbox.status()
    add("system", "ok", t("Hörwerk {version}, läuft seit {since}", version=VERSION,
                          since=time.strftime("%d.%m. %H:%M", time.localtime(STARTED))), id="version")
    ff = _ffmpeg_version()
    add("system", "error" if ff == t("nicht gefunden") else "ok", t("ffmpeg {version}", version=ff), id="ffmpeg")
    add("system", "ok" if sb["active"] else "warn",
        t("ffmpeg abgeschottet (Landlock ABI {abi})", abi=sb["landlock_abi"]) if sb["active"]
        else t("ffmpeg ohne Sandbox"), id="sandbox",
        detail="" if sb["active"] else t("Kernel oder Docker erlauben kein Landlock – kein Fehler, aber weniger Schutz bei kaputten Dateien."))
    q = app.engine.stats()
    add("system", "info", t("Warteschlangen: Umwandeln {e} aktiv / {ew} wartend, Erkennen & Ablegen {i} / {iw}",
                            e=q["encode"]["running"], ew=q["encode"]["waiting"], i=q["io"]["running"], iw=q["io"]["waiting"]),
        id="queues")
    return {"checks": checks, "server_time": time.time(), "app_url": app.notifier.link_base(), "version": VERSION,
            "services": {"indexers": [{"id": i["id"], "name": i.get("name") or i.get("url", "")} for i in src["indexers"]
                                      if i.get("enabled", True) and i.get("apikey_set")],
                         "sab": bool(src["sab"].get("apikey_set") and src["sab"].get("url")),
                         "abs": bool(abs_cfg.get("url") and abs_cfg.get("token_set")),
                         "plex": bool(plex_cfg.get("url") and plex_cfg.get("token_set"))}}
