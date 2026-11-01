"""Requests (Seerr-like), release verdicts (Radarr-like), rights between accounts, second factor, audit."""
import http.client
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import accounts as acc  # noqa: E402
import bookrequests as br  # noqa: E402
import config  # noqa: E402
import db as dbm  # noqa: E402
import releases  # noqa: E402
import services  # noqa: E402
from web import server  # noqa: E402

PW = "korrekt pferd batterie heftklammer"
BOOK = {"asin": "b0req00001", "title": "Das stille Haus", "authors": ["Anna Autorin"], "runtime_min": 600,
        "language": "german"}
GB = 1 << 30


class ReleaseVerdictTest(unittest.TestCase):
    def ev(self, title, **kw):
        rel = {"title": title, "size": kw.pop("size", int(0.4 * GB)), **kw}
        return releases.evaluate(rel, {**BOOK, **kw.pop("book", {})})

    def test_accepts_and_scores(self):
        m4b = self.ev("Anna.Autorin-Das.stille.Haus.German.M4B", format="M4B", language="de")
        mp3 = self.ev("Anna Autorin - Das stille Haus (Ungekürzt) MP3", format="MP3")
        self.assertTrue(m4b["ok"] and mp3["ok"])
        self.assertGreater(m4b["score"], mp3["score"])
        self.assertEqual(m4b["kbps"], 95)                     # 0.4 GB over 10 h
        self.assertIsNone(m4b["reason"])

    def test_rejections_have_reasons(self):
        cases = {
            "Bert.Berger-Das.stille.Haus": "Autor",
            "Anna.Autorin-Das.laute.Meer": "Titel passt nicht",
            "Anna.Autorin-Das.stille.Haus.GEKUERZT": "gekürzt",
            "Anna.Autorin-Das.stille.Haus.ENGLISH": "andere Sprache",
        }
        extra = {"Anna.Autorin-Das.stille.Haus.GEKUERZT": {"abridged": True},
                 "Anna.Autorin-Das.stille.Haus.ENGLISH": {"language": "en"}}
        for title, why in cases.items():
            v = releases.evaluate({"title": title, **extra.get(title, {})}, BOOK)
            self.assertFalse(v["ok"], title)
            self.assertIn(why, v["reason"], title)
        self.assertIn("gesperrt", releases.evaluate({"title": "Anna Autorin Das stille Haus"}, BOOK,
                                                    blocked=["Anna Autorin Das stille Haus"])["reason"])

    def test_size_must_fit_the_runtime(self):
        tiny = releases.evaluate({"title": "Anna Autorin Das stille Haus", "size": 30 << 20}, BOOK)
        self.assertFalse(tiny["ok"])
        self.assertIn("zu klein", tiny["reason"])
        ok = releases.evaluate({"title": "Anna Autorin Das stille Haus", "size": 400 << 20}, BOOK)   # ~56 kbit/s
        self.assertTrue(ok["ok"])
        huge = releases.evaluate({"title": "Anna Autorin Das stille Haus", "size": 20 * GB}, BOOK)
        self.assertIn("zu groß", huge["reason"])

    def test_packs(self):
        book = {**BOOK, "series": "Haus", "series_position": "3"}
        self.assertTrue(releases.evaluate({"title": "Anna Autorin - Das stille Haus 01 - 05"}, book)["ok"])
        self.assertIn("Paket ohne", releases.evaluate({"title": "Anna Autorin - Das stille Haus 05 - 07"}, book)["reason"])


class FakeUsenet:
    def __init__(self):
        self.results = []
        self.sent = []
        self.items = []
        self.indexers = 1

    def configured(self):
        return {"indexers": self.indexers, "sab": True}

    def search(self, query, language="", extra=None):
        return {"results": [dict(r) for r in self.results], "errors": [], "queries": [query]}

    def send(self, rid, hints=None):
        self.sent.append((rid, hints))
        return {"nzo_id": f"nzo{len(self.sent)}", "title": next(r["title"] for r in self.results if r["id"] == rid)}

    def activity(self):
        return {"items": self.items}


