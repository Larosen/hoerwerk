"""Upload into the inbox: safe paths, staging, moving in one step, rights before the body is read."""
import http.client
import io
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import config  # noqa: E402
import services  # noqa: E402
import uploads  # noqa: E402
from web import server  # noqa: E402

PW = "korrekt-pferd-batterie"


class UploadsTest(unittest.TestCase):
    def setUp(self):
        self.inbox = tempfile.mkdtemp()
        self.arrived = []
        self.u = uploads.Uploads(self.inbox, on_done=self.arrived.extend)

    def put(self, uid, path, data=b"ID3audio"):
        return self.u.put(uid, path, io.BytesIO(data), len(data))

    def test_paths(self):
        self.assertEqual(uploads.clean_path("Storm - Schimmelreiter\\CD1\\01.mp3"), ["Storm - Schimmelreiter", "CD1", "01.mp3"])
        for bad in ("../x.mp3", "A/../../x.mp3", "A/.hidden/x.mp3", "A/_fertig/x.mp3", "A/x.exe", "A/notiz.txt",
                    "A/x.mp3.sh", "", "A/b:c.mp3", "A/ x.mp3"):
            with self.assertRaises(uploads.UploadError, msg=bad):
                uploads.clean_path(bad)
        self.assertEqual(uploads.clean_path("A/chapters.txt")[-1], "chapters.txt")

    def test_staging_then_one_move(self):
        uid = self.u.start("admin")
        self.put(uid, "Storm - Schimmelreiter/CD1/01.mp3")
        self.put(uid, "Storm - Schimmelreiter/CD2/01.mp3")
        self.put(uid, "Storm - Schimmelreiter/cover.jpg", b"\xff\xd8\xff")
        self.assertEqual(sorted(os.listdir(self.inbox)), [f".upload-{uid}"])          # hidden while uploading
        os.makedirs(os.path.join(self.inbox, "Storm - Schimmelreiter"))                  # same name already there
        self.assertEqual(self.u.finish(uid), ["Storm - Schimmelreiter (2)"])
        self.assertEqual(sorted(os.listdir(self.inbox)), ["Storm - Schimmelreiter", "Storm - Schimmelreiter (2)"])
        self.assertEqual(sorted(os.listdir(os.path.join(self.inbox, "Storm - Schimmelreiter (2)"))), ["CD1", "CD2", "cover.jpg"])
        self.assertEqual(self.arrived, ["Storm - Schimmelreiter (2)"])
        with self.assertRaises(uploads.UploadError):
            self.u.finish(uid)                                                           # gone

    def test_broken_upload_leaves_nothing(self):
        uid = self.u.start("admin")
        with self.assertRaises(uploads.UploadError):
            self.u.put(uid, "A/01.mp3", io.BytesIO(b"abc"), 10)                       # connection dropped
        self.assertFalse(os.path.exists(os.path.join(self.inbox, f".upload-{uid}", "A", "01.mp3")))
        self.assertFalse(os.path.exists(os.path.join(self.inbox, f".upload-{uid}", "A", "01.mp3.part")))
        self.u.cancel(uid)
        self.assertEqual(os.listdir(self.inbox), [])
        with self.assertRaises(uploads.UploadError):
            self.put("0123456789abcdef", "A/01.mp3")                                     # unknown session
        with self.assertRaises(uploads.UploadError):
            self.put("../../etc", "A/01.mp3")

    def test_stale_staging_is_removed(self):
        old = os.path.join(self.inbox, ".upload-0000000000000000")
        os.makedirs(old)
        os.utime(old, (1, 1))
        self.u.start("admin")
        self.assertFalse(os.path.exists(old))


class WebUploadTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="up-")
        env = {k: os.path.join(cls.tmp, v) for k, v in
               {"CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
        for d in env.values():
            os.makedirs(d, exist_ok=True)
        cls.inbox = env["INBOX_DIR"]
        cls.app = services.App(config.Config.from_env({**env, "AUDIOBOOK_WATCH": "false"}), start_workers=False)
        cls.server = server.make_server(cls.app, "127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        a = cls.app.accounts
        cls.admin = "hw_session=" + a.start_session(a.create("Admin", PW, "admin")["id"], False)
        cls.eva = "hw_session=" + a.start_session(a.create("Eva", PW)["id"], False)

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def req(self, method, path, body=b"", cookie="", headers=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=20)
        h = {"Host": f"127.0.0.1:{self.port}", "X-Requested-With": "Hoerwerk", **(headers or {})}
        if cookie:
            h["Cookie"] = cookie
        c.request(method, path, body=body, headers=h)
        r = c.getresponse()
        out = (r.status, json.loads(r.read() or b"{}"))
        c.close()
        return out

    def test_flow_and_rights(self):
        status, res = self.req("POST", "/api/upload", cookie=self.admin)
        self.assertEqual(status, 200)
        uid = res["id"]
        data = b"ID3" + b"x" * 5000
        status, res = self.req("PUT", f"/api/upload/{uid}?path=Max%20Muster%20-%20Buch/CD1/01.mp3", data, self.admin,
                               {"Content-Type": "application/octet-stream"})
        self.assertEqual((status, res["files"]), (200, 1))
        # without login / as family account / with a bad path: refused (and the body is never read)
        self.assertEqual(self.req("PUT", f"/api/upload/{uid}?path=A/x.mp3", data)[0], 401)
        self.assertEqual(self.req("PUT", f"/api/upload/{uid}?path=A/x.mp3", data, self.eva)[0], 403)
        self.assertEqual(self.req("PUT", f"/api/upload/{uid}?path=../../x.mp3", data, self.admin)[0], 400)
        self.assertEqual(self.req("PUT", f"/api/upload/{uid}?path=A/run.sh", data, self.admin)[0], 400)
        status, res = self.req("POST", f"/api/upload/{uid}/finish", cookie=self.admin)
        self.assertEqual((status, res["names"]), (200, ["Max Muster - Buch"]))
        with open(os.path.join(self.inbox, "Max Muster - Buch", "CD1", "01.mp3"), "rb") as fh:
            self.assertEqual(fh.read(), data)
        self.assertIn("Max Muster - Buch", self.app.pipeline.complete)                  # no waiting in the inbox

    def test_huge_announcement_is_refused_before_reading(self):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.putrequest("PUT", "/api/upload/0123456789abcdef?path=A/x.mp3")
        for k, v in {"Host": f"127.0.0.1:{self.port}", "X-Requested-With": "Hoerwerk", "Cookie": self.admin,
                     "Content-Length": str(uploads.MAX_FILE + 1)}.items():
            c.putheader(k, v)
        c.endheaders()
        self.assertEqual(c.getresponse().status, 413)
        c.close()


if __name__ == "__main__":
    unittest.main()
