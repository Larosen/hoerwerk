"""Gesucht: matching releases, periodic search, announcing, auto download, leaving the list."""
import os
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import db as dbm  # noqa: E402
import wanted  # noqa: E402
from config import default_settings  # noqa: E402


class FakeUsenet:
    def __init__(self):
        self.results = []
        self.searches = []
        self.sent = []
        self.indexers = 1

    def configured(self):
        return {"indexers": self.indexers, "sab": True}

    def search(self, query, language="", extra=None):
        self.searches.append(query)
        return {"results": [{**r, "id": f"id{n}"} for n, r in enumerate(self.results)], "errors": []}

    def send(self, rid, hints=None):
        self.sent.append((rid, hints))
        return {"title": "sent"}


class FakeIndex:
    def __init__(self):
        self.lock = threading.Lock()
        self.entries = {}
        self.owned = set()

    def find(self, b):
        return [{"path": "x"}] if b.get("title") in self.owned else []


class FakeNotifier:
    def __init__(self):
        self.sent = []

    def send(self, event, title, message, priority=None):
        self.sent.append((event, title, message))


BOOK = {"asin": "b0wanted1", "title": "Das stille Haus", "authors": ["Anna Autorin"], "cover_url": None}


class WantedTest(unittest.TestCase):
    def setUp(self):
        self.db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        self.addCleanup(self.db.close)
        self.settings = default_settings({})
        self.usenet, self.index, self.notifier = FakeUsenet(), FakeIndex(), FakeNotifier()
        self.busy = set()
        self.w = wanted.Wanted(self.db, self.usenet, self.index, self.notifier, lambda: self.settings,
                               pending=lambda: self.busy)

    def test_fits(self):
        self.assertTrue(wanted.fits("Anna.Autorin-Das.stille.Haus.German.2026.M4B", BOOK))
        self.assertTrue(wanted.fits("Autorin, Anna - Das Stille Haus (Ungekürzt)", BOOK))
        self.assertFalse(wanted.fits("Anna.Autorin-Das.laute.Meer.German.M4B", BOOK))      # other book
        self.assertFalse(wanted.fits("Bert.Berger-Das.stille.Haus.German.M4B", BOOK))      # other author
        self.assertTrue(wanted.fits("Mueller - Fraeulein Else", {"title": "Fräulein Else", "authors": ["Eva Müller"]}))

    def test_add_search_announce_once(self):
        item = self.w.add(BOOK)
        self.assertEqual(item["id"], "B0WANTED1")
        self.w.search(item["id"])
        self.assertEqual(self.w.items()[0]["status"], "missing")
        self.assertEqual(self.notifier.sent, [])
        self.usenet.results = [{"title": "Anna.Autorin-Das.stille.Haus.German.M4B", "size": 1, "indexer": "X"},
                               {"title": "Ganz.anderes.Buch", "size": 1, "indexer": "X"}]
        self.w.search(item["id"])
        got = self.w.items()[0]
        self.assertEqual((got["status"], len(got["found"])), ("found", 1))
        self.assertEqual([e for e, *_ in self.notifier.sent], ["wanted_found"])
        self.w.search(item["id"])                                    # same release again: no second message
        self.assertEqual(len(self.notifier.sent), 1)
        self.assertEqual(self.usenet.sent, [])                       # only announced by default

    def test_auto_download_prefers_unabridged(self):
        self.settings["wanted_auto"] = True
        item = self.w.add(BOOK)
        self.usenet.results = [{"title": "Autorin - Das stille Haus (gekürzt)", "size": 1, "indexer": "X", "abridged": True},
                               {"title": "Autorin - Das stille Haus", "size": 2, "indexer": "X"}]
        self.w.search(item["id"])
        self.assertEqual(self.usenet.sent[0][0], "id1")
        self.assertEqual(self.usenet.sent[0][1]["asin"], "B0WANTED1")
        self.w.search(item["id"])
        self.usenet.results.append({"title": "Autorin - Das stille Haus MP3", "size": 3, "indexer": "Y"})
        self.w.search(item["id"])
        self.assertEqual(len(self.usenet.sent), 1)                   # never twice

    def test_run_due_respects_interval(self):
        a = self.w.add(BOOK)
        self.w.add({"title": "Zweites Buch", "authors": ["Bert Berger"]})
        self.assertEqual(self.w.run_due(), 2)
        self.assertEqual(self.w.run_due(), 0)                        # 12 h not up yet
        data = self.db.get("wanted")
        data[a["id"]]["last_search"] = time.time() - 13 * 3600
        self.db.put("wanted", data)
        self.assertEqual(self.w.run_due(), 1)

    def test_leaves_list_when_in_library_and_shows_download(self):
        self.w.add(BOOK)
        self.busy = {"B0WANTED1"}
        self.assertEqual(self.w.items()[0]["status"], "loading")
        self.index.owned = {"Das stille Haus"}
        self.assertEqual(self.w.items(), [])

    def test_no_indexer_and_bad_input(self):
        self.usenet.indexers = 0
        item = self.w.add(BOOK)
        self.assertTrue(self.w.search(item["id"])["error"])
        self.assertEqual(self.usenet.searches, [])
        with self.assertRaises(ValueError):
            self.w.add({"title": "Ohne Autor"})
        self.assertTrue(self.w.remove(item["id"]))
        self.assertFalse(self.w.remove(item["id"]))


if __name__ == "__main__":
    unittest.main()