class FakeIndex:
    def __init__(self):
        self.lock = threading.Lock()
        self.entries = {}
        self.owned = set()

    def find(self, b):
        return [{"path": "x"}] if b.get("title") in self.owned else []


class FakeNotifier:
    def __init__(self):
        self.sent, self.direct = [], []

    def send(self, event, title, message, priority=None):
        self.sent.append(event)

    def send_to(self, topic, title, message):
        self.direct.append((topic, title))


class Job:
    def __init__(self, rid, status, dest="", error=""):
        self.hints, self.status, self.dest, self.error = {"request_id": rid}, status, dest, error


GOOD = {"id": "r1", "title": "Anna.Autorin-Das.stille.Haus.German.M4B", "size": 400 << 20, "format": "M4B",
        "language": "de", "indexer": "X"}
ALSO = {"id": "r2", "title": "Anna Autorin - Das stille Haus MP3", "size": 500 << 20, "format": "MP3", "indexer": "Y"}


class RequestsTest(unittest.TestCase):
    def setUp(self):
        self.db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        self.addCleanup(self.db.close)
        self.settings = config.default_settings({})
        self.accounts = acc.Accounts(self.db)
        self.admin = self.accounts.create("Admin", PW, "admin")
        self.eva = self.accounts.create("Eva", PW)
        self.tom = self.accounts.create("Tom", PW, perms={"auto_approve": True})
        self.usenet, self.index, self.notifier = FakeUsenet(), FakeIndex(), FakeNotifier()
        self.jobs = []
        self.r = br.Requests(self.db, self.accounts, self.usenet, self.index, self.notifier, lambda: self.settings,
                             jobs=lambda: self.jobs)
        self.r.search_soon = self.r.search            # synchronous in tests

    def test_pending_then_approved_then_grabbed_then_available(self):
        self.usenet.results = [ALSO, GOOD]
        req = self.r.create(self.eva, BOOK)
        self.assertEqual(req["status"], "pending")
        self.assertEqual(self.notifier.sent, ["request_new"])
        self.assertEqual(self.usenet.sent, [])                     # nothing happens before approval
        req = self.r.approve(req["id"], self.admin)
        req = self.r.get(req["id"])
        self.assertEqual(req["status"], "downloading")
        rid, hints = self.usenet.sent[0]
        self.assertEqual(rid, "r1")                                # M4B in German beats MP3
        self.assertEqual((hints["asin"], hints["request_id"]), ("B0REQ00001", req["id"]))
        self.usenet.items = [{"id": "nzo1", "done": True, "status": "Completed"}]
        self.r.tick()
        self.assertEqual(self.r.get(req["id"])["status"], "processing")
        self.jobs.append(Job(req["id"], "done", "/audiobooks/Anna Autorin/Das stille Haus"))
        self.r.tick()
        done = self.r.get(req["id"])
        self.assertEqual((done["status"], done["path"]), ("available", "/audiobooks/Anna Autorin/Das stille Haus"))

    def test_failed_download_blocks_release_and_tries_the_next(self):
        self.usenet.results = [GOOD, ALSO]
        req = self.r.create(self.tom, BOOK)                        # auto approve
        self.assertEqual(self.r.get(req["id"])["status"], "downloading")
        self.usenet.items = [{"id": "nzo1", "done": True, "status": "Failed", "error": "CRC"}]
        self.r.tick()
        again = self.r.get(req["id"])
        self.assertEqual(again["blocked"], [GOOD["title"]])
        self.assertEqual(again["status"], "downloading")
        self.assertEqual(self.usenet.sent[-1][0], "r2")           # the next release

    def test_import_that_fails_counts_too_and_gives_up_after_max(self):
        self.usenet.results = [GOOD]
        req = self.r.create(self.tom, BOOK)
        self.jobs.append(Job(req["id"], "failed", error="falsches Buch"))
        self.r.on_job(self.jobs[-1])
        self.assertEqual(self.r.get(req["id"])["status"], "searching")      # GOOD blocked, nothing else
        item = self.r.get(req["id"])
        item["attempts"] = br.MAX_ATTEMPTS
        item["release"] = "x"
        self.r._failed_release(item, "kaputt")
        self.assertEqual(self.r.get(req["id"])["status"], "failed")
        self.assertIn("request_failed", self.notifier.sent)

    def test_book_found_in_the_inbox_fulfils_the_request(self):
        """Not downloaded by Hörwerk, but the recognition linked it: searching stops, available when filed."""
        self.usenet.indexers = 0                                  # nothing to download
        req = self.r.create(self.tom, BOOK)
        self.assertEqual(self.r.get(req["id"])["status"], "searching")
        self.assertEqual([r["id"] for r in self.r.open_for_matching()], [req["id"]])
        job = Job(req["id"], "converting")
        self.r.on_job(job)
        self.assertEqual(self.r.get(req["id"])["status"], "processing")
        job.status, job.dest = "done", "/audiobooks/Anna Autorin/Das stille Haus"
        self.r.on_job(job)
        self.assertEqual(self.r.get(req["id"])["status"], "available")
        self.assertEqual(self.r.open_for_matching(), [])

    def test_quota_duplicates_owned(self):
        self.settings["request_quota"] = 2
        a = self.r.create(self.eva, BOOK)
        self.r.create(self.eva, {**BOOK, "asin": None, "title": "Zweites Buch"})
        with self.assertRaises(br.RequestError) as e:
            self.r.create(self.eva, {**BOOK, "asin": None, "title": "Drittes Buch"})
        self.assertEqual(e.exception.status, 429)
        self.r.decline(a["id"], self.admin, "gibt es nur gekürzt")      # declined ones don't count
        self.r.create(self.eva, {**BOOK, "asin": None, "title": "Drittes Buch"})
        self.assertEqual(self.r.quota(self.eva), {"limit": 2, "used": 2, "left": 0})
        with self.assertRaises(br.RequestError) as e:
            self.r.create(self.tom, {**BOOK, "asin": None, "title": "Zweites Buch"})
        self.assertEqual(e.exception.status, 409)                         # someone asked for it already
        self.index.owned = {"Viertes Buch"}
        with self.assertRaises(br.RequestError):
            self.r.create(self.admin, {**BOOK, "asin": None, "title": "Viertes Buch"})
        self.assertIsNone(self.r.quota(self.admin)["limit"])              # admins: no limit

    def test_visibility_and_withdraw(self):
        mine = self.r.create(self.eva, BOOK)
        self.assertFalse(self.r.can_see(self.tom, mine))
        self.assertTrue(self.r.can_see(self.admin, mine))
        self.accounts.update(self.tom["id"], {"perms": {"see_all_requests": True}})
        tom = self.accounts.get(self.tom["id"])
        self.assertTrue(self.r.can_see(tom, mine))
        with self.assertRaises(br.RequestError):
            self.r.withdraw(mine["id"], tom)                              # seeing is not deleting
        self.r.withdraw(mine["id"], self.eva)
        self.assertIsNone(self.r.get(mine["id"]))
        out = br.summary(self.r.create(self.tom, BOOK), self.eva, False)
        self.assertNotIn("blocked", out)
        self.assertNotIn("user", out)

    def test_old_wanted_list_becomes_requests(self):
        self.db.put("wanted", {"B0X": {"asin": "B0X", "title": "Gemerkt", "authors": ["Max Muster"]}})
        self.assertEqual(self.r.import_wanted(), 1)
        self.assertIsNone(self.db.get("wanted"))
        self.assertEqual(self.r.list(self.admin)[0]["book"]["title"], "Gemerkt")

    def test_test_message_to_own_topic(self):
        import notify
        sent = []
        n = notify.Notifier(self.db)
        n.db.put("ntfy", {"url": "https://ntfy.example", "topic": "admin-topic", "events": {}})
        n._post = lambda url, body, headers: sent.append(body)
        self.assertIn("eva-buecher", n.test_to("eva-buecher"))
        self.assertEqual(sent[0]["topic"], "eva-buecher")
        with self.assertRaises(notify.NotifyError):
            n.test_to("bad topic/../x")

    def test_requester_hears_about_it(self):
        self.accounts.set_prefs(self.eva["id"], {"ntfy_topic": "eva-buecher"})
        req = self.r.create(self.eva, BOOK)
        self.r.decline(req["id"], self.admin, "nein")
        self.assertEqual(self.notifier.direct[-1][0], "eva-buecher")


