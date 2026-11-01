"""Invitations by link: expiry, uses, revoking, guessing, and the way through the web."""
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
import config  # noqa: E402
import db as dbm  # noqa: E402
import invites as inv  # noqa: E402
import services  # noqa: E402
from web import server  # noqa: E402

PW = "Ein-gutes-Wort-42"


class Clock:
    def __init__(self):
        self.now = 1_000_000.0

    def __call__(self):
        return self.now


class InvitesTest(unittest.TestCase):
    def setUp(self):
        self.db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        self.addCleanup(self.db.close)
        self.clock = Clock()
        self.accounts = acc.Accounts(self.db, clock=self.clock)
        self.inv = inv.Invites(self.db, self.accounts, clock=self.clock)

    def test_single_use_link(self):
        i = self.inv.create("admin", "Mama", days=7, uses=1, quota=3)
        self.assertEqual(i["status"], "open")
        self.assertGreaterEqual(len(i["token"]), 30)
        user = self.inv.accept(i["token"], "Mama", PW)
        self.assertEqual((user["role"], user["quota"]), ("user", 3))       # never an admin through a link
        self.assertEqual(self.accounts.authenticate("mama", PW, "1.2.3.4")["name"], "Mama")
        after = self.inv.get(i["id"])
        self.assertEqual((after["status"], after["uses"], after["used_by"][0]["name"]), ("used", 1, "Mama"))
        with self.assertRaises(inv.InviteError):
            self.inv.accept(i["token"], "Papa", PW)
        self.assertIsNone(self.accounts.by_name("Papa"))

    def test_starting_rights(self):
        """The invitation decides what the new account may do – but never more than the family rights."""
        i = self.inv.create("admin", "Papa", perms={"auto_approve": True, "interactive_search": True, "admin": True})
        self.assertEqual(i["perms"], {"see_all_requests": False, "auto_approve": True, "interactive_search": True})
        user = self.inv.accept(i["token"], "Papa", PW)
        self.assertEqual((user["role"], user["perms"]),
                         ("user", {"see_all_requests": False, "auto_approve": True, "interactive_search": True}))
        plain = self.inv.accept(self.inv.create("admin")["token"], "Oma", PW)
        self.assertFalse(any(plain["perms"].values()))                      # default: request only, with approval
        with self.assertRaises(ValueError):
            self.inv.create("admin", perms="alles")

    def test_expiry_and_unlimited(self):
        short = self.inv.create("admin", days=1)
        forever = self.inv.create("admin", days=0, uses=5)
        self.clock.now += 2 * 86400
        self.assertEqual(self.inv.get(short["id"])["status"], "expired")
        with self.assertRaises(inv.InviteError):
            self.inv.check(short["token"])
        self.inv.accept(forever["token"], "Eva", PW)
        self.inv.accept(forever["token"], "Ole", PW)
        self.assertEqual(self.inv.get(forever["id"])["uses"], 2)

    def test_revoke_and_delete(self):
        i = self.inv.create("admin")
        self.assertEqual(self.inv.revoke(i["id"])["status"], "revoked")
        with self.assertRaises(inv.InviteError):
            self.inv.accept(i["token"], "Eva", PW)
        self.inv.delete(i["id"])
        self.assertEqual(self.inv.list(), [])

    def test_bad_name_or_password_keeps_the_link(self):
        i = self.inv.create("admin")
        self.accounts.create("Eva", PW)
        for name, pw in (("Eva", PW), ("Neu", "kurz"), ("", PW)):
            with self.assertRaises(ValueError):
                self.inv.accept(i["token"], name, pw)
        self.assertEqual(self.inv.get(i["id"])["uses"], 0)        # nothing used up, no half account
        self.assertEqual(self.accounts.count(), 1)
        self.inv.accept(i["token"], "Neu", PW)

    def test_last_use_cannot_be_taken_twice(self):
        """Two people with the same single-use link at the same moment: only one account."""
        i = self.inv.create("admin")
        results = []

        def take(name):
            try:
                self.inv.accept(i["token"], name, PW)
                results.append(name)
            except inv.InviteError:
                pass
        threads = [threading.Thread(target=take, args=(f"Person{n}",)) for n in range(5)]
        for th in threads:
            th.start()
        for th in threads:
            th.join()
        self.assertEqual(len(results), 1)
        self.assertEqual(self.accounts.count(), 1)

    def test_limits(self):
        for kw in ({"days": 400}, {"uses": 0}, {"uses": 101}, {"quota": -1}, {"days": "x"}):
            with self.assertRaises(ValueError):
                self.inv.create("admin", **kw)
        self.assertEqual(len(self.inv.create("admin", "x" * 200)["note"]), inv.NOTE_MAX)
        self.assertFalse(inv.valid_token_format("../../etc"))
        self.assertFalse(inv.valid_token_format("kurz"))


