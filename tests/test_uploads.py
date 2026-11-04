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

PW = "korrekt pferd batterie heftklammer"


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

    def test_limits_hold_for_parallel_uploads_of_one_account(self):
        saved = (uploads.MAX_SESSIONS_PER_USER, uploads.MAX_USER_BYTES, uploads.MAX_SESSION_BYTES, uploads.MAX_FILES)
        uploads.MAX_SESSIONS_PER_USER, uploads.MAX_USER_BYTES, uploads.MAX_SESSION_BYTES, uploads.MAX_FILES = 2, 100, 80, 50
        try:
            a, b = self.u.start("u1"), self.u.start("u1")
            with self.assertRaises(uploads.UploadError):
                self.u.start("u1")                                                   # a third one
            self.u.start("u2")                                                       # someone else may

            class Slow(io.BytesIO):                                                  # a file still arriving
                def __init__(s, data, gate):
                    super().__init__(data)
                    s.gate = gate

                def read(s, n=-1):
                    s.gate.wait(5)
                    return super().read(n)
            gate = threading.Event()
            results = []

            def send(uid, path):
                try:
                    data = b"ID3" + b"x" * 57                                        # 60 bytes each
                    self.u.put(uid, path, Slow(data, gate), len(data))
                    results.append("ok")
                except uploads.UploadError:
                    results.append("refused")
            import time
            first = threading.Thread(target=send, args=(a, "B/0.mp3"))
            first.start()                                                            # one file of upload a on its way …
            for _ in range(200):
                if self.u.sessions[a]["active"]:
                    break
                time.sleep(0.01)
            threads = [first] + [threading.Thread(target=send, args=(uid, f"B/{i}.mp3"))
                                 for i, uid in enumerate((b, a, b), 1)]
            for th in threads[1:]:
                th.start()                                                           # … the others: over the account limit
            with self.assertRaises(uploads.UploadError):                             # finishing while files arrive
                self.u.finish(a)
            gate.set()
            for th in threads:
                th.join()
            self.assertEqual(sorted(results), ["ok", "refused", "refused", "refused"])   # 100 bytes per account
            with self.assertRaises(uploads.UploadError):
                self.put(b, "B/same.mp3", b"ID3" + b"x" * 90)                         # one upload: 80 bytes
        finally:
            uploads.MAX_SESSIONS_PER_USER, uploads.MAX_USER_BYTES, uploads.MAX_SESSION_BYTES, uploads.MAX_FILES = saved

    def test_finish_only_once_and_only_by_the_owner(self):
        uid = self.u.start("u1")
        self.put(uid, "A/01.mp3")
        with self.assertRaises(uploads.UploadError):
            self.u.put(uid, "A/01.mp3", io.BytesIO(b"ID3audio"), 8)                  # same file twice
        for fn in (lambda: self.u.finish(uid, owner="u2"), lambda: self.u.cancel(uid, owner="u2"),
                   lambda: self.u.put(uid, "A/02.mp3", io.BytesIO(b"ID3x"), 4, owner="u2")):
            with self.assertRaises(uploads.UploadError):
                fn()                                                                  # a foreign upload: "not found"
        for bad in ("../../etc", "0123456789abcdeg", uid.upper(), uid + "0", ""):
            with self.assertRaises(uploads.UploadError):
                self.u.finish(bad)
        results = []
        barrier = threading.Barrier(5)

        def finish():
            barrier.wait()
            try:
                results.append(self.u.finish(uid, owner="u1"))
            except uploads.UploadError:
                results.append(None)
        threads = [threading.Thread(target=finish) for _ in range(5)]
        for th in threads:
            th.start()
        for th in threads:
            th.join()
        self.assertEqual([r for r in results if r], [["A"]])                        # exactly one finish
        self.assertEqual(self.arrived, ["A"])
        self.assertEqual(sorted(os.listdir(self.inbox)), ["A"])

    def test_restart_completes_or_removes_staging(self):
        done = self.u.start("u1")
        self.put(done, "Fertig/01.mp3")
        self.put(done, "Zweites/01.mp3")
        half = self.u.start("u1")
        self.put(half, "Halb/01.mp3")
        staging = os.path.join(self.inbox, f".upload-{done}")
        open(os.path.join(staging, uploads.FINISHING), "w").close()                  # crash while moving …
        os.replace(os.path.join(staging, "Fertig"), os.path.join(self.inbox, "Fertig"))   # … after the first entry
        arrived = []
        uploads.Uploads(self.inbox, on_done=arrived.extend)                          # the next start
        self.assertEqual(sorted(os.listdir(self.inbox)), ["Fertig", "Zweites"])     # completed, unfinished removed
        self.assertEqual(arrived, ["Zweites"])

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

    def test_disguised_files_are_refused_by_content(self):
        """An .exe, a PDF, an archive or a script renamed to .mp3 is refused by its first bytes – nothing stays."""
        uid = self.u.start("admin")
        for name, data in (("Buch/01.mp3", b"MZ\x90\x00" + b"\x00" * 500), ("Buch/02.mp3", b"%PDF-1.7\n"),
                           ("Buch/03.mp3", b"PK\x03\x04" + b"\x00" * 50), ("Buch/04.mp3", b"#!/bin/sh\nrm -rf /\n"),
                           ("Buch/05.mp3", b"just some text, no audio"), ("Buch/cover.jpg", b"ID3\x04"),
                           ("Buch/chapters.txt", b"\x00\x01\x02binary"), ("Buch/x.cue", b"<html><script>")):
            with self.assertRaises(uploads.UploadError, msg=name):
                self.put(uid, name, data)
        self.assertEqual(os.listdir(os.path.join(self.inbox, f".upload-{uid}", "Buch")), [])      # not even a .part
        for name, data in (("Buch/01.mp3", b"\xff\xfb\x90\x00" + b"\x00" * 64), ("Buch/02.m4b", b"\x00\x00\x00\x20ftypM4B "),
                           ("Buch/03.flac", b"fLaC\x00"), ("Buch/04.ogg", b"OggS\x00"), ("Buch/cover.png", b"\x89PNG\r\n\x1a\n"),
                           ("Buch/a.cue", "TITLE \"Fräulein\"\n".encode("cp1252"))):
            self.put(uid, name, data)
        self.assertEqual(len(os.listdir(os.path.join(self.inbox, f".upload-{uid}", "Buch"))), 6)

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