class TotpTest(unittest.TestCase):
    def setUp(self):
        self.db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        self.addCleanup(self.db.close)
        self.now = [1_700_000_000.0]
        self.a = acc.Accounts(self.db, clock=lambda: self.now[0])
        self.u = self.a.create("Admin", PW, "admin")

    def code(self, secret, delta=0):
        return acc.totp_code(secret, int(self.now[0] // 30) + delta)

    def test_setup_login_replay(self):
        secret = self.a.totp_begin(self.u["id"])["secret"]
        self.assertFalse(self.a.totp_confirm(self.u["id"], "000000"))
        self.assertTrue(self.a.totp_confirm(self.u["id"], self.code(secret)))
        self.assertTrue(self.a.get(self.u["id"])["totp"])
        self.now[0] += 60
        ticket = self.a.ticket(self.u["id"], True)
        with self.assertRaises(acc.AuthError):
            self.a.redeem(ticket, "123456", "1.1.1.1")
        user = self.a.redeem(ticket, self.code(secret), "1.1.1.1")
        self.assertTrue(user["persistent"])
        with self.assertRaises(acc.AuthError):                       # a ticket works once
            self.a.redeem(ticket, self.code(secret), "1.1.1.1")
        t2 = self.a.ticket(self.u["id"], False)
        with self.assertRaises(acc.AuthError):                       # the same code again: replay
            self.a.redeem(t2, self.code(secret), "1.1.1.1")

    def test_ticket_tries_are_limited(self):
        secret = self.a.totp_begin(self.u["id"])["secret"]
        self.a.totp_confirm(self.u["id"], self.code(secret))
        ticket = self.a.ticket(self.u["id"], False)
        for _ in range(acc.TICKET_TRIES):
            with self.assertRaises(acc.AuthError):
                self.a.redeem(ticket, "000000", f"9.9.9.{_}")
        self.now[0] += 90
        with self.assertRaises(acc.AuthError):                       # even the right code: ticket used up
            self.a.redeem(ticket, self.code(secret), "1.1.1.1")


class WebRequestsTest(unittest.TestCase):
    """Rights through the real server: other people's requests, interactive search, audit."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="req-")
        env = {k: os.path.join(cls.tmp, v) for k, v in
               {"CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
        for d in env.values():
            os.makedirs(d, exist_ok=True)
        cls.app = services.App(config.Config.from_env({**env, "AUDIOBOOK_WATCH": "false"}), start_workers=False)
        cls.app.requests.search_soon = lambda rid: None
        cls.server = server.make_server(cls.app, "127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        a = cls.app.accounts
        cls.admin = a.create("Admin", PW, "admin")
        cls.eva = a.create("Eva", PW)
        cls.tom = a.create("Tom", PW)
        cls.cookies = {u["name"]: "hw_session=" + a.start_session(u["id"], False) for u in (cls.admin, cls.eva, cls.tom)}

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def req(self, who, method, path, body=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=20)
        c.request(method, path, body=json.dumps(body).encode() if body is not None else None,
                  headers={"Host": f"127.0.0.1:{self.port}", "X-Requested-With": "Hoerwerk",
                           "Content-Type": "application/json", "Cookie": self.cookies[who]})
        r = c.getresponse()
        out = (r.status, json.loads(r.read() or b"{}"))
        c.close()
        return out

    def test_rights(self):
        status, mine = self.req("Eva", "POST", "/api/requests", {"book": BOOK})
        self.assertEqual((status, mine["status"]), (200, "pending"))
        rid = mine["id"]
        # Tom can't see, withdraw or search Eva's request – and gets 404, not 403 (ids can't be probed)
        self.assertEqual(self.req("Tom", "GET", f"/api/requests/{rid}")[0], 404)
        self.assertEqual(self.req("Tom", "DELETE", f"/api/requests/{rid}")[0], 404)
        self.assertEqual(self.req("Tom", "POST", f"/api/requests/{rid}/releases", {})[0], 404)
        status, lst = self.req("Tom", "GET", "/api/requests?scope=all")
        self.assertTrue(all(i["mine"] for i in lst["items"]))                 # only his own, even when asking for all
        self.assertNotIn(rid, [i["id"] for i in lst["items"]])
        # Eva may see her own, but not search releases or approve
        self.assertEqual(self.req("Eva", "GET", f"/api/requests/{rid}")[0], 200)
        self.assertEqual(self.req("Eva", "POST", f"/api/requests/{rid}/releases", {})[0], 403)
        self.assertEqual(self.req("Eva", "POST", f"/api/requests/{rid}/approve")[0], 403)
        self.assertEqual(self.req("Eva", "POST", f"/api/requests/{rid}/grab", {"release_id": "x"})[0], 403)
        # the admin sees everyone's, with the name, and approves – which lands in the audit log
        status, lst = self.req("Admin", "GET", "/api/requests?scope=all")
        self.assertEqual(lst["items"][0]["user"], "Eva")
        self.assertGreaterEqual(lst["pending"], 1)
        self.assertEqual(self.req("Admin", "POST", f"/api/requests/{rid}/approve")[0], 200)
        status, log = self.req("Admin", "GET", "/api/audit")
        actions = [e["action"] for e in log["entries"]]
        self.assertIn("request_approved", actions)
        self.assertIn(f"POST /api/requests/{rid}/approve", actions)
        self.assertEqual(self.req("Eva", "GET", "/api/audit")[0], 403)

    def test_ids_can_not_be_probed(self):
        """Unknown and foreign ids answer the same 404 – nobody learns which requests exist."""
        status, mine = self.req("Eva", "POST", "/api/requests", {"book": {**BOOK, "asin": "B0PROBE01", "title": "Probe"}})
        self.assertEqual(status, 200)
        foreign = self.req("Tom", "GET", f"/api/requests/{mine['id']}")
        missing = self.req("Tom", "GET", "/api/requests/999999")
        self.assertEqual(foreign, missing)
        self.assertEqual(self.req("Tom", "GET", "/api/requests/abc")[0], 404)
        # a family account can't sneak a request for someone else or set its own status
        status, other = self.req("Tom", "POST", "/api/requests", {"book": {**BOOK, "asin": "B0PROBE02", "title": "Probe 2"},
                                                                  "user_id": self.eva["id"], "status": "available"})
        self.assertEqual((status, other["status"], other["mine"]), (200, "pending", True))

    def test_admin_second_factor_can_be_required(self):
        self.app.update_settings({"admin_totp_required": True})
        try:
            status, body = self.req("Admin", "GET", "/api/sources")
            self.assertEqual(status, 403)
            self.assertTrue(body["totp_setup"])
            self.assertEqual(self.req("Admin", "POST", "/api/auth/totp/setup")[0], 200)   # enrolling still works
        finally:
            self.app.update_settings({"admin_totp_required": False})


if __name__ == "__main__":
    unittest.main()