class InviteWebTest(unittest.TestCase):
    OWN = {"X-Requested-With": "Hoerwerk", "Content-Type": "application/json"}

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="inv-")
        for d in ("config", "inbox", "lib"):
            os.makedirs(os.path.join(cls.tmp, d))
        env = {k: os.path.join(cls.tmp, v) for k, v in {"CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
        cls.app = services.App(config.Config.from_env({**env, "AUDIOBOOK_WATCH": "false"}), start_workers=False)
        cls.server = server.make_server(cls.app, "127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        admin = cls.app.accounts.create("admin", "Test-Kaffee-123", "admin")
        cls.cookie = "hw_session=" + cls.app.accounts.start_session(admin["id"], False)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def req(self, method, path, body=None, cookie=True, ip=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        headers = {"Host": f"127.0.0.1:{self.port}", **self.OWN}
        if cookie:
            headers["Cookie"] = self.cookie if cookie is True else cookie
        c.request(method, path, body=json.dumps(body).encode() if body is not None else None, headers=headers)
        r = c.getresponse()
        out = (r.status, dict(r.getheaders()), r.read())
        c.close()
        return out

    def setUp(self):
        self.app.update_settings({"public_url": "https://hoerwerk.example.de"})

    def test_links_always_point_to_the_public_address(self):
        self.app.update_settings({"public_url": ""})
        self.assertEqual(self.req("POST", "/api/invites", {"days": 1})[0], 400)     # no address from outside: no link
        # opened once through the trusted proxy over HTTPS: that address is taken over
        old = self.app.config
        import dataclasses
        self.app.config = dataclasses.replace(old, trusted_proxies=config.parse_networks("127.0.0.1"))
        try:
            c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
            c.request("GET", "/api/invites", headers={"Host": "hoerwerk.example.org", "Cookie": self.cookie,
                                                      "X-Forwarded-For": "203.0.113.9", "X-Forwarded-Proto": "https"})
            self.assertEqual(json.loads(c.getresponse().read())["public_url"], "https://hoerwerk.example.org")
            c.close()
        finally:
            self.app.config = old
        link = json.loads(self.req("POST", "/api/invites", {"days": 1})[2])["link"]
        self.assertTrue(link.startswith("https://hoerwerk.example.org/invite/"), link)   # also when made from home
        # invalid addresses are refused in the settings
        status, _, _ = self.req("PUT", "/api/settings", {"public_url": "javascript:alert(1)"})
        self.assertEqual(status, 400)

    def test_admin_creates_person_accepts(self):
        status, _, body = self.req("POST", "/api/invites", {"note": "Mama", "days": 7, "uses": 1})
        self.assertEqual(status, 200)
        token = json.loads(body)["token"]
        self.assertEqual(json.loads(body)["link"], f"https://hoerwerk.example.de/invite/{token}")
        # the page itself is open (no account yet), and shows the sign-up form
        status, headers, page = self.req("GET", f"/invite/{token}", cookie=False)
        self.assertEqual(status, 200)
        self.assertIn(b'id="inviteForm"', page)
        self.assertEqual(self.req("GET", "/invite/..%2f..%2fetc", cookie=False)[0], 404)
        status, _, body = self.req("GET", f"/api/invite/{token}", cookie=False)
        self.assertEqual((status, json.loads(body)["note"]), (200, "Mama"))
        status, headers, body = self.req("POST", f"/api/invite/{token}", {"name": "Mama", "password": PW}, cookie=False)
        self.assertEqual(status, 200, body)
        self.assertIn("hw_session=", headers["Set-Cookie"])          # signed in right away
        new_cookie = headers["Set-Cookie"].split(";")[0]
        me = json.loads(self.req("GET", "/api/auth/me", cookie=new_cookie)[2])
        self.assertEqual((me["name"], me["role"]), ("Mama", "user"))
        # used up – and the family account can't manage invitations
        self.assertEqual(self.req("POST", f"/api/invite/{token}", {"name": "Papa", "password": PW}, cookie=False)[0], 404)
        self.assertEqual(self.req("GET", "/api/invites", cookie=new_cookie)[0], 403)
        listed = json.loads(self.req("GET", "/api/invites")[2])["invites"]
        self.assertEqual([(i["note"], i["status"]) for i in listed if i["token"] == token], [("Mama", "used")])

    def test_guessing_links_is_braked(self):
        self.app.accounts.ip_fails.clear()                 # other tests failed from the same address
        for n in range(acc.IP_FAILS):
            self.assertEqual(self.req("GET", f"/api/invite/{'x' * 30}{n}", cookie=False)[0], 404)
        self.assertEqual(self.req("GET", f"/api/invite/{'y' * 32}", cookie=False)[0], 429)
        self.app.accounts.ip_fails.clear()

    def test_without_the_header_no_account(self):
        token = json.loads(self.req("POST", "/api/invites", {"days": 1})[2])["token"]
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.request("POST", f"/api/invite/{token}", body=json.dumps({"name": "Fremd", "password": PW}).encode(),
                  headers={"Host": f"127.0.0.1:{self.port}", "Content-Type": "application/json"})
        self.assertEqual(c.getresponse().status, 403)                 # a foreign page can't use the link (CSRF)
        c.close()


if __name__ == "__main__":
    unittest.main()
