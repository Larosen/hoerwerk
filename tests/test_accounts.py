"""Accounts and login: hashing, brake, sessions, rights, real client IP, security log."""
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
import authlog  # noqa: E402
import config  # noqa: E402
import db as dbm  # noqa: E402
import services  # noqa: E402
from web import server  # noqa: E402

PW = "korrekt-pferd-batterie"


class Clock:
    def __init__(self):
        self.now = 1_000_000.0

    def __call__(self):
        return self.now


class AccountsTest(unittest.TestCase):
    def setUp(self):
        self.db = dbm.Database(os.path.join(tempfile.mkdtemp(), "t.db"))
        self.addCleanup(self.db.close)
        self.clock = Clock()
        self.a = acc.Accounts(self.db, clock=self.clock)
        self.admin = self.a.create("Admin", PW, "admin")

    def test_hash_format_and_verify(self):
        h = acc.hash_password("geheim-geheim")
        self.assertTrue(h.startswith("scrypt$32768$8$1$"))
        self.assertNotEqual(h, acc.hash_password("geheim-geheim"))          # own salt every time
        self.assertTrue(acc.verify_password("geheim-geheim", h))
        self.assertFalse(acc.verify_password("geheim-geheiM", h))
        self.assertFalse(acc.verify_password("x", "garbage"))
        row = self.db.one("SELECT pw FROM users WHERE id=?", (self.admin["id"],))
        self.assertNotIn(PW, row["pw"])

    def test_rules(self):
        for bad in ("kurz", "admin-name", "x" * 300):
            with self.assertRaises(ValueError):
                self.a.create("admin-name", bad)
        with self.assertRaises(ValueError):
            self.a.create("admin", PW)                                   # names are case-insensitive
        with self.assertRaises(ValueError):
            self.a.create("bad\nname", PW)
        with self.assertRaises(ValueError):
            self.a.create("Eva", PW, role="root")

    def test_login_and_brake_per_ip(self):
        self.assertEqual(self.a.authenticate("admin", PW, "1.2.3.4")["name"], "Admin")
        for _ in range(acc.IP_FAILS):
            with self.assertRaises(acc.AuthError) as e:
                self.a.authenticate("admin", "falsch-falsch", "6.6.6.6")
            self.assertEqual(e.exception.status, 401)
        with self.assertRaises(acc.AuthError) as e:                       # blocked – even with the right password
            self.a.authenticate("admin", PW, "6.6.6.6")
        self.assertEqual((e.exception.status, e.exception.reason), (429, "ip_blocked"))
        self.assertEqual(self.a.authenticate("admin", PW, "1.2.3.4")["name"], "Admin")   # others are not locked out
        self.clock.now += acc.WINDOW + 1
        self.assertEqual(self.a.authenticate("admin", PW, "6.6.6.6")["name"], "Admin")

    def test_reasons_and_account_delay(self):
        with self.assertRaises(acc.AuthError) as e:
            self.a.authenticate("niemand", PW, "1.1.1.1")
        self.assertEqual(e.exception.reason, "unknown_user")
        for _ in range(4):
            with self.assertRaises(acc.AuthError):
                self.a.authenticate("admin", "falsch-falsch", f"9.9.9.{_}")
        self.assertEqual(self.a.delay_for("admin"), 2.0)
        self.a.authenticate("admin", PW, "1.1.1.2")
        self.assertEqual(self.a.delay_for("admin"), 0.0)
        eva = self.a.create("Eva", PW)
        self.a.update(eva["id"], {"disabled": True})
        with self.assertRaises(acc.AuthError) as e:
            self.a.authenticate("eva", PW, "1.1.1.3")
        self.assertEqual(e.exception.reason, "disabled")

    def test_sessions(self):
        short = self.a.start_session(self.admin["id"], False, "1.2.3.4", "Firefox")
        long = self.a.start_session(self.admin["id"], True)
        self.assertEqual(self.a.session_user(short)["name"], "Admin")
        self.assertIsNone(self.a.session_user("nope"))
        stored = [r["token_hash"] for r in self.db.all("SELECT token_hash FROM sessions")]
        self.assertNotIn(short, stored)                                   # only the hash is kept
        self.clock.now += acc.SESSION_SHORT + 1
        self.assertIsNone(self.a.session_user(short))
        self.assertIsNotNone(self.a.session_user(long))                   # used: extended
        self.clock.now += acc.SESSION_LONG - 100
        self.assertIsNotNone(self.a.session_user(long))
        self.a.set_password(self.admin["id"], "ein-neues-passwort")       # signs out everywhere
        self.assertIsNone(self.a.session_user(long))

    def test_disable_ends_sessions_and_last_admin_is_protected(self):
        eva = self.a.create("Eva", PW)
        tok = self.a.start_session(eva["id"], True)
        self.a.update(eva["id"], {"disabled": True})
        self.assertIsNone(self.a.session_user(tok))
        for change in ({"role": "user"}, {"disabled": True}):
            with self.assertRaises(ValueError):
                self.a.update(self.admin["id"], change)
        with self.assertRaises(ValueError):
            self.a.delete(self.admin["id"])
        self.a.update(eva["id"], {"disabled": False, "role": "admin"})
        self.a.update(self.admin["id"], {"role": "user"})                 # now there is another admin

    def test_end_other_sessions(self):
        keep = self.a.start_session(self.admin["id"], True)
        other = self.a.start_session(self.admin["id"], True)
        me = self.a.session_user(keep)
        self.assertEqual(self.a.end_sessions(self.admin["id"], keep=me["session"]), 1)
        self.assertIsNotNone(self.a.session_user(keep))
        self.assertIsNone(self.a.session_user(other))


