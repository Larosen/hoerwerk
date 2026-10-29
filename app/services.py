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
from converter import Converter
from engine import Engine
from events import EventBus
from library_index import LibraryIndex
from meta import cache as meta_cache
from pipeline import Pipeline
from providers import Providers
from stores import History, Presets, Settings
from usenet import Usenet


class App:
    def __init__(self, config: Config, start_workers: bool = True) -> None:
        self.config = config
        self.start_workers = start_workers
        os.makedirs(config.config_dir, exist_ok=True)
        self.db = dbm.Database(os.path.join(config.config_dir, dbm.DB_NAME))
        imported = dbm.import_legacy(self.db, config.config_dir)
        if imported:
            print(f"Übernommen in die Datenbank: {', '.join(imported)}", flush=True)
        self.defaults = default_settings(config.env or os.environ)
        self._settings = Settings(self.db, self.defaults)
        self.history = History(self.db)
        self.presets = Presets(self.db)
        self.events = EventBus()
        self.engine = Engine({"encode": config.encode_workers, "io": config.io_workers},
                             on_change=self.events.notifier("audiobooks"))
        self.converter = Converter(self.settings, self.db, self.engine, jobs=config.cpu_jobs, nice=config.nice,
                                   notify=self.events.notifier("jobs"))
        self.usenet = Usenet(self.db)
        self.abs = Audiobookshelf(self.db)
        self.library_index = LibraryIndex(config.library, self.db, read_m4b=tagger.read_book, probe=meta_cache.probe,
                                          notify=self.events.notifier("library"))
        self.pipeline = Pipeline(self.settings, self.db,
                                 config.inbox, config.library, config.work_dir, jobs=config.cpu_jobs,
                                 nice=config.nice, hint_provider=self.usenet.hint_for,
                                 index=self.library_index, engine=self.engine,
                                 notify=self.events.notifier("audiobooks"),
                                 on_filed=lambda dest: self.abs.request_scan())
        self.pipeline.providers = Providers(self.pipeline.client,
                                            enabled=lambda: bool(self.settings().get("ab_fallback", True)))
        self.events.provide("audiobooks", lambda: self.pipeline.status() | {"queues": self.engine.stats()})
        self.events.provide("jobs", self.converter.list)
        self.events.provide("library", self.library_index.stats)
        self._started = False

    # -- settings ----------------------------------------------------------
    def settings(self) -> dict:
        return self._settings.get()

    def update_settings(self, changes: dict) -> dict:
        return self._settings.update(changes)

    # -- background work ---------------------------------------------------
    def start(self) -> "App":
        if self._started or not self.start_workers:
            return self
        self._started = True
        self.engine.start()
        self.converter.start()
        self.pipeline.start()
        threading.Thread(target=self._library_rescanner, daemon=True, name="library-index").start()
        return self

    def _library_rescanner(self) -> None:
        """Read the library at start and then every few hours (new books are added right away anyway)."""
        while True:
            try:
                self.library_index.scan()
            except Exception as e:  # noqa: BLE001 – keep the thread alive
                print(f"Bibliothek konnte nicht eingelesen werden: {e}", flush=True)
            time.sleep(max(1, int(self.settings().get("library_rescan_hours") or 6)) * 3600)


def create_app(config: Optional[Config] = None, start: bool = True) -> App:
    app = App(config or Config.from_env())
    return app.start() if start else app
