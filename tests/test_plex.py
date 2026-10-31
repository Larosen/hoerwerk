"""Plex: pick the audiobook library, refresh it after filing, empty the trash, keep the token safe."""
import http.server
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import db as dbm  # noqa: E402
import plex  # noqa: E402


class FakePlex(http.server.BaseHTTPRequestHandler):
    calls = []
    refreshing = []          # answers for "refreshing" while a scan runs

    def log_message(self, *args):
        pass

    def _answer(self, status, data=None):
        body = json.dumps(data or {}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _handle(self):
        FakePlex.calls.append((self.command, self.path, self.headers.get("X-Plex-Token")))
        if self.headers.get("X-Plex-Token") != "plex-secret":
            return self._answer(401)
        if self.path == "/library/sections":
            busy = FakePlex.refreshing.pop(0) if FakePlex.refreshing else False
            return self._answer(200, {"MediaContainer": {"Directory": [
                {"key": "1", "title": "Filme", "type": "movie", "Location": [{"path": "/movies"}]},
                {"key": "2", "title": "Musik", "type": "artist", "Location": [{"path": "/music"}]},
                {"key": "3", "title": "Hörbücher", "type": "artist", "refreshing": busy,
                 "Location": [{"path": "/audiobooks"}]}]}})
        if self.path == "/moved":
            self.send_response(302)
            self.send_header("Location", "http://evil.example/")
            self.end_headers()
            return None
        return self._answer(200)

    do_GET = do_PUT = _handle


class PlexTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakePlex)
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()
        cls.url = f"http://127.0.0.1:{cls.srv.server_address[1]}"

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def setUp(self):
        FakePlex.calls = []
        FakePlex.refreshing = []
        self.dir = tempfile.mkdtemp(prefix="plex-")
        self.db = dbm.Database(os.path.join(self.dir, "t.db"))
        self.p = plex.Plex(self.db, delay=0.05, poll=0.05)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_picks_the_audiobook_library_and_refreshes_once(self):
        pub = self.p.update_config({"url": self.url + "/web/index.html", "token": "plex-secret"})
        self.assertEqual(pub["url"], self.url)                 # pasted the Plex Web address: the server is kept
        self.assertNotIn("token", pub)
        self.assertIn("Hörbücher", self.p.test())              # two music libraries, the name decides
        self.assertTrue(self.p.configured())
        for _ in range(3):                                     # three books in a row …
            self.p.request_scan()
        self.p._timer.join(2)
        refreshes = [c for c in FakePlex.calls if c[1].endswith("/refresh")]
        self.assertEqual(refreshes, [("GET", "/library/sections/3/refresh", "plex-secret")])   # … one scan

    def test_empties_the_trash_after_the_scan(self):
        self.p.update_config({"url": self.url, "token": "plex-secret", "section": "3", "empty_trash": True})
        FakePlex.refreshing = [True, True, False]              # Plex is busy for a moment
        self.p.scan_now()
        self.assertEqual([c[:2] for c in FakePlex.calls if c[0] == "PUT"], [("PUT", "/library/sections/3/emptyTrash")])
        self.assertIn("Papierkorb", self.p.last)

    def test_token_stays_with_its_server(self):
        self.p.update_config({"url": self.url, "token": "plex-secret", "section": "3"})
        pub = self.p.update_config({"url": self.url, "section": "3"})          # empty field: keep it
        self.assertTrue(pub["token_set"])
        pub = self.p.update_config({"url": "http://other.example:32400", "section": "3"})
        self.assertFalse(pub["token_set"])                                      # never to another server
        with self.assertRaises(plex.PlexError):
            self.p.update_config({"url": "http://me:pw@plex.example"})
        with self.assertRaises(plex.PlexError):
            self.p.update_config({"url": "javascript:alert(1)"})

    def test_wrong_token_and_no_redirects(self):
        errors = []
        self.p = plex.Plex(self.db, on_error=errors.append)
        self.p.update_config({"url": self.url, "token": "wrong", "section": "3"})
        self.p.scan_now()
        self.assertEqual(len(errors), 1)
        self.assertIn("Token", errors[0])
        with self.assertRaises(plex.PlexError):
            plex._call("GET", self.url + "/moved", "plex-secret")
        self.assertFalse(any("evil" in str(c) for c in FakePlex.calls))


if __name__ == "__main__":
    unittest.main()
