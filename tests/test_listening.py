"""Listening history: links to Audiobookshelf / Plex, numbers per account, seeds for a personal "Für dich"."""
import os
import sys
import tempfile
import threading
import time
import unittest
from datetime import datetime

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import abs as absm  # noqa: E402
import accounts as acc  # noqa: E402
import db as dbm  # noqa: E402
import discover  # noqa: E402
import listening as ls  # noqa: E402
import plex as plexm  # noqa: E402
from config import default_settings  # noqa: E402

PW = "Ein-gutes-Wort-42"
NOW = time.time()
DAY = 86400


def day(offset):
    return datetime.fromtimestamp(NOW - offset * DAY).strftime("%Y-%m-%d")


class FakeAbs:
    def __init__(self):
        self.calls = 0
        self.down = False

    def public_config(self):
        return {"url": "http://abs", "token_set": True}

    def users(self):
        return [{"id": "u-eva", "name": "eva"}, {"id": "u-ole", "name": "ole"}]

    def library_items(self):
        return {"i1": {"title": "Das stille Haus", "author": "Anna Autorin", "asin": "B0HAUS0001", "duration": 36000},
                "i2": {"title": "Ohne ASIN", "author": "Bert Berger", "asin": None, "duration": 20000},
                "i3": {"title": "Kurz reingehört", "author": "C. C.", "asin": "B0KURZ0001", "duration": 10000}}

    def progress(self, user_id):
        self.calls += 1
        if self.down:
            raise absm.AbsError("Audiobookshelf nicht erreichbar")
        return [{"libraryItemId": "i1", "progress": 1.0, "isFinished": True, "finishedAt": (NOW - 2 * DAY) * 1000,
                 "lastUpdate": (NOW - 2 * DAY) * 1000},
                {"libraryItemId": "i2", "progress": 0.4, "isFinished": False, "lastUpdate": (NOW - 1 * DAY) * 1000},
                {"libraryItemId": "i3", "progress": 0.05, "isFinished": False, "lastUpdate": (NOW - 400 * DAY) * 1000}]

    def listening_stats(self, user_id):
        return {"totalTime": 9000, "days": {day(1): 3600, day(2): 1800, day(100): 3600}}

    def check_login(self, username, password):
        if password != "richtig":
            raise absm.AbsError("Audiobookshelf: Benutzername oder Passwort falsch")
        return {"id": "u-" + username, "name": username}


class FakePlex:
    def public_config(self):
        return {"url": "http://plex", "token_set": True}

    def accounts(self):
        return [{"id": "1", "name": "Admin"}, {"id": "7", "name": "Mama"}]

    def history(self, account_id, since=0):
        return [{"key": "t1", "book_key": "b1", "title": "Teil 1", "book": "Der Schimmelreiter", "author": "Theodor Storm",
                 "viewed": int(NOW - 3 * DAY)},
                {"key": "t2", "book_key": "b1", "title": "Teil 2", "book": "Der Schimmelreiter", "author": "Theodor Storm",
                 "viewed": int(NOW - 3 * DAY) + 60}]

    def durations(self, keys):
        return {"t1": 1800.0, "t2": 1200.0}


class FakeIndex:
    lock = threading.Lock()
    entries = {}

    def find(self, b):
        known = {"Ohne ASIN": "B0OHNE0001", "Der Schimmelreiter": "B0STORM001"}
        return [{"asin": known[b["title"]]}] if b.get("title") in known else []


