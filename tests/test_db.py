"""SQLite storage: schema, import of the JSON files of older versions, stores."""
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import config  # noqa: E402
import db as dbm  # noqa: E402
import services  # noqa: E402
import stores  # noqa: E402


class DatabaseTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="db-")
        self.path = os.path.join(self.dir, dbm.DB_NAME)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_schema_and_permissions(self):
        d = dbm.Database(self.path)
        self.assertEqual(d.one("PRAGMA user_version")[0], len(dbm.MIGRATIONS))
        self.assertEqual(d.one("PRAGMA journal_mode")[0], "wal")
        self.assertEqual(oct(os.stat(self.path).st_mode & 0o777), "0o600")
        d.close()
        # opening again does not migrate twice
        d = dbm.Database(self.path)
        self.assertEqual(d.one("PRAGMA user_version")[0], len(dbm.MIGRATIONS))

    def test_newer_database_is_refused(self):
        d = dbm.Database(self.path)
        d.conn.execute(f"PRAGMA user_version={len(dbm.MIGRATIONS) + 1}")
        d.close()
        with self.assertRaises(RuntimeError):
            dbm.Database(self.path)

    def test_transaction_rolls_back(self):
        d = dbm.Database(self.path)
        with self.assertRaises(ValueError):
            with d.tx() as c:
                c.execute("INSERT INTO kv(key, value) VALUES('a', '1')")
                raise ValueError
        self.assertIsNone(d.get("a"))

    def test_concurrent_writers(self):
        d = dbm.Database(self.path)
        s = stores.Settings(d, {"n": 0})
        errors = []

        def work(i):
            try:
                for _ in range(20):
                    s.update({f"k{i}": i})
                    d.put(f"x{i}", i)
            except Exception as e:  # noqa: BLE001
                errors.append(e)
        threads = [threading.Thread(target=work, args=(i,)) for i in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(errors, [])
        self.assertEqual(d.get("x7"), 7)

    def test_import_of_old_json_files(self):
        files = {
            "settings.json": {"ab_region": "us", "bitrate": "96k"},
            "history.json": [{"id": "h1", "time": 1.0, "folder": "a", "phases": [[["x", "y"]]], "created": []},
                             {"id": "h2", "time": 2.0, "folder": "b", "phases": [[["p", "q"]]], "created": []}],
            "presets.json": {"Hörbuch": [{"type": "case"}]},
            "audiobooks.json": [{"id": "j1", "source": "/inbox/x", "status": "review", "created": 5.0,
                                 "log": ["a"], "rel": "x", "name": "x"}],
            "library.json": {"entries": {"A/B": {"path": "A/B", "title": "B", "authors": ["A"], "asin": "B000000001",
                                                 "format": "m4b", "files": 1, "mtime": 3.0, "series_position": 2}},
                             "last_scan": 99.0},
            "sources.json": {"indexers": [{"id": "i", "name": "I", "url": "https://i.example", "apikey": "SECRET"}],
                             "sab": {"url": "http://sab", "apikey": "S2"}},
            "grabs.json": {"Autor - Titel": {"query": "Titel Autor", "time": 7.0}},
        }
        for name, data in files.items():
            with open(os.path.join(self.dir, name), "w", encoding="utf-8") as fh:
                json.dump(data, fh)
        app = services.App(config.Config.from_env({
            "CONFIG_DIR": self.dir, "DATA_DIR": self.dir, "INBOX_DIR": os.path.join(self.dir, "in"),
            "LIBRARY_DIR": os.path.join(self.dir, "lib")}), start_workers=False)
        for name in files:
            self.assertFalse(os.path.exists(os.path.join(self.dir, name)), name)
            self.assertTrue(os.path.exists(os.path.join(self.dir, name + ".migrated")), name)
        self.assertEqual(app.settings()["ab_region"], "us")
        self.assertEqual([h["id"] for h in app.history.list()], ["h2", "h1"])
        self.assertEqual(app.presets.get(), {"Hörbuch": [{"type": "case"}]})
        self.assertEqual(app.pipeline.get("j1").status, "review")
        self.assertEqual(app.library_index.find({"asin": "b000000001"})[0]["path"], "A/B")
        self.assertEqual(app.library_index.last_scan, 99.0)
        self.assertEqual(app.usenet.public_config()["configured"]["sab"], True)
        self.assertNotIn("SECRET", json.dumps(app.usenet.public_config()))
        self.assertEqual(app.usenet.hint_for("Autor.-.Titel")["query"], "Titel Autor")
        app.db.close()
        # a second start imports nothing and keeps everything
        again = services.App(app.config, start_workers=False)
        self.assertEqual(again.settings()["bitrate"], "96k")
        self.assertEqual(len(again.pipeline.jobs), 1)

    def test_history_limit_and_undo_order(self):
        d = dbm.Database(self.path)
        h = stores.History(d)
        for i in range(stores.History.LIMIT + 5):
            h.add(f"f{i}", [[["a", "b"]]], [])
        self.assertEqual(len(h.list()), stores.History.LIMIT)
        last = h.last()
        self.assertEqual(last["folder"], f"f{stores.History.LIMIT + 4}")
        h.remove(last["id"])
        self.assertEqual(h.last()["folder"], f"f{stores.History.LIMIT + 3}")

    def test_unknown_settings_are_ignored(self):
        d = dbm.Database(self.path)
        s = stores.Settings(d, {"a": 1})
        s.update({"a": 2, "evil": "x"})
        self.assertEqual(s.get(), {"a": 2})


if __name__ == "__main__":
    unittest.main()
