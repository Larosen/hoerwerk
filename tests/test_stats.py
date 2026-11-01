"""Statistics per account: books, hours, space, waiting time; a series counts once; only library folders are measured."""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import accounts as acc  # noqa: E402
import bookrequests as br  # noqa: E402
import config  # noqa: E402
import db as dbm  # noqa: E402
import stats as st  # noqa: E402
from test_requests import FakeIndex, FakeNotifier, FakeUsenet, PW, volume  # noqa: E402


class Clock:
    def __init__(self):
        self.now = 1_760_000_000.0

    def __call__(self):
        return self.now


class StatsTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.mkdtemp()
        self.db = dbm.Database(os.path.join(tmp, "t.db"))
        self.addCleanup(self.db.close)
        self.clock = Clock()
        self.accounts = acc.Accounts(self.db, clock=self.clock)
        self.admin = self.accounts.create("Admin", PW, "admin")
        self.eva = self.accounts.create("Eva", PW)
        self.accounts.create("Ole", PW)
        self.index = FakeIndex()
        self.index.root = os.path.realpath(os.path.join(tmp, "lib"))
        os.makedirs(self.index.root)
        self.r = br.Requests(self.db, self.accounts, FakeUsenet(), self.index, FakeNotifier(),
                             lambda: config.default_settings({}), clock=self.clock)
        self.r.search_soon = lambda rid: None
        self.r.search_group_soon = lambda ids: None
        self.stats = st.Stats(self.db, self.accounts, self.index, clock=self.clock)

    def book_folder(self, name, size):
        folder = os.path.join(self.index.root, name)
        os.makedirs(folder)
        with open(os.path.join(folder, "book.m4b"), "wb") as f:
            f.write(b"\0" * size)
        return folder

    def test_per_account(self):
        old = self.r.create(self.eva, {**volume(9, "Alt"), "runtime_min": 60})
        self.clock.now += 200 * 86400                                    # half a year later
        a = self.r.create(self.eva, {**volume(1, "Eins"), "runtime_min": 600, "language": "english", "genres": ["Fantasy"]})
        b = self.r.create(self.eva, {**volume(2, "Zwei"), "runtime_min": 300, "genres": ["Krimis"]})
        series = self.r.create_series(self.admin, {"title": "Haus-Reihe"}, [volume(3), volume(4)])
        self.r.decline(b["id"], self.admin, "nein")
        self.clock.now += 3 * 86400                                       # arrives three days later
        item = self.r.get(a["id"])
        item["path"] = self.book_folder("Eins", 5000)
        self.r._save(item, "available")
        item = self.r.get(old["id"])
        item["path"] = "/etc"                                             # outside the library: not measured
        self.r._save(item, "available")

        s = self.stats.summary("90")
        eva = next(u for u in s["users"] if u["name"] == "Eva")
        self.assertEqual((eva["books"], eva["requests"], eva["available"], eva["declined"]), (2, 2, 1, 1))
        self.assertEqual(eva["minutes"], 600)                             # declined doesn't count
        self.assertEqual((eva["bytes"], eva["english"], eva["top_genres"][0]), (5000, 1, "Fantasy"))
        self.assertEqual(round(eva["wait_median"] / 86400), 3)
        admin = next(u for u in s["users"] if u["name"] == "Admin")
        self.assertEqual((admin["books"], admin["requests"], admin["series"]), (2, 1, 1))   # a series is one request
        self.assertEqual(s["users"][-1]["name"], "Ole")                   # nothing asked: still listed, last
        self.assertEqual(s["total"]["books"], 4)
        self.assertEqual(len(s["months"]), 12)
        everything = self.stats.summary("all")
        self.assertEqual(everything["total"]["books"], 5)
        self.assertEqual(everything["total"]["bytes"], 5000)              # /etc stayed unmeasured
        self.assertEqual(series["group"]["count"], 2)
        with self.assertRaises(ValueError):
            self.stats.summary("1000")

    def test_deleted_account(self):
        self.r.create(self.eva, volume(1))
        self.accounts.delete(self.eva["id"])
        users = {u["name"]: u for u in self.stats.summary("all")["users"]}
        self.assertNotIn("Eva", users)
        self.assertEqual(users["gelöschtes Konto"]["books"], 1)         # the requests stay counted


if __name__ == "__main__":
    unittest.main()