class ListeningTest(unittest.TestCase):
    def setUp(self):
        self.db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        self.addCleanup(self.db.close)
        self.accounts = acc.Accounts(self.db)
        self.eva = self.accounts.create("Eva", PW)
        self.ole = self.accounts.create("Ole", PW)
        self.abs, self.plex = FakeAbs(), FakePlex()
        self.l = ls.Listening(self.accounts, self.abs, self.plex, FakeIndex())

    def test_links_belong_to_one_account(self):
        self.l.set_link(self.eva["id"], "abs", {"id": "u-eva", "name": "eva"})
        self.l.set_link(self.ole["id"], "abs", {"id": "u-eva", "name": "eva"})        # moved to Ole
        self.assertIsNone(self.l.links(self.eva["id"])["abs"])
        self.assertEqual(self.l.links(self.ole["id"])["abs"]["name"], "eva")
        self.l.set_link(self.ole["id"], "abs", None)
        self.assertIsNone(self.l.links(self.ole["id"])["abs"])
        with self.assertRaises(ls.LinkError):
            self.l.set_link(self.eva["id"], "spotify", {"id": "x"})

    def test_own_audiobookshelf_by_signing_in(self):
        with self.assertRaises(absm.AbsError):
            self.l.link_own_abs(self.eva["id"], "eva", "falsch")
        self.assertIsNone(self.l.links(self.eva["id"])["abs"])
        self.assertEqual(self.l.link_own_abs(self.eva["id"], "eva", "richtig")["abs"], {"id": "u-eva", "name": "eva"})
        self.assertNotIn("richtig", str(self.accounts.prefs(self.eva["id"])))           # the password is not kept

    def test_numbers_since(self):
        self.l.set_link(self.eva["id"], "abs", {"id": "u-eva", "name": "eva"})
        self.l.set_link(self.eva["id"], "plex", {"id": "7", "name": "Mama"})
        s = self.l.summary(self.eva["id"], NOW - 30 * DAY)
        self.assertEqual(s["linked"], ["abs", "plex"])
        self.assertEqual(s["seconds"], 3600 + 1800 + 1800 + 1200)                       # ABS days + Plex plays, 30 days
        self.assertEqual((s["books"], s["finished"]), (3, 2))                            # Haus, Ohne ASIN, Schimmelreiter
        self.assertAlmostEqual(s["last"], NOW - DAY, delta=2)
        self.assertEqual(self.l.summary(self.eva["id"])["seconds"], 3600 * 2 + 1800 + 3000)
        self.assertEqual(self.l.summary(self.ole["id"])["linked"], [])

    def test_cached_and_errors_do_not_break(self):
        self.l.set_link(self.eva["id"], "abs", {"id": "u-eva", "name": "eva"})
        self.l.summary(self.eva["id"])
        self.l.summary(self.eva["id"])
        self.assertEqual(self.abs.calls, 1)                                              # 15 minutes cache
        self.l.set_link(self.eva["id"], "abs", {"id": "u-eva", "name": "eva"})           # a new link: fresh numbers
        self.abs.down = True
        s = self.l.summary(self.eva["id"])
        self.assertEqual((s["seconds"], s["errors"]), (0, ["Audiobookshelf nicht erreichbar"]))

    def test_seeds_for_you(self):
        self.l.set_link(self.eva["id"], "abs", {"id": "u-eva", "name": "eva"})
        self.l.set_link(self.eva["id"], "plex", {"id": "7", "name": "Mama"})
        seeds = self.l.seeds(self.eva["id"])
        # newest first; ASIN from Audiobookshelf or found in the library; "only looked in" (5 %) is no seed
        self.assertEqual([s["asin"] for s in seeds], ["B0OHNE0001", "B0HAUS0001", "B0STORM001"])


