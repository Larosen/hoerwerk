"""Security regression tests: CSRF, headers, untrusted downloads, network fetches."""
import dataclasses
import http.client
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import db as dbm  # noqa: E402
import audible  # noqa: E402
import config  # noqa: E402
import services  # noqa: E402
from web import server  # noqa: E402
import encoder  # noqa: E402
import pipeline  # noqa: E402
import tagger  # noqa: E402
import usenet  # noqa: E402
from media import tone  # noqa: E402

HAVE_FFMPEG = bool(shutil.which("ffmpeg"))


class WebTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="sec-")
        for d in ("config", "inbox", "lib"):
            os.makedirs(os.path.join(cls.tmp, d))
        env = {k: os.path.join(cls.tmp, v) for k, v in
               {"CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
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

    def req(self, method, path, body=None, headers=None, cookie=True):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        data = json.dumps(body).encode() if body is not None else None
        auth = {"Cookie": self.cookie} if cookie is True else {"Cookie": cookie} if cookie else {}
        c.request(method, path, body=data, headers={"Host": f"127.0.0.1:{self.port}", **auth, **(headers or {})})
        r = c.getresponse()
        out = (r.status, dict(r.getheaders()), r.read())
        c.close()
        return out

    OWN = {"X-Requested-With": "Hoerwerk", "Content-Type": "application/json"}

    def test_csrf_from_foreign_page_is_refused(self):
        change = {"language": "en"}
        # what a foreign web page can send without a CORS preflight
        status, _, _ = self.req("PUT", "/api/settings", change, {"Content-Type": "text/plain",
                                                                "Origin": "http://evil.example"})
        self.assertEqual(status, 403)
        status, _, _ = self.req("PUT", "/api/settings", change, {**self.OWN, "Origin": "http://evil.example"})
        self.assertEqual(status, 403)
        status, _, _ = self.req("PUT", "/api/settings", change, {**self.OWN, "Sec-Fetch-Site": "cross-site"})
        self.assertEqual(status, 403)
        self.assertEqual(self.app.settings()["language"], "de")
        # our own page works
        status, _, _ = self.req("POST", "/api/audiobooks/scan", None,
                                {"X-Requested-With": "Hoerwerk", "Origin": f"http://127.0.0.1:{self.port}",
                                 "Sec-Fetch-Site": "same-origin"})
        self.assertEqual(status, 200)

    def test_get_needs_no_token_but_has_security_headers(self):
        status, headers, _ = self.req("GET", "/")
        self.assertEqual(status, 200)
        self.assertIn("frame-ancestors 'none'", headers["Content-Security-Policy"])
        self.assertEqual(headers["X-Content-Type-Options"], "nosniff")
        self.assertEqual(headers["X-Frame-Options"], "DENY")
        self.assertEqual(headers["Referrer-Policy"], "no-referrer")

    def test_body_limit(self):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.putrequest("POST", "/api/search")
        for k, v in {**self.OWN, "Cookie": self.cookie, "Content-Length": str(server.MAX_BODY + 1)}.items():
            c.putheader(k, v)
        c.endheaders()
        self.assertEqual(c.getresponse().status, 413)
        c.close()

    def test_static_traversal(self):
        for p in ("/static/../main.py", "/static/%2e%2e/main.py", "/static/..%2fmain.py"):
            self.assertEqual(self.req("GET", p)[0], 404, p)

    def test_library_cover(self):
        book = os.path.join(self.tmp, "lib", "Theodor Storm", "Immensee")
        os.makedirs(book, exist_ok=True)
        jpeg = b"\xff\xd8\xff\xe0" + b"0" * 64
        with open(os.path.join(book, "cover.jpg"), "wb") as fh:
            fh.write(jpeg)
        status, headers, body = self.req("GET", "/api/library/cover?path=Theodor%20Storm/Immensee")
        self.assertEqual((status, headers["Content-Type"], body), (200, "image/jpeg", jpeg))
        self.assertIn("max-age", headers["Cache-Control"])
        # a "cover" that is really HTML is not served as an image
        with open(os.path.join(book, "cover.jpg"), "wb") as fh:
            fh.write(b"<script>alert(1)</script>")
        self.assertEqual(self.req("GET", "/api/library/cover?path=Theodor%20Storm/Immensee")[0], 404)
        self.assertEqual(self.req("GET", "/api/library/cover?path=../config")[0], 404)

    def test_library_paths_stay_inside(self):
        for p in ("../config", "..", "/etc", "x/../../config"):
            status, _, body = self.req("GET", "/api/library/tracks?path=" + p)
            self.assertEqual(status, 404, (p, body))

    def test_connection_reuse_like_a_proxy(self):
        """Traefik (Pangolin) keeps connections open: either they work again, or the answer says close."""
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            for path in ("/login", "/static/icon.png", "/i18n.js", "/", "/api/health", "/api/settings"):
                c.request("GET", path, headers={"Host": f"127.0.0.1:{self.port}", "Cookie": self.cookie})
                r = c.getresponse()
                r.read()
                self.assertIn(r.status, (200, 303), path)
                self.assertNotEqual(r.getheader("Connection"), "close", path)
            # a refused request with a body: closed, and the answer says so
            c.request("POST", "/api/settings", body=b"{}", headers={"Host": f"127.0.0.1:{self.port}", "Content-Type": "application/json"})
            r = c.getresponse()
            r.read()
            self.assertEqual((r.status, r.getheader("Connection")), (403, "close"))
        finally:
            c.close()

    def test_internal_error_tells_nothing_but_a_number(self):
        import applog
        from web import router

        @router.route("GET", "/api/test-boom", access="user")
        def boom(req):
            raise RuntimeError("geheim /config/hoerwerk.db SELECT * FROM accounts apikey=abc123")
        try:
            status, _, body = self.req("GET", "/api/test-boom")
        finally:
            router.ROUTES[:] = [r for r in router.ROUTES if r[2] is not boom]
        self.assertEqual(status, 500)
        answer = json.loads(body)
        for secret in ("geheim", "/config", "SELECT", "abc123", "RuntimeError", "Traceback"):
            self.assertNotIn(secret, body.decode())
        rid = answer["request_id"]
        self.assertRegex(rid, r"^[0-9a-f]{8}$")
        self.assertIn(rid, answer["error"])
        logged = [e["text"] for e in applog.log.since(0, "error") if rid in e["text"]]
        self.assertEqual(len(logged), 1)
        self.assertIn("apikey=abc123", logged[0])                         # the full error stays in the log
        self.assertIn("Traceback", logged[0])
        status, _, body = self.req("GET", "/api/nothing-here")            # regular HTTP errors unchanged
        self.assertEqual((status, json.loads(body)["error"]), (404, "Nicht gefunden"))

    def test_allowed_hosts(self):
        old = self.app.config
        self.app.config = dataclasses.replace(old, allowed_hosts=frozenset({"tower"}))
        try:
            self.assertEqual(self.req("GET", "/api/settings", headers={"Host": "evil.example"})[0], 421)
            self.assertEqual(self.req("GET", "/", headers={"Host": "hoerwerk"})[0], 421)
            self.assertEqual(self.req("GET", "/api/settings", headers={"Host": "tower:8080"})[0], 200)
            self.assertEqual(self.req("GET", "/api/settings", headers={"Host": "127.0.0.1:8080"})[0], 200)
            # health checks of Docker (127.0.0.1) and Pangolin/Newt (target name) always get through
            self.assertEqual(self.req("GET", "/api/health", headers={"Host": "hoerwerk:8080"})[0], 200)
        finally:
            self.app.config = old

    def test_without_login_only_the_login_page(self):
        """Without a session: logo, UI code and login page – no data, no app page."""
        self.assertEqual(self.req("GET", "/static/icon.png", cookie=False)[0], 200)
        self.assertEqual(self.req("GET", "/api/health", cookie=False)[0], 200)
        status, headers, _ = self.req("GET", "/", cookie=False)
        self.assertEqual((status, headers["Location"]), (303, "/login"))
        self.assertEqual(self.req("GET", "/login", cookie=False)[0], 200)
        self.assertEqual(self.req("GET", "/me.js", cookie=False)[2], b"const ME = null;\n")
        for path in ("/api/settings", "/api/library/books", "/api/sources", "/api/users", "/api/events?topics=library"):
            status, _, body = self.req("GET", path, cookie=False)
            self.assertEqual(status, 401, path)
            self.assertTrue(json.loads(body)["login"])
        self.assertEqual(self.req("GET", "/api/settings", cookie="hw_session=forged-token")[0], 401)
        self.assertEqual(self.req("PUT", "/api/settings", {"language": "en"}, self.OWN, cookie=False)[0], 401)


class AccessTest(WebTest):
    """Nothing without login: every API route, every app page, the UI code."""
    PUBLIC_API = {("GET", "/api/health"), ("GET", "/api/auth/status"), ("POST", "/api/auth/setup"),
                  ("POST", "/api/auth/login"), ("POST", "/api/auth/login/totp"),
                  ("GET", "/api/invite/1"), ("POST", "/api/invite/1")}       # the invitation link (token checked)

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        eva = cls.app.accounts.create("eva", "Test-Kaffee-456", "user")
        cls.user_cookie = "hw_session=" + cls.app.accounts.start_session(eva["id"], False)

    def _routes(self):
        import re
        from web import router
        for method, rx, fn, access in router.ROUTES:
            path = re.sub(r"\(\?P<\w+>[^)]*\)", "1", rx.pattern).lstrip("^").rstrip("$").replace("\\", "")
            yield method, path, access

    def test_every_api_route_needs_login(self):
        """The registry of all routes: only the login itself and the health check are open."""
        public = set()
        for method, path, access in self._routes():
            if access == "public":
                public.add((method, path))
                continue
            status, _, body = self.req(method, path, {} if method != "GET" else None, self.OWN, cookie=False)
            self.assertEqual(status, 401, f"{method} {path}")
        self.assertEqual(public, self.PUBLIC_API)

    def test_admin_routes_refuse_family_accounts(self):
        for method, path, access in self._routes():
            if access != "admin" or path == "/api/events":
                continue
            status, _, _ = self.req(method, path, {} if method != "GET" else None, self.OWN, cookie=self.user_cookie)
            self.assertEqual(status, 403, f"{method} {path}")

    def test_app_pages(self):
        status, headers, _ = self.req("GET", "/settings/health", cookie=False)
        self.assertEqual((status, headers["Location"]), (303, "/login?next=/settings/health"))
        for path in ("/settings", "/settings/users", "/books", "/search", "/account"):
            self.assertEqual(self.req("GET", path, cookie=False)[0], 303, path)
            status, headers, body = self.req("GET", path)
            self.assertEqual(status, 200, path)
            self.assertIn(b'id="viewSettings"', body)
        for path in ("/settings/nope", "/settings/health/x", "/search/x", "/admin", "/settings/../config"):
            self.assertEqual(self.req("GET", path)[0], 404, path)
        # family accounts: no settings page, no inbox – back to the start
        for path in ("/settings", "/settings/health", "/books"):
            status, headers, _ = self.req("GET", path, cookie=self.user_cookie)
            self.assertEqual((status, headers.get("Location")), (303, "/"), path)
        self.assertEqual(self.req("GET", "/account", cookie=self.user_cookie)[0], 200)

    def test_ui_code_only_after_login(self):
        status, headers, _ = self.req("GET", "/favicon.ico", cookie=False)
        self.assertEqual((status, headers["Content-Type"]), (200, "image/png"))
        for name in ("style.css", "icon.svg"):
            self.assertEqual(self.req("GET", "/static/" + name, cookie=False)[0], 200, name)
        for name in ("theme.js", "core.js", "login.js"):        # built from TypeScript: 404 until built, never 401
            self.assertIn(self.req("GET", "/static/" + name, cookie=False)[0], (200, 404), name)
        for name in ("index.html", "app.js", "settings.js", "router.js", "i18n/en.json", "../web/server.py", "%2e%2e/main.py"):
            self.assertEqual(self.req("GET", "/static/" + name, cookie=False)[0], 401, name)
        self.assertIn(self.req("GET", "/static/settings.js")[0], (200, 404))     # with login: served when built
        self.assertEqual(self.req("GET", "/static/../web/server.py")[0], 404)
        self.assertEqual(self.req("GET", "/static/%2e%2e/main.py")[0], 404)

    def test_health_tells_the_version_only_after_login(self):
        self.assertEqual(json.loads(self.req("GET", "/api/health", cookie=False)[2]), {"ok": True})
        self.assertEqual(json.loads(self.req("GET", "/api/health")[2])["version"], config.VERSION)

    def test_diagnostics_and_proxy(self):
        """The health check sees the proxy: trusted -> real address, not trusted -> error with the fix."""
        xff = {"X-Forwarded-For": "203.0.113.7", "X-Forwarded-Proto": "https"}
        checks = {c["id"]: c for c in json.loads(self.req("GET", "/api/diagnostics", headers=xff)[2])["checks"]}
        self.assertEqual(checks["proxy"]["status"], "error")
        self.assertEqual(checks["proxy"]["hint"], "TRUSTED_PROXIES=127.0.0.1")
        old = self.app.config
        self.app.config = dataclasses.replace(old, trusted_proxies=config.parse_networks("127.0.0.1"))
        try:
            res = json.loads(self.req("GET", "/api/diagnostics", headers=xff)[2])
            checks = {c["id"]: c for c in res["checks"]}
            self.assertEqual(checks["proxy"]["status"], "ok")
            self.assertEqual(checks["https"]["status"], "ok")
            self.assertIn("203.0.113.7", checks["ip"]["title"])
            me = json.loads(self.req("GET", "/api/auth/me", headers=xff)[2])["connection"]
            self.assertEqual((me["ip"], me["trusted"], me["https"]), ("203.0.113.7", True, True))
        finally:
            self.app.config = old
        self.assertEqual(self.req("GET", "/api/diagnostics", cookie=self.user_cookie)[0], 403)

    def test_untrusted_proxy_in_own_network_is_noted(self):
        self.app.untrusted_proxies.clear()             # e.g. Newt with a new IP after a restart
        self.server.note_untrusted("10.30.0.9")
        self.assertIn("10.30.0.9", self.app.untrusted_proxies)
        checks = {c["id"]: c for c in json.loads(self.req("GET", "/api/diagnostics")[2])["checks"]}
        self.assertEqual(checks["untrusted-10.30.0.9"]["hint"], "TRUSTED_PROXIES=10.30.0.9")


class DownloadTest(unittest.TestCase):
    """What a malicious download could try."""

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="evil-")
        self.inbox = os.path.join(self.root, "inbox")
        self.secret = os.path.join(self.root, "sources.json")
        with open(self.secret, "w") as fh:
            fh.write('{"apikey": "TOPSECRET"}')
        os.makedirs(self.inbox)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_symlinks_are_ignored(self):
        book = os.path.join(self.inbox, "Buch")
        os.makedirs(book)
        os.symlink(self.secret, os.path.join(book, "cover.jpg"))       # would put the keys into the library
        os.symlink(self.secret, os.path.join(book, "01.mp3"))
        os.symlink(self.root, os.path.join(book, "CD1"))               # folder pointing outside
        os.symlink(self.root, os.path.join(self.inbox, "Link"))       # top-level link
        self.assertEqual(encoder.collect_audio(book), [])
        self.assertIsNone(encoder.find_cover(book))
        self.assertEqual(pipeline.book_units(os.path.join(self.inbox, "Link")), [])
        self.assertEqual(pipeline.signature(os.path.join(self.inbox, "Link")), (0, 0, 0.0))
        self.assertIn("cover.jpg (Verknüpfung)", pipeline.suspicious_files(book))

    def test_suspicious_files_are_reported(self):
        book = os.path.join(self.inbox, "Buch")
        os.makedirs(os.path.join(book, "extra"))
        for n in ("01.mp3", "Codec_Installer.exe", "extra/run.bat", "cover.jpg", "info.nfo"):
            open(os.path.join(book, n), "w").close()
        self.assertEqual(pipeline.suspicious_files(book), ["Codec_Installer.exe", "extra/run.bat"])

    @unittest.skipUnless(HAVE_FFMPEG, "ffmpeg fehlt")
    def test_playlist_disguised_as_mp3_is_not_followed(self):
        book = os.path.join(self.inbox, "Buch")
        os.makedirs(book)
        with open(os.path.join(book, "01.mp3"), "w") as fh:
            fh.write(f"#EXTM3U\n#EXTINF:1,\nfile://{self.secret}\n#EXT-X-ENDLIST\n")
        with open(os.path.join(book, "02.mp3"), "w") as fh:
            fh.write(f"ffconcat version 1.0\nfile {self.secret}\n")
        for f in ("01.mp3", "02.mp3"):
            with self.assertRaises(encoder.EncodeError):
                encoder.merge_to_m4b(book, [f], os.path.join(self.root, "out.m4b"))
        self.assertFalse(os.path.exists(os.path.join(self.root, "out.m4b")))

    @unittest.skipUnless(HAVE_FFMPEG, "ffmpeg fehlt")
    def test_real_audio_still_works_with_whitelist(self):
        book = os.path.join(self.inbox, "Buch")
        tone(os.path.join(book, "01.mp3"), 1.0)
        tone(os.path.join(book, "02.m4a"), 1.0)
        info = encoder.merge_to_m4b(book, ["01.mp3", "02.m4a"], os.path.join(self.root, "out.m4b"))
        self.assertEqual(len(info["chapters"]), 2)

    def test_yaml_bomb_and_size(self):
        with open(os.path.join(self.root, "metadata.yml"), "w") as fh:
            fh.write("a: &a [x, x, x]\nb: &b [*a, *a, *a]\nc: [*b, *b, *b]\ntitle: T\n")
        with self.assertRaises(ValueError):
            tagger.load_manual(self.root)
        with open(os.path.join(self.root, "metadata.yml"), "w") as fh:
            fh.write("title: T\ndescription: '" + "x" * 600_000 + "'\n")
        with self.assertRaises(ValueError):
            tagger.load_manual(self.root)

    def test_library_paths_cannot_escape(self):
        import library
        evil = {"title": "../../../etc/cron.d/x", "authors": ["..", "/root"], "series": "../..",
                "series_position": "1"}
        folder, name = library.destination(self.root, evil, library.DEFAULT_FOLDER, library.DEFAULT_FILE)
        full = os.path.realpath(os.path.join(folder, name))
        self.assertTrue(full.startswith(os.path.realpath(self.root) + os.sep), full)
        self.assertNotIn("..", os.path.relpath(full, self.root).split(os.sep))


class NetworkTest(unittest.TestCase):
    def test_public_url_check(self):
        for url in ("http://127.0.0.1/", "http://localhost:8080/", "http://192.168.178.1/", "http://10.1.2.3/x",
                    "http://169.254.169.254/latest/meta-data", "http://[::1]/", "file:///etc/passwd",
                    "ftp://example.com/x", "javascript:alert(1)", ""):
            self.assertFalse(audible.is_public_url(url), url)

    def test_http_get_only_http(self):
        for url in ("file:///etc/passwd", "ftp://x/y", "gopher://x"):
            with self.assertRaises(audible.AudibleError):
                audible.http_get(url, retries=1)

    def test_redirect_to_internal_is_blocked(self):
        class Redirect(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                self.send_response(302)
                self.send_header("Location", "http://10.0.0.1/admin")
                self.send_header("Content-Length", "0")
                self.end_headers()

        srv = ThreadingHTTPServer(("127.0.0.1", 0), Redirect)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            url = f"http://127.0.0.1:{srv.server_address[1]}/cover.jpg"
            with self.assertRaises(audible.AudibleError) as cm:
                audible.http_get(url, retries=1, allow=lambda u: u.startswith("http://127.0.0.1"))
            self.assertIn("abgelehnt", str(cm.exception))
        finally:
            srv.shutdown()

    def test_indexer_xml_with_entities_is_refused(self):
        bomb = (b'<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;">]>'
                b"<rss><channel><item><title>&lol2;</title></item></channel></rss>")
        with self.assertRaises(usenet.UsenetError):
            usenet.parse_results(bomb, "x")

    def test_indexer_links_must_be_http(self):
        xml = (b'<rss xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/"><channel>'
               b"<item><title>A</title><link>javascript:alert(1)</link></item>"
               b"<item><title>B</title><link>https://idx/b.nzb</link><comments>javascript:alert(2)</comments></item>"
               b"</channel></rss>")
        res = usenet.parse_results(xml, "x")
        self.assertEqual([r["title"] for r in res], ["B"])
        self.assertIsNone(res[0]["details"])

    def test_release_name_for_sab(self):
        self.assertEqual(usenet.safe_release_name("../../.work/evil"), "_.._.work_evil")
        self.assertEqual(usenet.safe_release_name(".hidden"), "hidden")
        self.assertEqual(usenet.safe_release_name("Autor - Titel: Teil 1"), "Autor - Titel_ Teil 1")

    def test_keys_do_not_follow_a_changed_url(self):
        cfg = tempfile.mkdtemp()
        try:
            db = dbm.Database(os.path.join(cfg, "t.db"))
            u = usenet.Usenet(db)
            u.update_config({"indexers": [{"name": "A", "url": "https://indexer.example", "apikey": "K1"}],
                             "sab": {"url": "http://192.168.1.5:8080", "apikey": "K2"}})
            pub = u.public_config()
            pub["indexers"][0]["url"] = "https://attacker.example"
            pub["sab"]["url"] = "http://attacker.example:8080"
            u.update_config(pub)
            saved = db.get("sources")
            self.assertEqual(saved["indexers"][0]["apikey"], "")
            self.assertEqual(saved["sab"]["apikey"], "")
            self.assertEqual(oct(os.stat(os.path.join(cfg, "t.db")).st_mode & 0o777), "0o600")
        finally:
            shutil.rmtree(cfg)


if __name__ == "__main__":
    unittest.main()
