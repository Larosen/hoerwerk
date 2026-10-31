"""SQLite storage: one file in /config instead of many JSON files.

* WAL mode: readers never block the writer, a crash never leaves a half-written file.
* Schema versions in ``PRAGMA user_version``; ``MIGRATIONS`` only ever grows.
* On the first start the JSON files of older versions are imported and renamed
  to ``*.json.migrated`` (kept as a backup).
"""
from __future__ import annotations

import json
import os
import re
import sqlite3
import threading
import time
from contextlib import contextmanager
from typing import Any, Iterator, List, Optional

DB_NAME = "hoerwerk.db"
OLD_DB_NAMES = ("better-renamer.db",)       # the app was called "Better Renamer" up to version 4


def db_path(config_dir: str) -> str:
    """Path of the database; a database from before the rename is taken over (moved, with its WAL files)."""
    path = os.path.join(config_dir, DB_NAME)
    if not os.path.exists(path):
        for old in OLD_DB_NAMES:
            old_path = os.path.join(config_dir, old)
            if os.path.exists(old_path):
                for suffix in ("", "-wal", "-shm"):
                    if os.path.exists(old_path + suffix):
                        os.replace(old_path + suffix, path + suffix)
                break
    return path

MIGRATIONS: List[str] = [
    # 1: everything that used to be a JSON file
    """
    CREATE TABLE kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE rename_history (
        id     TEXT PRIMARY KEY,
        time   REAL NOT NULL,
        folder TEXT NOT NULL,
        data   TEXT NOT NULL
    );
    CREATE INDEX rename_history_time ON rename_history(time);
    CREATE TABLE presets (
        name  TEXT PRIMARY KEY,
        rules TEXT NOT NULL
    );
    CREATE TABLE jobs (
        id      TEXT PRIMARY KEY,
        kind    TEXT NOT NULL,
        status  TEXT NOT NULL,
        source  TEXT NOT NULL,
        created REAL NOT NULL,
        updated REAL NOT NULL,
        data    TEXT NOT NULL
    );
    CREATE INDEX jobs_status ON jobs(kind, status);
    CREATE INDEX jobs_source ON jobs(source);
    CREATE TABLE library (
        path            TEXT PRIMARY KEY,
        title           TEXT NOT NULL DEFAULT '',
        authors         TEXT NOT NULL DEFAULT '[]',
        asin            TEXT,
        series          TEXT,
        series_position TEXT,
        format          TEXT,
        files           INTEGER NOT NULL DEFAULT 0,
        mtime           REAL NOT NULL DEFAULT 0
    );
    CREATE INDEX library_asin ON library(asin);
    CREATE TABLE grabs (
        name TEXT PRIMARY KEY,
        key  TEXT NOT NULL,
        time REAL NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX grabs_key ON grabs(key);
    CREATE INDEX grabs_time ON grabs(time);
    """,
]