class ServerApiTest(unittest.TestCase):
    """The answers of Audiobookshelf and Plex, read the way the servers send them."""

    def test_audiobookshelf(self):
        db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        a = absm.Audiobookshelf(db, call=lambda m, url, token, timeout=15: {
            "/api/users": {"users": [{"id": "u1", "username": "eva", "isActive": True}, {"username": "kaputt"}]},
            "/api/users/u1": {"mediaProgress": [{"libraryItemId": "i1", "progress": 0.5},
                                                {"libraryItemId": "p1", "episodeId": "e1"}]},
            "/api/libraries/L1/items": {"results": [{"id": "i1", "media": {"duration": 3600.5, "metadata": {
                "title": "Das stille Haus", "authorName": "Anna Autorin", "asin": "b0haus0001"}}}]},
        }[url.split("http://abs")[1].split("?")[0]])
        a.update_config({"url": "http://abs", "token": "geheim", "library": "L1"})
        self.assertEqual(a.users(), [{"id": "u1", "name": "eva", "active": True}])
        self.assertEqual(len(a.progress("u1")), 1)                                       # no podcast episodes
        self.assertEqual(a.library_items()["i1"], {"title": "Das stille Haus", "author": "Anna Autorin",
                                                   "asin": "B0HAUS0001", "duration": 3600.5})
        a._login_call = lambda url, u, p: {"id": "u1", "name": u} if url == "http://abs/login" else {}
        self.assertEqual(a.check_login("eva", "x"), {"id": "u1", "name": "eva"})

    def test_plex(self):
        db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        asked = []

        def call(method, url, token, timeout=15):
            asked.append(url)
            if "/accounts" in url:
                return {"MediaContainer": {"Account": [{"id": 0, "name": ""}, {"id": 1, "name": "Viktor"},
                                                       {"id": 7, "name": "Mama"}]}}
            if "/history/" in url:
                return {"MediaContainer": {"Metadata": [
                    {"ratingKey": "11", "parentRatingKey": "10", "title": "Track", "parentTitle": "Der Schimmelreiter",
                     "grandparentTitle": "Theodor Storm", "viewedAt": 2000},
                    {"ratingKey": "12", "parentRatingKey": "10", "viewedAt": 500}]}}
            return {"MediaContainer": {"Metadata": [{"duration": 1800000}]}}
        p = plexm.Plex(db, call=call)
        p.update_config({"url": "http://plex:32400", "token": "geheim", "section": "3"})
        self.assertEqual(p.accounts(), [{"id": "1", "name": "Viktor"}, {"id": "7", "name": "Mama"}])
        plays = p.history("7", since=1000)
        self.assertEqual([(x["key"], x["book"], x["author"]) for x in plays], [("11", "Der Schimmelreiter", "Theodor Storm")])
        self.assertIn("accountID=7", asked[-1])
        self.assertIn("librarySectionID=3", asked[-1])
        self.assertEqual(p.durations(["11"]), {"11": 1800.0})
        n = len(asked)
        p.durations(["11"])
        self.assertEqual(len(asked), n)                                                  # remembered


class PersonalForYouTest(unittest.TestCase):
    def test_personal_list_from_what_was_heard(self):
        class Client:
            def similar(self, asin, region, limit=20):
                base = {"authors": ["X"], "narrators": [], "rating": 4.7, "ratings": 500, "genres": ["Krimis & Thriller"],
                        "format_type": "unabridged", "language": "german", "is_adult": False}
                return {"B0HEARD001": [{**base, "asin": "B0NEW00001", "title": "Neu für Eva"}],
                        "B0LIB00001": [{**base, "asin": "B0NEW00002", "title": "Für alle"}]}.get(asin, [])

        class Index:
            lock = threading.Lock()
            entries = {"a": {"asin": "B0LIB00001", "title": "Neu in der Bibliothek", "mtime": 1}}

            def find(self, b):
                return []
        db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        d = discover.Discover(db, Client(), Index(), lambda: default_settings({}), fetch=lambda u: b"", prepare=False)
        d.seeds = lambda uid: [{"asin": "B0HEARD001", "title": "Gehört"}] if uid == 1 else []
        d.refresh("foryou", user_id=1, wait=True)
        d.refresh("foryou", user_id=2, wait=True)
        eva, other = d.get("foryou", user_id=1), d.get("foryou", user_id=2)
        self.assertEqual(([i["title"] for i in eva["items"]], eva["personal"]), (["Neu für Eva"], True))
        self.assertEqual(eva["items"][0]["because"], ["Gehört"])
        self.assertEqual(([i["title"] for i in other["items"]], other["personal"]), (["Für alle"], False))


class ListeningWebTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import http.client  # noqa: F401
        import shutil  # noqa: F401
        import config
        import services
        from web import server
        cls.tmp = tempfile.mkdtemp(prefix="listen-")
        env = {k: os.path.join(cls.tmp, v) for k, v in {"CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
        for d in env.values():
            os.makedirs(d, exist_ok=True)
        cls.app = services.App(config.Config.from_env({**env, "AUDIOBOOK_WATCH": "false"}), start_workers=False)
        cls.app.listening.abs, cls.app.listening.plex = FakeAbs(), FakePlex()
        cls.server = server.make_server(cls.app, "127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        a = cls.app.accounts
        cls.admin, cls.eva = a.create("Admin", PW, "admin"), a.create("Eva", PW)
        cls.cookies = {u["name"]: "hw_session=" + a.start_session(u["id"], False) for u in (cls.admin, cls.eva)}

    @classmethod
    def tearDownClass(cls):
        import shutil
        cls.server.shutdown()
        cls.server.server_close()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def req(self, who, method, path, body=None):
        import http.client
        import json
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=20)
        c.request(method, path, body=json.dumps(body).encode() if body is not None else None,
                  headers={"Host": f"127.0.0.1:{self.port}", "X-Requested-With": "Hoerwerk",
                           "Content-Type": "application/json", "Cookie": self.cookies[who]})
        r = c.getresponse()
        out = (r.status, json.loads(r.read() or b"{}"))
        c.close()
        return out

    def test_rights_and_linking(self):
        self.assertEqual(self.req("Eva", "GET", "/api/listening/accounts")[0], 403)           # only admins see everyone
        self.assertEqual(self.req("Eva", "PUT", "/api/listening/link", {"user_id": self.eva["id"], "kind": "plex",
                                                                         "target_id": "7"})[0], 403)
        status, me = self.req("Eva", "GET", "/api/me/listening")
        self.assertEqual((status, me["available"], me["links"]["abs"]), (200, {"abs": True, "plex": True}, None))
        self.app.accounts.ip_fails.clear()
        self.assertEqual(self.req("Eva", "POST", "/api/me/listening/abs", {"username": "eva", "password": "falsch"})[0], 400)
        self.assertEqual(len(self.app.accounts.ip_fails), 1)                                   # counts on the brake
        status, out = self.req("Eva", "POST", "/api/me/listening/abs", {"username": "eva", "password": "richtig"})
        self.assertEqual((status, out["links"]["abs"]["id"]), (200, "u-eva"))
        self.app.accounts.ip_fails.clear()
        # the admin assigns the Plex profile; an unknown profile is refused
        self.assertEqual(self.req("Admin", "PUT", "/api/listening/link", {"user_id": self.eva["id"], "kind": "plex",
                                                                           "target_id": "999"})[0], 400)
        status, out = self.req("Admin", "PUT", "/api/listening/link", {"user_id": self.eva["id"], "kind": "plex", "target_id": "7"})
        self.assertEqual((status, out["links"]["plex"]["name"]), (200, "Mama"))
        status, everyone = self.req("Admin", "GET", "/api/listening/accounts")
        self.assertEqual([p["name"] for p in everyone["plex"]], ["Admin", "Mama"])
        status, stats = self.req("Admin", "GET", "/api/stats?period=30")
        eva = next(u for u in stats["users"] if u["name"] == "Eva")
        self.assertGreater(eva["listen"]["seconds"], 0)
        self.assertEqual(stats["listen"]["linked"], 1)
        status, me = self.req("Eva", "GET", "/api/me/listening")
        self.assertGreater(me["recent"]["seconds"], 0)
        self.assertEqual(self.req("Eva", "DELETE", "/api/me/listening/plex")[0], 200)
        self.assertEqual(self.req("Eva", "DELETE", "/api/me/listening/spotify")[0], 404)


if __name__ == "__main__":
    unittest.main()
