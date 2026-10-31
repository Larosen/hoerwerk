"""Wires the services together. Nothing runs at import time: ``App(config).start()``."""
from __future__ import annotations

import os
import threading
import time
from typing import Optional

import db as dbm
from abs import Audiobookshelf
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
from stores import Settings
from usenet import Usenet
import i18n
import applog
from i18n import t


class App:
    def __init__(self, config: Config, start_workers: bool = True) -> None:
        self.config = config
        self.start_workers = start_workers
        os.makedirs(config.config_dir, exist_ok=True)
        self.db = dbm.Database(dbm.db_path(config.config_dir))
        imported = dbm.import_legacy(self.db, config.config_dir)
        if imported:
            applog.info(f"Übernommen in die Datenbank: {', '.join(imported)}")
        self.defaults = default_settings(config.env or os.environ)
        self._settings = Settings(self.db, self.defaults)
        i18n.set_language(self.settings().get("language", "de"))
        self.events = EventBus()
        self.notifier = Notifier(self.db)
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
        self.events.provide("audiobooks", lambda: self.pipeline.status() | {"queues": self.engine.stats()})
        self.events.provide("library", self.library_index.stats)
        self.downloads = DownloadWatcher(self.usenet, self.notifier, self.db)
        self._started = False

    def _pending_asins(self) -> set:
        """Books on their way: grabbed in the last 30 days or being processed (Entdecken hides them)."""
        out = {(j.match or {}).get("asin") or (j.hints or {}).get("asin")
               for j in list(self.pipeline.jobs) if j.status not in ("failed", "cancelled")}
        for row in self.db.all("SELECT data FROM grabs WHERE time > ?", (time.time() - 30 * 86400,)):
            out.add((dbm.loads(row["data"]) or {}).get("asin"))
        return {a for a in out if a}

    # -- settings ----------------------------------------------------------
    def settings(self) -> dict:
        return self._settings.get()

    def update_settings(self, changes: dict) -> dict:
        out = self._settings.update(changes)
        i18n.set_language(out.get("language", "de"))
        return out

    # -- notifications -----------------------------------------------------
    def _book_event(self, job) -> None:
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
        self.engine.start()
        self.pipeline.start()
        threading.Thread(target=self._library_rescanner, daemon=True, name="library-index").start()
        threading.Thread(target=self.downloads.run, daemon=True, name="download-watch").start()
        return self

    def _library_rescanner(self) -> None:
        """Read the library at start and then every few hours (new books are added right away anyway)."""
        while True:
            try:
                self.library_index.scan()
            except Exception as e:  # noqa: BLE001 – keep the thread alive
                applog.error(t("Bibliothek konnte nicht eingelesen werden: {error}", error=e), "Bibliothek")
            time.sleep(max(1, int(self.settings().get("library_rescan_hours") or 6)) * 3600)


def create_app(config: Optional[Config] = None, start: bool = True) -> App:
    app = App(config or Config.from_env())
    return app.start() if start else app
