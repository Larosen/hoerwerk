"""ntfy notifications: settings, selectable events, sending, SABnzbd download watcher."""
import http.server
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import db as dbm  # noqa: E402
import notify  # noqa: E402


class FakeNtfy(http.server.BaseHTTPRequestHandler):
    got = []

    def log_message(self, *a):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        FakeNtfy.got.append((self.path, self.headers.get("Authorization"), json.loads(body)))
        if self.path.startswith("/moved"):
            self.send_response(307)
            self.send_header("Location", "http://evil.example/")
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"{}")


class NotifierTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeNtfy)
        cls.url = f"http://127.0.0.1:{cls.srv.server_address[1]}"
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()

    def setUp(self):
        FakeNtfy.got.clear()
        self.dir = tempfile.mkdtemp()
        self.db = dbm.Database(os.path.join(self.dir, "t.db"))
        self.n = notify.Notifier(self.db)

    def tearDown(self):
        self.db.close()
        shutil.rmtree(self.dir, ignore_errors=True)

    def wait_for(self, count):
        end = time.time() + 5
        while len(FakeNtfy.got) < count and time.time() < end:
            time.sleep(0.02)

    def test_defaults_and_secrets(self):
        self.assertFalse(self.n.configured())
        events = self.n.events()
        self.assertTrue(events["failed"] and events["filed"] and events["download_failed"])
        self.assertFalse(events["download_sent"] or events["download_done"])
        pub = self.n.update_config({"url": self.url, "topic": "hoerbuecher", "token": "tk_SECRET"})
        self.assertTrue(pub["token_set"])
        self.assertNotIn("tk_SECRET", json.dumps(pub))
        # saving again with an empty field keeps the token …
        self.n.update_config({"url": self.url, "topic": "hoerbuecher", "token": ""})
        self.assertTrue(self.n.public_config()["token_set"])
        # … but never for another server
        self.n.update_config({"url": "https://ntfy.example.org", "topic": "hoerbuecher", "token": ""})
        self.assertFalse(self.n.public_config()["token_set"])

    def test_validation_and_pasted_topic_url(self):
        with self.assertRaises(notify.NotifyError):
            self.n.update_config({"url": "https://ntfy.sh", "topic": "../evil"})
        with self.assertRaises(notify.NotifyError):
            self.n.update_config({"url": "javascript:alert(1)", "topic": "x"})
        with self.assertRaises(notify.NotifyError):   # credentials belong in the password field, not the URL
            self.n.update_config({"url": "https://me:secret@ntfy.example", "topic": "x"})
        pub = self.n.update_config({"url": "https://ntfy.sh/mein_topic", "topic": ""})
        self.assertEqual((pub["url"], pub["topic"]), ("https://ntfy.sh", "mein_topic"))
        pub = self.n.update_config({"url": "https://example.org/ntfy/abc", "topic": ""})
        self.assertEqual((pub["url"], pub["topic"]), ("https://example.org/ntfy", "abc"))

    def test_only_chosen_events_are_sent(self):
        self.n.update_config({"url": self.url, "topic": "t", "token": "tk_1", "app_url": "http://tower:8080",
                              "events": {"review": False}})
        self.n.send("review", "Prüfen", "x")          # switched off
        self.n.send("download_sent", "Geschickt", "x")  # off by default
        self.n.send("filed", "Hörbuch abgelegt", "Die Verwandlung – Franz Kafka")
        self.wait_for(1)
        time.sleep(0.1)
        self.assertEqual(len(FakeNtfy.got), 1)
        path, auth, body = FakeNtfy.got[0]
        self.assertEqual((path, auth), ("/", "Bearer tk_1"))
        self.assertEqual(body["topic"], "t")
        self.assertEqual(body["title"], "Hörbuch abgelegt")          # UTF-8 in the JSON body
        self.assertEqual(body["click"], "http://tower:8080")
        self.assertEqual(body["tags"], ["books"])

    def test_basic_auth_and_test_message(self):
        self.n.update_config({"url": self.url, "topic": "t", "user": "ich", "password": "pw"})
        self.assertIn("gesendet", self.n.test())
        self.assertTrue(FakeNtfy.got[0][1].startswith("Basic "))

    def test_redirect_is_not_followed(self):
        self.n.update_config({"url": self.url + "/moved", "topic": "t", "token": "tk_2"})
        with self.assertRaises(notify.NotifyError):
            self.n.test()
        self.assertEqual(len(FakeNtfy.got), 1)        # only our server saw the token

    def test_unreachable_server_does_not_raise_from_send(self):
        self.n.update_config({"url": "http://127.0.0.1:9", "topic": "t"})
        self.n.send("failed", "Fehler", "x")
        end = time.time() + 5
        while self.n.last_error is None and time.time() < end:
            time.sleep(0.02)
        self.assertIn("nicht erreichbar", self.n.last_error)


class FakeUsenet:
    def __init__(self, items):
        self.items = items

    def configured(self):
        return {"sab": True, "indexers": 1}

    def activity(self):
        return {"items": self.items}


class WatcherTest(unittest.TestCase):
    def test_tells_once_and_only_about_own_downloads(self):
        d = tempfile.mkdtemp()
        try:
            db = dbm.Database(os.path.join(d, "t.db"))
            n = notify.Notifier(db)
            sent = []
            n.send = lambda event, title, message, priority=None: sent.append((event, message))
            n.update_config({"url": "https://ntfy.example.org", "topic": "t",
                             "events": {"download_done": True, "download_failed": True}})
            with db.tx() as c:
                dbm.save_grab(c, "Kafka - Die Verwandlung", {"nzo_id": "SABnzbd_nzo_1", "time": time.time()})
                dbm.save_grab(c, "Grimm - Maerchen", {"nzo_id": "SABnzbd_nzo_2", "time": time.time()})
            items = [{"id": "SABnzbd_nzo_1", "name": "Kafka - Die Verwandlung", "done": True, "status": "Completed"},
                     {"id": "SABnzbd_nzo_2", "name": "Grimm - Maerchen", "done": True, "status": "Failed",
                      "error": "Nicht genug Blöcke"},
                     {"id": "SABnzbd_nzo_9", "name": "Fremder Download", "done": True, "status": "Completed"},
                     {"id": "SABnzbd_nzo_3", "name": "läuft noch", "done": False, "status": "Downloading"}]
            w = notify.DownloadWatcher(FakeUsenet(items), n, db)
            self.assertEqual(sorted(w.check_once()), ["SABnzbd_nzo_1", "SABnzbd_nzo_2"])
            self.assertEqual(sorted(e for e, _ in sent), ["download_done", "download_failed"])
            self.assertEqual(w.check_once(), [])      # not twice
            db.close()
        finally:
            shutil.rmtree(d, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
