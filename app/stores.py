"""Settings in the database."""
from __future__ import annotations


import db as dbm
import library


class Settings:
    def __init__(self, db: dbm.Database, defaults: dict) -> None:
        self.db = db
        self.defaults = defaults

    def get(self) -> dict:
        saved = self.db.get("settings") or {}
        if saved.get("ab_folder_template") == library.OLD_DEFAULT_FOLDER:
            saved.pop("ab_folder_template")   # saved with the v3.0 default: follow the new default
        return {**self.defaults, **{k: v for k, v in saved.items() if k in self.defaults}}

    def update(self, changes: dict) -> dict:
        with self.db.tx():
            self.db.put("settings", {**(self.db.get("settings") or {}), **changes})
        return self.get()