def dumps(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def loads(text: Optional[str], default=None):
    if text is None:
        return default
    try:
        return json.loads(text)
    except ValueError:
        return default


class Database:
    """One connection shared by all threads, serialised by a lock.

    The app does little I/O on the database (a few writes per second at most), so one
    connection is simpler and safer than a pool – and SQLite serialises writers anyway.
    """

    def __init__(self, path: str) -> None:
        self.path = path
        self.lock = threading.RLock()
        new = path != ":memory:" and not os.path.exists(path)
        if path != ":memory:":
            os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
        self.conn = sqlite3.connect(path, check_same_thread=False, isolation_level=None, timeout=30)
        self.conn.row_factory = sqlite3.Row
        if new:
            try:
                os.chmod(path, 0o600)      # holds API keys of indexers and SABnzbd
            except OSError:
                pass
        with self.lock:
            self.conn.execute("PRAGMA journal_mode=WAL")
            self.conn.execute("PRAGMA synchronous=NORMAL")
            self.conn.execute("PRAGMA foreign_keys=ON")
            self._migrate()

    def _migrate(self) -> None:
        version = self.conn.execute("PRAGMA user_version").fetchone()[0]
        if version > len(MIGRATIONS):
            raise RuntimeError(f"Database is newer (version {version}) than this program – please update / Datenbank ist neuer als dieses Programm")
        for n, script in enumerate(MIGRATIONS[version:], version + 1):
            with self.tx():
                for stmt in (s.strip() for s in script.split(";")):
                    if stmt:
                        self.conn.execute(stmt)
                self.conn.execute(f"PRAGMA user_version={n}")

    @contextmanager
    def tx(self) -> Iterator[sqlite3.Connection]:
        """A write transaction; nested calls join the outer one."""
        with self.lock:
            if self.conn.in_transaction:
                yield self.conn
                return
            self.conn.execute("BEGIN IMMEDIATE")
            try:
                yield self.conn
            except BaseException:
                self.conn.execute("ROLLBACK")
                raise
            self.conn.execute("COMMIT")

    def all(self, sql: str, args: tuple = ()) -> List[sqlite3.Row]:
        with self.lock:
            return self.conn.execute(sql, args).fetchall()

    def one(self, sql: str, args: tuple = ()) -> Optional[sqlite3.Row]:
        with self.lock:
            return self.conn.execute(sql, args).fetchone()

    def run(self, sql: str, args: tuple = ()) -> int:
        with self.tx() as c:
            return c.execute(sql, args).rowcount

    def close(self) -> None:
        with self.lock:
            self.conn.close()

    # -- key/value documents (settings, sources …) --------------------------
    def get(self, key: str, default=None):
        row = self.one("SELECT value FROM kv WHERE key=?", (key,))
        return loads(row["value"], default) if row else default

    def put(self, key: str, value) -> None:
        self.run("INSERT INTO kv(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                 (key, dumps(value)))


# --------------------------------------------------------------------------- #
# Import of the JSON files written by versions up to 3.x
# --------------------------------------------------------------------------- #

def _read_json(path: str):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return None


def import_legacy(db: Database, config_dir: str) -> List[str]:
    """Import old JSON files once; returns the names that were imported."""
    done: List[str] = []

    def take(name: str, fn) -> None:
        path = os.path.join(config_dir, name)
        data = _read_json(path)
        if data is None:
            return
        with db.tx():
            fn(data)
        os.replace(path, path + ".migrated")
        done.append(name)

    def settings(data):
        if isinstance(data, dict) and db.get("settings") is None:
            db.put("settings", data)

    def history(data):
        for h in data if isinstance(data, list) else []:
            if isinstance(h, dict) and h.get("folder") is not None:
                db.conn.execute("INSERT OR IGNORE INTO rename_history(id, time, folder, data) VALUES(?,?,?,?)",
                                (h.get("id") or os.urandom(4).hex(), h.get("time") or time.time(),
                                 h["folder"], dumps(h)))

    def presets(data):
        for name, rules in (data.items() if isinstance(data, dict) else []):
            if isinstance(rules, list):
                db.conn.execute("INSERT OR IGNORE INTO presets(name, rules) VALUES(?,?)", (name, dumps(rules)))

    def audiobooks(data):
        for j in data if isinstance(data, list) else []:
            if isinstance(j, dict) and j.get("id") and j.get("source"):
                db.conn.execute(
                    "INSERT OR IGNORE INTO jobs(id, kind, status, source, created, updated, data) VALUES(?,?,?,?,?,?,?)",
                    (j["id"], "audiobook", j.get("status") or "queued", j["source"], j.get("created") or time.time(),
                     j.get("updated") or time.time(), dumps(j)))

    def library(data):
        entries = (data or {}).get("entries") if isinstance(data, dict) else None
        for e in (entries or {}).values():
            if isinstance(e, dict) and e.get("path"):
                save_library_entry(db.conn, e)
        if isinstance(data, dict) and data.get("last_scan"):
            db.put("library_last_scan", data["last_scan"])

    def sources(data):
        if isinstance(data, dict) and db.get("sources") is None:
            db.put("sources", data)

    def grabs(data):
        for name, info in (data.items() if isinstance(data, dict) else []):
            if isinstance(info, dict):
                save_grab(db.conn, name, info)

    for name, fn in (("settings.json", settings), ("history.json", history), ("presets.json", presets),
                     ("audiobooks.json", audiobooks), ("library.json", library), ("sources.json", sources),
                     ("grabs.json", grabs)):
        take(name, fn)
    return done


# --------------------------------------------------------------------------- #
# Row helpers used by several modules
# --------------------------------------------------------------------------- #

LIBRARY_COLS = ("path", "title", "authors", "asin", "series", "series_position", "format", "files", "mtime")


def save_library_entry(conn: sqlite3.Connection, e: dict) -> None:
    conn.execute(
        "INSERT OR REPLACE INTO library(path, title, authors, asin, series, series_position, format, files, mtime) "
        "VALUES(?,?,?,?,?,?,?,?,?)",
        (e["path"], e.get("title") or "", dumps(e.get("authors") or []), e.get("asin"), e.get("series"),
         None if e.get("series_position") is None else str(e["series_position"]), e.get("format"),
         int(e.get("files") or 0), float(e.get("mtime") or 0)))


def library_entry(row: sqlite3.Row) -> dict:
    e = {k: row[k] for k in LIBRARY_COLS}
    e["authors"] = loads(e["authors"], [])
    return e


def grab_key(name: str) -> str:
    return re.sub(r"[\W_]+", "", name.lower())


def save_grab(conn: sqlite3.Connection, name: str, info: dict) -> None:
    conn.execute("INSERT OR REPLACE INTO grabs(name, key, time, data) VALUES(?,?,?,?)",
                 (name, grab_key(name), float(info.get("time") or time.time()), dumps(info)))
