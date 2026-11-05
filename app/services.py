"""Wires the services together. Nothing runs at import time: ``App(config).start()``."""
from __future__ import annotations

import os
import threading
import time
from typing import Optional

import db as dbm
import authlog
import secrets
from abs import Audiobookshelf
from accounts import Accounts
from invites import Invites
import tagger
from config import Config, default_settings
from discover import Discover
from engine import Engine
from events import EventBus
from library_index import LibraryIndex
from meta import cache as meta_cache
from notify import DownloadWatcher, Notifier
from plex import Plex
from pipeline import Pipeline
from providers import Providers
from listening import Listening
from stats import Stats
from stores import Settings
from uploads import Uploads
from usenet import Usenet
from bookrequests import Requests
import i18n
import applog
from i18n import t


class App:
    def __init__(self, config: Config, start_workers: bool = True) -> None:
        self.config = config
        self.start_workers = start_workers
        os.makedirs(config.config_dir, exist_ok=True)
        self.db = dbm.Database(dbm.db_path(config.config_dir))
        self._db_maintained = 0.0
        self.db_backup_error = ""                  # last snapshot failed: why (health check)
        self.db_backup_at: Optional[float] = None
        self.library_ready = threading.Event()     # set after the first library scan: processing may start
        self.library_error = ""
        imported = dbm.import_legacy(self.db, config.config_dir)
        if imported:
            applog.info(f"Übernommen in die Datenbank: {', '.join(imported)}")
        authlog.setup(config.config_dir)
        self.accounts = Accounts(self.db)
        self.invites = Invites(self.db, self.accounts)
        self.setup_code = ""
        self._first_account()
        self.defaults = default_settings(config.env or os.environ)
        self._settings = Settings(self.db, self.defaults)
        i18n.set_language(self.settings().get("language", "de"))
        self.events = EventBus()
        self.notifier = Notifier(self.db)
        self.notifier.public_url = lambda: self.settings().get("public_url", "")
        self.engine = Engine({"encode": config.encode_workers, "io": config.io_workers},
                             on_change=self.events.notifier("audiobooks"))
        self.usenet = Usenet(self.db)
        self.abs = Audiobookshelf(self.db, on_error=lambda msg: self.notifier.send(
            "abs_error", t("Audiobookshelf-Scan fehlgeschlagen"), msg))
        self.plex = Plex(self.db, on_error=lambda msg: self.notifier.send(
            "plex_error", t("Plex-Scan fehlgeschlagen"), msg))
        self.library_index = LibraryIndex(config.library, self.db, read_m4b=tagger.read_book, probe=meta_cache.probe,
                                          notify=self.events.notifier("library"))
        self.pipeline = Pipeline(self.settings, self.db,
                                 config.inbox, config.library, config.work_dir, jobs=config.cpu_jobs,
                                 nice=config.nice, hint_provider=self.usenet.hint_for,
                                 index=self.library_index, engine=self.engine,
                                 notify=self.events.notifier("audiobooks"),
                                 on_filed=lambda dest: self._library_changed(), on_event=self._book_event)
        self.pipeline.providers = Providers(self.pipeline.client,
                                            enabled=lambda: bool(self.settings().get("ab_fallback", True)))
        self.discover = Discover(self.db, self.pipeline.client, self.library_index, self.settings,
                                 pending=self._pending_asins)
        self.requests = Requests(self.db, self.accounts, self.usenet, self.library_index, self.notifier, self.settings,
                                 jobs=lambda: list(self.pipeline.jobs))
        self.listening = Listening(self.accounts, self.abs, self.plex, self.library_index)
        self.discover.seeds = self.listening.seeds
        self.stats = Stats(self.db, self.accounts, self.library_index, listening=self.listening)
        self.pipeline.open_requests = self.requests.open_for_matching
        self.pipeline.on_request = self.requests.on_job
        self.pipeline.on_wrong_download = self.requests.wrong_download
        self.uploads = Uploads(config.inbox, on_done=self.pipeline.arrived)
        self.events.provide("audiobooks", lambda: self.pipeline.status() | {"queues": self.engine.stats()})
        self.events.provide("library", self.library_index.stats)
        self.downloads = DownloadWatcher(self.usenet, self.notifier, self.db,
                                         on_done=lambda name: self.pipeline.scan_now())
        self._started = False

    def _pending_asins(self) -> set:
        """Books on their way: grabbed in the last 30 days or being processed (Entdecken hides them)."""
        out = {(j.match or {}).get("asin") or (j.hints or {}).get("asin")
               for j in list(self.pipeline.jobs) if j.status not in ("failed", "cancelled")}
        for row in self.db.all("SELECT data FROM grabs WHERE time > ?", (time.time() - 30 * 86400,)):
            out.add((dbm.loads(row["data"]) or {}).get("asin"))
        return {a for a in out if a}

    def audit(self, req, action: str, detail: str = "") -> None:
        """Admin actions for the audit log (who changed what, from where)."""
        user = (req.user or {}).get("name", "–") if req is not None else "–"
        ip = getattr(req, "ip", "") or ""
        self.db.run("INSERT INTO audit(time, user, ip, action, detail) VALUES(?,?,?,?,?)",
                    (time.time(), user, ip, action[:80], str(detail)[:300]))
        self.db.run("DELETE FROM audit WHERE id <= (SELECT MAX(id) - 5000 FROM audit)")

    def _first_account(self) -> None:
        """AUTH_USER/AUTH_PASS of older versions become the admin; without any account a setup code is shown."""
        if self.accounts.count():
            if self.config.auth:
                applog.info(t("AUTH_USER/AUTH_PASS werden nicht mehr gebraucht – Konten verwaltest du in der App."))
            return
        if self.config.auth:
            from accounts import hash_password
            now = time.time()
            self.db.run("INSERT INTO users(name, pw, role, perms, created, pw_changed) VALUES(?,?,?,?,?,?)",
                        (self.config.auth_user.strip()[:40], hash_password(self.config.auth_pass), "admin", "{}", now, now))
            authlog.event("setup", user=self.config.auth_user, ip="env")
            applog.info(t("Admin-Konto „{name}“ aus AUTH_USER/AUTH_PASS angelegt.", name=self.config.auth_user))
            return
        self.setup_code = "-".join(secrets.token_hex(2).upper() for _ in range(3))
        applog.warn(t("Noch kein Konto: Admin im Browser anlegen. Von außerhalb des Heimnetzes braucht es den Einrichtungs-Code {code}.",  # noqa: E501
                      code=self.setup_code), "Anmeldung")

    # -- settings ----------------------------------------------------------
    def settings(self) -> dict:
        return self._settings.get()

    def update_settings(self, changes: dict) -> dict:
        out = self._settings.update(changes)
        i18n.set_language(out.get("language", "de"))
        return out

    # -- notifications -----------------------------------------------------
    def _book_event(self, job) -> None:
        try:
            self.requests.on_job(job)                   # a request waiting for this book hears about it
        except Exception as e:  # noqa: BLE001 – a request must never stop the pipeline
            applog.warn(t("Anfragen: {error}", error=e), "Anfragen")
        n = self.notifier
        m = job.match or {}
        name = m.get("title") or job.name
        who = ", ".join(m.get("authors") or [])
        if job.status == "done":
            where = os.path.relpath(job.dest, self.config.library) if job.dest else ""
            n.send("filed", t("Hörbuch abgelegt"), f"{name}" + (f" – {who}" if who else "") + (f"\n{where}" if where else ""))
        elif job.status == "review" and job.duplicate:
            n.send("duplicate", t("Schon in der Bibliothek"), f"{job.name}\n→ {job.duplicate.get('path')}")
        elif job.status == "review":
            n.send("review", t("Hörbuch braucht Prüfung"), f"{job.name}\n{job.message}")
        elif job.status == "failed":
            n.send("failed", t("Fehler bei einem Hörbuch"), f"{job.name}\n{job.error or ''}".strip(), priority=4)

    def _library_changed(self) -> None:
        """Books were filed or replaced: media servers should look again (each one once things calm down)."""
        self.abs.request_scan()
        self.plex.request_scan()

    # -- background work ---------------------------------------------------
    def start(self) -> "App":
        if self._started or not self.start_workers:
            return self
        self._started = True
        # Books are only processed once the library is known: duplicates are checked against it. The rescanner
        # reads it first (from the database cache at once, from disk now) and then lets the pipeline start.
        self.pipeline.scan_note = t("Wartet, bis die Bibliothek eingelesen ist …")
        threading.Thread(target=self._library_rescanner, daemon=True, name="library-index").start()
        threading.Thread(target=self.downloads.run, daemon=True, name="download-watch").start()
        threading.Thread(target=self._request_worker, daemon=True, name="requests").start()
        return self

    def _library_rescanner(self, sleep=time.sleep) -> None:
        """Read the library at start and then every few hours (new books are added right away anyway)."""
        self.wait_for_library(sleep)
        while True:
            sleep(max(1, int(self.settings().get("library_rescan_hours") or 6)) * 3600)
            try:
                self.library_index.scan()
            except Exception as e:  # noqa: BLE001 – keep the thread alive
                applog.error(t("Bibliothek konnte nicht eingelesen werden: {error}", error=e), "Bibliothek")

    def wait_for_library(self, sleep=time.sleep) -> None:
        """The first scan, then processing starts. If the library cannot be read, nothing is processed (no
        duplicate check against a half-known library) – the health check says why, and it is tried every minute."""
        while not self.library_ready.is_set():
            try:
                self.library_index.scan()
            except Exception as e:  # noqa: BLE001
                self.library_error = str(e) or e.__class__.__name__
                applog.error(t("Bibliothek konnte nicht eingelesen werden – Verarbeitung wartet: {error}",
                               error=self.library_error), "Bibliothek")
                sleep(60)
                continue
            self.library_error = ""
            self.library_ready.set()
            self.pipeline.scan_note = ""
            self.engine.start()
            self.pipeline.start()


    def _request_worker(self) -> None:
        """Requests: search again when due, follow downloads and imports (every two minutes)."""
        time.sleep(60)                                      # let the library index load first
        try:
            n = self.requests.import_wanted()
            if n:
                applog.info(t("1 Buch aus „Gesucht“ als Anfrage übernommen") if n == 1
                            else t("{n} Bücher aus „Gesucht“ als Anfragen übernommen", n=n), "Anfragen")
        except Exception as e:  # noqa: BLE001
            applog.error(t("Anfragen: {error}", error=e), "Anfragen")
        while True:
            try:
                self.requests.tick()
            except Exception as e:  # noqa: BLE001 – keep the thread alive
                applog.error(t("Anfragen: {error}", error=e), "Anfragen")
            try:
                self.discover.tick()            # review list daily, late audiobooks monthly – also unseen
            except Exception as e:  # noqa: BLE001
                applog.warn(t("Entdecken: {error}", error=e), "Entdecken")
            self.maintain_db()
            time.sleep(120)

    def maintain_db(self, now: Optional[float] = None) -> Optional[str]:
        """Once a day: a consistent snapshot of the database (backups/hoerwerk-<weekday>.db, the last seven days)
        and the WAL moved into the main file. A copy of /config taken while Hörwerk runs (Unraid appdata
        backup) may catch the database between two writes – the snapshot never does."""
        now = time.time() if now is None else now
        # after a success the next one in a day; after a failure (disk full for a moment) again in an hour
        wait = 3600 if self.db_backup_error else 23 * 3600
        if now - self._db_maintained < wait:
            return None
        self._db_maintained = now
        target = os.path.join(self.config.config_dir, "backups", time.strftime("hoerwerk-%a.db", time.localtime(now)))
        try:
            os.makedirs(os.path.dirname(target), mode=0o700, exist_ok=True)
            self.db.backup(target)
            self.db.checkpoint()
        except Exception as e:  # noqa: BLE001 – a failed snapshot must not stop the worker
            self.db_backup_error = str(e) or e.__class__.__name__
            applog.warn(t("Datenbank-Sicherung fehlgeschlagen, neuer Versuch in einer Stunde: {error}",
                          error=self.db_backup_error), "Datenbank")
            return None
        self.db_backup_error = ""
        self.db_backup_at = now
        return target


def create_app(config: Optional[Config] = None, start: bool = True) -> App:
    app = App(config or Config.from_env())
    return app.start() if start else app
