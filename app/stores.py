"""Settings, rename history and presets in the database."""
from __future__ import annotations

import threading
import time
import uuid
from typing import List, Optional

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


class History:
    """Renames that can be undone (newest last)."""
    LIMIT = 50

    def __init__(self, db: dbm.Database) -> None:
        self.db = db
        self.lock = threading.Lock()    # held while an undo touches the files

    def add(self, folder: str, phases: list, created: list) -> dict:
        entry = {"id": uuid.uuid4().hex[:8], "time": time.time(), "folder": folder,
                 "phases": phases, "created": created, "count": sum(len(p) for p in phases),
                 "sample": [[o, n] for o, n in phases[0][:3]] if phases else []}
        with self.db.tx() as c:
            c.execute("INSERT INTO rename_history(id, time, folder, data) VALUES(?,?,?,?)",
                      (entry["id"], entry["time"], folder, dbm.dumps(entry)))
            c.execute("DELETE FROM rename_history WHERE id NOT IN "
                      "(SELECT id FROM rename_history ORDER BY time DESC LIMIT ?)", (self.LIMIT,))
        return entry

    def list(self) -> List[dict]:
        return [dbm.loads(r["data"], {}) for r in
                self.db.all("SELECT data FROM rename_history ORDER BY time DESC LIMIT ?", (self.LIMIT,))]

    def last(self) -> Optional[dict]:
        row = self.db.one("SELECT data FROM rename_history ORDER BY time DESC LIMIT 1")
        return dbm.loads(row["data"]) if row else None

    def remove(self, entry_id: str) -> None:
        self.db.run("DELETE FROM rename_history WHERE id=?", (entry_id,))


class Presets:
    def __init__(self, db: dbm.Database) -> None:
        self.db = db

    def get(self) -> dict:
        return {r["name"]: dbm.loads(r["rules"], []) for r in self.db.all("SELECT name, rules FROM presets ORDER BY name")}

    def replace(self, data: dict) -> dict:
        with self.db.tx() as c:
            c.execute("DELETE FROM presets")
            for name, rules in data.items():
                c.execute("INSERT INTO presets(name, rules) VALUES(?,?)", (str(name), dbm.dumps(rules)))
        return self.get()