class AuthLogTest(unittest.TestCase):
    def test_line_is_one_line_and_quoted(self):
        line = authlog.line("login_failed", ts=0, user='max" event=login_ok\nfake', ip="203.0.113.7", reason="bad_password")
        self.assertEqual(line, 'time=1970-01-01T00:00:00Z app=hoerwerk event=login_failed '
                               'user="max\\" event=login_ok?fake" ip=203.0.113.7 reason=bad_password')
        self.assertNotIn("\n", line)


class WebAuthTest(unittest.TestCase):
    """Through the real server, behind a "proxy" on 127.0.0.1 (TRUSTED_PROXIES)."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="auth-")
        env = {k: os.path.join(cls.tmp, v) for k, v in
               {"CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
        for d in env.values():
            os.makedirs(d, exist_ok=True)
        cls.app = services.App(config.Config.from_env({**env, "AUDIOBOOK_WATCH": "false",
                                                       "TRUSTED_PROXIES": "127.0.0.1/32"}), start_workers=False)
        cls.server = server.make_server(cls.app, "127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.lines = []
        cls._event = authlog.event
        authlog.event = lambda name, **f: cls.lines.append((name, f)) or cls._event(name, **f)

    @classmethod
    def tearDownClass(cls):
        authlog.event = cls._event
        cls.server.shutdown()
        cls.server.server_close()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def req(self, method, path, body=None, cookie="", ip="", proto=""):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=20)
        headers = {"Host": f"127.0.0.1:{self.port}", "X-Requested-With": "Hoerwerk", "Content-Type": "application/json"}
        if cookie:
            headers["Cookie"] = cookie
        if ip:
            headers["X-Forwarded-For"] = ip
        if proto:
            headers["X-Forwarded-Proto"] = proto
        c.request(method, path, body=json.dumps(body).encode() if body is not None else None, headers=headers)
        r = c.getresponse()
        out = (r.status, dict(r.getheaders()), r.read())
        c.close()
        return out

    def login(self, name, pw, ip="198.51.100.1", proto=""):
        status, headers, body = self.req("POST", "/api/auth/login", {"name": name, "password": pw, "remember": True},
                                         ip=ip, proto=proto)
        cookie = headers.get("Set-Cookie", "")
        return status, cookie, cookie.split(";")[0]

    def test_1_setup_then_login_rights_and_log(self):
        status, _, body = self.req("GET", "/api/auth/status", ip="93.184.216.34")
        st = json.loads(body)
        self.assertTrue(st["setup"] and st["setup_code"])                    # from outside: code needed
        status, _, _ = self.req("POST", "/api/auth/setup", {"name": "Admin", "password": PW, "code": "nope"},
                                ip="93.184.216.34")
        self.assertEqual(status, 403)
        # through a proxy – even if it claims a home address – the code is needed (proxy not yet trusted …)
        self.assertEqual(self.req("POST", "/api/auth/setup", {"name": "Admin", "password": PW},
                                  ip="192.168.1.10")[0], 403)
        self.assertTrue(self.app.setup_code)
        status, headers, _ = self.req("POST", "/api/auth/setup", {"name": "Admin", "password": PW,
                                                                   "code": self.app.setup_code.lower()},
                                      ip="192.168.1.10")
        self.assertEqual(status, 200)
        self.assertIn("HttpOnly", headers["Set-Cookie"])
        self.assertEqual(self.req("POST", "/api/auth/setup", {"name": "X", "password": PW})[0], 409)

        status, raw, admin = self.login("admin", PW, proto="https")
        self.assertEqual(status, 200)
        for part in ("HttpOnly", "SameSite=Lax", "Secure", "Max-Age="):
            self.assertIn(part, raw)
        self.assertEqual(self.req("GET", "/api/sources", cookie=admin)[0], 200)

        status, _, body = self.req("POST", "/api/users", {"name": "Eva", "password": "evas-passwort-1"}, cookie=admin)
        self.assertEqual(status, 200)
        eva_id = json.loads(body)["id"]
        _, raw, eva = self.login("eva", "evas-passwort-1")
        self.assertNotIn("Secure", raw)                                      # plain http: no Secure flag
        self.assertEqual(self.req("GET", "/api/discover?section=critics", cookie=eva)[0], 200)
        for method, path in (("GET", "/api/sources"), ("GET", "/api/users"), ("PUT", "/api/settings"),
                             ("GET", "/api/logs"), ("PUT", f"/api/users/{eva_id}")):
            self.assertEqual(self.req(method, path, {"role": "admin"}, cookie=eva)[0], 403, path)
        self.assertEqual(self.app.accounts.get(eva_id)["role"], "user")      # no promotion through the back door
        self.assertIn(("forbidden", {"user": "Eva", "ip": "127.0.0.1", "path": "/api/users"}), self.lines)

        status, _, body = self.req("GET", "/me.js", cookie=eva)
        self.assertIn(b'"role": "user"', body)

        self.lines.clear()
        self.assertEqual(self.login("eva", "falsches-passwort", ip="203.0.113.50")[0], 401)
        self.assertEqual(self.lines[-1], ("login_failed", {"user": "eva", "ip": "203.0.113.50", "reason": "bad_password"}))

        self.assertEqual(self.req("POST", "/api/auth/logout", cookie=eva)[0], 200)
        self.assertEqual(self.req("GET", "/api/auth/me", cookie=eva)[0], 401)

    def test_0_home_network_rule(self):
        from web.router import Req
        from web.routes import auth as routes
        self.assertTrue(routes._home(Req(None, None, {}, ip="192.168.1.10")))
        self.assertFalse(routes._home(Req(None, None, {}, ip="192.168.1.10", proxied=True)))
        self.assertFalse(routes._home(Req(None, None, {}, ip="93.184.216.34")))

    def test_2_client_ip(self):
        h = server.Handler.__new__(server.Handler)
        h.server = self.server

        def ip(peer, xff="", real=""):
            h.client_address = (peer, 1)
            h.headers = {k: v for k, v in (("X-Forwarded-For", xff), ("X-Real-IP", real)) if v}
            return h.client_ip()
        self.assertEqual(ip("8.8.8.8", "1.2.3.4"), "8.8.8.8")                # untrusted peer: header ignored
        self.assertEqual(ip("127.0.0.1", "1.2.3.4"), "1.2.3.4")
        self.assertEqual(ip("127.0.0.1", "6.6.6.6, 1.2.3.4"), "1.2.3.4")     # forged left part ignored
        self.assertEqual(ip("127.0.0.1", "1.2.3.4, 127.0.0.1"), "1.2.3.4")   # chain of trusted proxies
        self.assertEqual(ip("::ffff:127.0.0.1", "2001:db8::1"), "2001:db8::1")
        self.assertEqual(ip("127.0.0.1", "", "5.6.7.8"), "5.6.7.8")
        self.assertEqual(ip("127.0.0.1", "garbage"), "127.0.0.1")              # unreadable header: the peer

    def test_4_fixation_and_login_csrf(self):
        """A cookie set by an attacker is never taken over; a login from a foreign page is refused."""
        planted = "hw_session=attacker-chosen-token"
        status, headers, _ = self.req("POST", "/api/auth/login", {"name": "admin", "password": PW}, cookie=planted,
                                      ip="198.51.100.9")
        if status == 200:
            self.assertNotIn("attacker-chosen-token", headers["Set-Cookie"])
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=20)
        c.request("POST", "/api/auth/login", body=json.dumps({"name": "admin", "password": PW}).encode(),
                  headers={"Host": f"127.0.0.1:{self.port}", "Content-Type": "text/plain", "Origin": "http://evil.example"})
        self.assertEqual(c.getresponse().status, 403)
        c.close()

    def test_3_brake_answers_429(self):
        for _ in range(acc.IP_FAILS):
            self.login("admin", "falsch-falsch", ip="203.0.113.77")
        self.lines.clear()
        self.assertEqual(self.login("admin", PW, ip="203.0.113.77")[0], 429)
        self.assertEqual(self.lines[-1][0], "login_blocked")


if __name__ == "__main__":
    unittest.main()
