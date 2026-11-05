"""The API contract of Hörwerk – black-box: only HTTP and the folders, no import of the application.

Every backend (today Python, later Rust) must pass these unchanged:

    python3 -m unittest discover -s contract                      # against python3 app/main.py
    HW_SERVER_CMD=target/release/hoerwerk python3 -m unittest discover -s contract
"""
from __future__ import annotations

import http.client
import json
import os
import re
import shutil
import signal
import sqlite3
import subprocess
import time
import unittest

from harness import ADMIN, OWN, ROOT, Client, FakeMediaServers, FakeUsenet, Server, deliver, quote, tone

ROUTES = json.load(open(os.path.join(os.path.dirname(__file__), "routes.json"), encoding="utf-8"))["routes"]
FIXTURE_DB = os.path.join(os.path.dirname(__file__), "fixtures", "hoerwerk-7.13.db")
FAMILY = {"name": "Eva", "password": "Kaffee Garten Hund Fahrrad"}


def concrete(path: str) -> str:
    return re.sub(r"\{(\w+)\}", lambda m: "0123456789abcdef" if m.group(1) == "uid" and "upload" in path else "1", path)


class ContractTest(unittest.TestCase):
    server: Server
    admin: Client

    @classmethod
    def setUpClass(cls):
        cls.server = Server().start()
        cls.admin = Client(cls.server, **ADMIN)

    @classmethod
    def tearDownClass(cls):
        cls.server.cleanup()

    def family(self) -> Client:
        users = self.admin.get("/api/users")["users"]
        if not any(u["name"] == FAMILY["name"] for u in users):
            self.admin.call("POST", "/api/users", {**FAMILY, "role": "user"})
        return Client(self.server, **FAMILY)


class Access(ContractTest):
    def test_routes_need_login(self):
        """Every route that is not public answers 401 without a session – before reading a body."""
        for r in ROUTES:
            if r["access"] == "public":
                continue
            headers = OWN if r["method"] != "GET" else {}
            status, _, body = self.server.request(r["method"], concrete(r["path"]), headers=headers,
                                                  body={} if r["method"] != "GET" and not r["raw_body"] else None)
            self.assertEqual(status, 401, f'{r["method"]} {r["path"]}: {body[:120]!r}')
            self.assertTrue(json.loads(body).get("login"), r["path"])

    def test_admin_routes_refuse_family_accounts(self):
        eva = self.family()
        for r in ROUTES:
            if r["access"] != "admin":
                continue
            status, _ = eva.call(r["method"], concrete(r["path"]), {} if r["method"] != "GET" and not r["raw_body"] else None,
                                 expect=None)
            self.assertEqual(status, 403, f'{r["method"]} {r["path"]}')

    def test_without_login_only_the_login_page(self):
        s = self.server
        self.assertEqual(s.request("GET", "/api/health")[0], 200)
        status, headers, _ = s.request("GET", "/")
        self.assertEqual((status, headers["location"]), (303, "/login"))
        self.assertEqual(s.request("GET", "/login")[0], 200)
        self.assertEqual(s.request("GET", "/me.js")[2], b"const ME = null;\n")
        self.assertEqual(s.request("GET", "/static/app.js")[0], 401)           # no app code before login
        self.assertEqual(s.request("GET", "/static/style.css")[0], 200)

    def test_health_tells_the_version_only_after_login(self):
        self.assertNotIn("version", json.loads(self.server.request("GET", "/api/health")[2]))
        self.assertRegex(self.admin.get("/api/health")["version"], r"^\d+\.\d+\.\d+$")


class Security(ContractTest):
    def test_headers(self):
        _, h, _ = self.server.request("GET", "/login")
        self.assertIn("frame-ancestors 'none'", h["content-security-policy"])
        self.assertEqual((h["x-content-type-options"], h["x-frame-options"], h["referrer-policy"]),
                         ("nosniff", "DENY", "no-referrer"))

    def test_session_cookie(self):
        status, h, _ = self.server.request("POST", "/api/auth/login", ADMIN, headers=OWN)
        self.assertEqual(status, 200)
        cookie = h["set-cookie"]
        self.assertTrue(cookie.startswith("hw_session="))
        self.assertIn("HttpOnly", cookie)
        self.assertRegex(cookie, r"SameSite=(Strict|Lax)")

    def test_wrong_password(self):
        status, _, body = self.server.request("POST", "/api/auth/login", {"name": "admin", "password": "falsch-falsch"},
                                              headers=OWN)
        self.assertEqual(status, 401)
        self.assertIn("error", json.loads(body))

    def test_csrf_refused(self):
        change = {"language": "de"}
        status, _ = self.admin.call("PUT", "/api/settings", change, expect=None, headers={"X-Requested-With": ""})
        self.assertEqual(status, 403)
        status, _ = self.admin.call("PUT", "/api/settings", change, expect=None, headers={"Origin": "http://evil.example"})
        self.assertEqual(status, 403)
        status, _ = self.admin.call("PUT", "/api/settings", change, expect=None, headers={"Sec-Fetch-Site": "cross-site"})
        self.assertEqual(status, 403)

    def test_foreign_host_names(self):
        for host, want in (("evil.example", 421), ("tower:8080", 200), ("192.168.1.10", 200), ("nas.local", 200)):
            status, _, _ = self.server.request("GET", "/api/settings", cookie=self.admin.cookie, host=host)
            self.assertEqual(status, want, host)
        self.assertEqual(self.server.request("GET", "/api/health", host="evil.example")[0], 200)   # health checks

    def test_errors_are_json(self):
        status, h, body = self.server.request("GET", "/api/nothing-here", cookie=self.admin.cookie)
        self.assertEqual(status, 404)
        self.assertTrue(h["content-type"].startswith("application/json"))
        self.assertIn("error", json.loads(body))

    def test_body_limit(self):
        c = http.client.HTTPConnection("127.0.0.1", self.server.port, timeout=10)
        c.putrequest("POST", "/api/search/books")
        for k, v in {"Host": f"127.0.0.1:{self.server.port}", "Cookie": self.admin.cookie, **OWN,
                     "Content-Type": "application/json", "Content-Length": str(50 * 1024 * 1024)}.items():
            c.putheader(k, v)
        c.endheaders()
        self.assertEqual(c.getresponse().status, 413)
        c.close()

    def test_paths_stay_inside_the_library(self):
        for bad in ("../config", "/etc", "x/../../config", ".."):
            status, _ = self.admin.call("GET", f"/api/library/tracks?path={quote(bad)}", expect=None)
            self.assertIn(status, (400, 404), bad)
        for bad in ("/static/../main.py", "/static/%2e%2e/main.py"):
            self.assertEqual(self.server.request("GET", bad, cookie=self.admin.cookie)[0], 404)


class Accounts(ContractTest):
    def test_me_and_logout(self):
        c = Client(self.server, **ADMIN)
        me = c.get("/api/auth/me")
        self.assertEqual((me["name"], me["role"]), ("admin", "admin"))
        c.call("POST", "/api/auth/logout")
        status, _ = c.call("GET", "/api/auth/me", expect=None)
        self.assertEqual(status, 401)

    def test_family_account_lifecycle(self):
        user = self.admin.call("POST", "/api/users", {"name": "Ole", "password": "Kaffee Garten Hund Blume 7", "role": "user"})
        self.assertEqual(user["role"], "user")
        self.assertNotIn("pw", user)                                       # never the password hash
        names = [u["name"] for u in self.admin.get("/api/users")["users"]]
        self.assertIn("Ole", names)
        status, _ = self.admin.call("POST", "/api/users", {"name": "ole", "password": "Kaffee Garten Hund Blume 7"}, expect=None)
        self.assertEqual(status, 400)                                      # names are unique, any case
        status, _ = self.admin.call("POST", "/api/users", {"name": "Kurz", "password": "kurz"}, expect=None)
        self.assertEqual(status, 400)                                      # password rules
        self.admin.call("PUT", f'/api/users/{user["id"]}', {"disabled": True})
        with self.assertRaises(AssertionError):
            Client(self.server, "Ole", "Kaffee Garten Hund Blume 7")         # disabled: no login
        self.admin.call("DELETE", f'/api/users/{user["id"]}')
        self.assertNotIn("Ole", [u["name"] for u in self.admin.get("/api/users")["users"]])

    def test_last_admin_stays(self):
        me = self.admin.get("/api/auth/me")
        status, _ = self.admin.call("DELETE", f'/api/users/{me["id"]}', expect=None)
        self.assertEqual(status, 400)

    def test_invitation(self):
        status, _ = self.admin.call("POST", "/api/invites", {"days": 1}, expect=None)
        self.assertEqual(status, 400)                                      # no public address yet: no link
        self.admin.call("PUT", "/api/settings", {"public_url": "https://hoerbuecher.example.de"})
        try:
            inv = self.admin.call("POST", "/api/invites", {"note": "Mama", "days": 7, "uses": 1})
            self.assertTrue(inv["link"].startswith("https://hoerbuecher.example.de/invite/"))
            token = inv["link"].rsplit("/", 1)[1]
            self.assertEqual(self.server.request("GET", f"/api/invite/{token}")[0], 200)
            status, _, _ = self.server.request("POST", f"/api/invite/{token}", {"name": "Mama", "password": "Kaffee Garten Hund Sonne"},
                                               headers=OWN)
            self.assertEqual(status, 200)
            Client(self.server, "Mama", "Kaffee Garten Hund Sonne")
            status, _, _ = self.server.request("POST", f"/api/invite/{token}", {"name": "Papa", "password": "Kaffee Garten Hund Regen"},
                                               headers=OWN)
            self.assertNotEqual(status, 200)                               # used up
        finally:
            self.admin.call("PUT", "/api/settings", {"public_url": ""})

    def test_second_factor(self):
        """TOTP: set up with a code, then every login needs one; a code works once; the admin can switch it off."""
        import base64
        import hashlib
        import hmac as hmaclib

        def code(secret: str, step: int) -> str:
            key = base64.b32decode(secret + "=" * (-len(secret) % 8), casefold=True)
            d = hmaclib.new(key, step.to_bytes(8, "big"), hashlib.sha1).digest()
            o = d[-1] & 15
            return f"{(int.from_bytes(d[o:o + 4], 'big') & 0x7FFFFFFF) % 1000000:06d}"

        name, pw = "Zweifaktor", "Kaffee Garten Hund Regen 9"
        self.admin.call("POST", "/api/users", {"name": name, "password": pw, "role": "user"})
        user = Client(self.server, name, pw)
        setup = user.call("POST", "/api/auth/totp/setup")
        self.assertTrue(setup["uri"].startswith("otpauth://totp/"))
        step = int(time.time() // 30)
        status, _ = user.call("POST", "/api/auth/totp/confirm", {"code": "000000" if code(setup["secret"], step) != "000000" else "111111"}, expect=None)
        self.assertEqual(status, 400)
        user.call("POST", "/api/auth/totp/confirm", {"code": code(setup["secret"], step)})
        status, _, body = self.server.request("POST", "/api/auth/login", {"name": name, "password": pw}, headers=OWN)
        first = json.loads(body)
        self.assertEqual((status, first["ok"], first["totp"]), (200, False, True))
        ticket = first["ticket"]
        status, h, body = self.server.request("POST", "/api/auth/login/totp", {"ticket": ticket, "code": code(setup["secret"], step)},
                                              headers=OWN)
        self.assertEqual(status, 401, "a code that was used once does not work again")
        status, h, body = self.server.request("POST", "/api/auth/login/totp",
                                              {"ticket": ticket, "code": code(setup["secret"], step + 1)}, headers=OWN)
        self.assertEqual(status, 200, body)
        self.assertIn("hw_session=", h.get("set-cookie", ""))
        status, _, _ = self.server.request("POST", "/api/auth/login/totp", {"ticket": ticket, "code": "123456"}, headers=OWN)
        self.assertEqual(status, 401, "a ticket ends with the login")
        uid = next(u["id"] for u in self.admin.get("/api/users")["users"] if u["name"] == name)
        self.assertTrue(next(u for u in self.admin.get("/api/users")["users"] if u["id"] == uid)["totp"])
        self.admin.call("PUT", f"/api/users/{uid}", {"totp": False})            # lost phone: the admin switches it off
        status, _, body = self.server.request("POST", "/api/auth/login", {"name": name, "password": pw}, headers=OWN)
        self.assertEqual((status, json.loads(body)["ok"]), (200, True))


class Settings(ContractTest):
    def test_read_change_refuse(self):
        s = self.admin.get("/api/settings")
        for key in ("language", "audio_quality", "ab_region", "public_url"):
            self.assertIn(key, s)
        changed = self.admin.call("PUT", "/api/settings", {"audio_quality": "original"})
        self.assertEqual(changed["audio_quality"], "original")
        self.assertEqual(self.admin.get("/api/settings")["audio_quality"], "original")
        status, _ = self.admin.call("PUT", "/api/settings", {"audio_quality": "laut"}, expect=None)
        self.assertEqual(status, 400)
        status, _ = self.admin.call("PUT", "/api/settings", {"public_url": "javascript:alert(1)"}, expect=None)
        self.assertEqual(status, 400)
        self.admin.call("PUT", "/api/settings", {"audio_quality": "high"})

    def test_family_may_read_not_change(self):
        eva = self.family()
        self.assertIn("language", eva.get("/api/settings"))
        self.assertEqual(eva.call("PUT", "/api/settings", {"language": "en"}, expect=None)[0], 403)

    def test_keys_are_never_shown(self):
        self.admin.call("PUT", "/api/sources", {"indexers": [{"name": "Test", "url": "https://indexer.example",
                                                              "apikey": "geheim-123", "enabled": True}],
                                               "sab": {"url": "", "apikey": ""}})
        src = self.admin.get("/api/sources")
        self.assertTrue(src["indexers"][0]["apikey_set"])
        self.assertNotIn("geheim-123", json.dumps(src))


class Requests(ContractTest):
    BOOK = {"asin": "B0VERTRAG1", "title": "Der Vertrag", "authors": ["Anna Autorin"], "language": "german",
            "runtime_min": 300}

    def test_family_asks_admin_declines(self):
        eva = self.family()
        item = eva.call("POST", "/api/requests", {"book": self.BOOK})
        self.assertEqual(item["book"]["title"], "Der Vertrag")
        self.assertIn(item["status"], ("pending", "searching"))
        self.assertIn(item["id"], [r["id"] for r in eva.get("/api/requests")["items"]])
        self.assertEqual(eva.get("/api/requests/keys")["keys"] and True, True)
        status, _ = eva.call("POST", "/api/requests", {"book": self.BOOK}, expect=None)
        self.assertIn(status, (400, 409))                                  # the same book twice
        self.admin.call("POST", f'/api/requests/{item["id"]}/decline', {"reason": "gibt es schon"})
        self.assertEqual(eva.get(f'/api/requests/{item["id"]}')["status"], "declined")
        self.assertIn("total", self.admin.get("/api/stats?period=all"))

    def test_search_finds_the_book_and_sends_it_to_sabnzbd(self):
        """Indexer and SABnzbd (a local fake): the request finds the right release; keys never reach the browser."""
        size = 300 * 60 * 128000 // 8
        fake = FakeUsenet([{"title": "Bert.Andere-Der.Vertrag.German.M4B", "size": size},          # other author
                           {"title": "Anna.Autorin-Der.Vertrag.German.M4B", "size": size},
                           {"title": "Anna.Autorin-Der.Vertrag.English.M4B", "size": size}])       # other language
        try:
            saved = self.admin.call("PUT", "/api/sources", {
                "indexers": [{"name": "Fake", "url": fake.url, "apikey": "indexerkey123"}],
                "sab": {"url": fake.url, "apikey": "sabkey123456", "category": "hoerbuecher"}})
            self.assertEqual(saved["configured"], {"indexers": 1, "sab": True})
            self.assertTrue(saved["sab"]["apikey_set"])
            idx = saved["indexers"][0]
            self.assertEqual(self.admin.call("POST", "/api/sources/test", {"kind": "sab"}),
                             {"ok": True, "message": "SABnzbd 4.5.0"})
            self.assertTrue(self.admin.call("POST", "/api/sources/test", {"kind": "indexer", "id": idx["id"]})["ok"])
            book = {**self.BOOK, "asin": "B0VERTRAG2"}
            item = self.admin.call("POST", "/api/requests", {"book": book})
            self.assertEqual(item["status"], "searching")                     # admins need no approval

            def sent():
                r = self.admin.get(f'/api/requests/{item["id"]}')
                return r if r["status"] == "downloading" else None
            r = self.admin.wait(sent, 30, what="release sent")
            self.assertEqual(r["release"], "Anna.Autorin-Der.Vertrag.German.M4B")
            self.assertEqual(len(fake.added), 1)
            self.assertTrue(fake.added[0]["name"].endswith("/getnzb/1?r=secretkey123"))
            self.assertEqual((fake.added[0]["cat"], fake.added[0]["apikey"]), ("hoerbuecher", "sabkey123456"))
            downloads = self.admin.get("/api/downloads")
            self.assertEqual([d["name"] for d in downloads["items"]], ["Anna.Autorin-Der.Vertrag.German.M4B"])
            found = self.admin.call("POST", f'/api/requests/{item["id"]}/releases', {})
            verdicts = {x["title"]: x["verdict"]["ok"] for x in found["results"]}
            self.assertEqual(verdicts, {"Anna.Autorin-Der.Vertrag.German.M4B": True,
                                        "Bert.Andere-Der.Vertrag.German.M4B": False,
                                        "Anna.Autorin-Der.Vertrag.English.M4B": False})
            for path in ("/api/sources", f'/api/requests/{item["id"]}', "/api/requests", "/api/downloads"):
                body = json.dumps(self.admin.get(path))
                for secret in ("indexerkey123", "sabkey123456", "secretkey123"):
                    self.assertNotIn(secret, body, path)
            self.assertNotIn("secretkey123", json.dumps(found))
        finally:
            self.admin.call("PUT", "/api/sources", {"indexers": [], "sab": {}})
            fake.close()


class Pipeline(ContractTest):
    """The core promise: a download in the inbox becomes a tagged M4B with chapters in the library."""

    def library_files(self):
        out = []
        for base, _, files in os.walk(self.server.library):
            out += [os.path.relpath(os.path.join(base, f), self.server.library) for f in files]
        return sorted(out)

    def test_inbox_to_library(self):
        tags = {"artist": "Theodor Storm", "album_artist": "Theodor Storm", "album": "Der Schimmelreiter",
                "genre": "Hörbuch", "date": "1888"}
        deliver(self.server.inbox, "Theodor Storm - Der Schimmelreiter", {
            "01.mp3": {**tags, "title": "Teil 1", "track": "1"}, "02.mp3": {**tags, "title": "Teil 2", "track": "2"}})
        self.admin.call("POST", "/api/audiobooks/scan")

        def filed():
            jobs = self.admin.get("/api/audiobooks")["jobs"]
            job = next((j for j in jobs if j["name"] == "Theodor Storm - Der Schimmelreiter"), None)
            if job and job["status"] in ("failed", "review"):
                raise AssertionError(job)
            return job if job and job["status"] == "done" else None
        job = self.admin.wait(filed, 90, what="job done")
        m4b = [f for f in self.library_files() if f.endswith(".m4b")]
        self.assertEqual(m4b, ["Theodor Storm/Der Schimmelreiter/Der Schimmelreiter.m4b"])
        probe = json.loads(subprocess.run(
            ["ffprobe", "-v", "error", "-print_format", "json", "-show_format", "-show_chapters",
             os.path.join(self.server.library, m4b[0])], capture_output=True, text=True, check=True).stdout)
        self.assertEqual(len(probe["chapters"]), 2)
        tags = {k.lower(): v for k, v in probe["format"]["tags"].items()}
        self.assertEqual(tags.get("album"), "Der Schimmelreiter")
        self.assertIn("Theodor Storm", tags.get("artist", ""))
        self.assertTrue(os.path.isfile(os.path.join(self.server.library, os.path.dirname(m4b[0]), "metadata.json")))
        self.assertFalse(os.path.exists(os.path.join(self.server.inbox, "Theodor Storm - Der Schimmelreiter")))
        books = self.admin.get("/api/library/books?q=Schimmelreiter")
        self.assertEqual([b["title"] for b in books["books"]], ["Der Schimmelreiter"])
        self.assertEqual(job["dest"], os.path.join(self.server.library, m4b[0]))

    def test_upload_lands_in_the_inbox(self):
        src = os.path.join(self.server.dir, "upload.mp3")
        tone(src, tags={"artist": "Max Muster", "album": "Hochgeladen"})
        data = open(src, "rb").read()
        uid = self.admin.call("POST", "/api/upload")["id"]
        status, _, body = self.server.request("PUT", f"/api/upload/{uid}?path={quote('Max Muster - Hochgeladen/01.mp3')}",
                                              raw=data, headers={**OWN, "Content-Type": "application/octet-stream"},
                                              cookie=self.admin.cookie)
        self.assertEqual(status, 200, body)
        status, _, _ = self.server.request("PUT", f"/api/upload/{uid}?path={quote('A/run.sh')}", raw=b"#!/bin/sh",
                                           headers=OWN, cookie=self.admin.cookie)
        self.assertEqual(status, 400)                                     # only audio, covers, Hörwerk's text files
        names = self.admin.call("POST", f"/api/upload/{uid}/finish")["names"]
        self.assertEqual(names, ["Max Muster - Hochgeladen"])
        self.admin.wait(lambda: any(j["name"] == "Max Muster - Hochgeladen" for j in self.admin.get("/api/audiobooks")["jobs"]),
                        30, what="upload becomes a job")


class Processing(ContractTest):
    """The pipeline's other paths: duplicates, broken downloads, packs, own data, taking a book back."""

    TAGS = {"artist": "Anna Autorin", "album_artist": "Anna Autorin", "genre": "Hörbuch"}

    def job(self, name: str, until=("done", "failed", "review", "cancelled"), seconds: float = 90) -> dict:
        def find():
            j = next((j for j in self.admin.get("/api/audiobooks")["jobs"] if j["name"] == name), None)
            return j if j and j["status"] in until else None
        return self.admin.wait(find, seconds, what=f"{name} in {until}")

    def book(self, name: str, album: str, n: int = 1, extra: dict = None) -> None:
        deliver(self.server.inbox, name, {f"{i + 1:02d}.mp3": {**self.TAGS, "album": album, "title": f"Teil {i + 1}",
                                                               "track": str(i + 1), **(extra or {})} for i in range(n)})

    def test_duplicate_is_asked_then_replaced(self):
        self.book("Anna Autorin - Doppelt", "Doppelt")
        first = self.job("Anna Autorin - Doppelt")
        self.assertEqual(first["status"], "done", first)
        self.book("Anna Autorin - Doppelt (neu)", "Doppelt", n=2)
        second = self.job("Anna Autorin - Doppelt (neu)")
        self.assertEqual(second["status"], "review")
        self.assertEqual(second["duplicate"]["path"], "Anna Autorin/Doppelt")
        self.assertEqual(second["duplicate"]["why"], "title")
        self.assertEqual(sorted(second["duplicate"]["quality"]), ["new", "old"])
        self.admin.call("POST", f'/api/audiobooks/{second["id"]}/duplicate', {"action": "replace"})
        done = self.job("Anna Autorin - Doppelt (neu)", until=("done", "failed"))
        self.assertEqual(done["status"], "done", done)
        self.assertEqual(done["dest"], os.path.join(self.server.library, "Anna Autorin", "Doppelt", "Doppelt.m4b"))
        moved = os.path.join(self.server.inbox, "_ersetzt", "Anna Autorin", "Doppelt", "Doppelt.m4b")
        self.assertTrue(os.path.isfile(moved), "the old copy is kept in _ersetzt")

    def test_duplicate_can_be_discarded(self):
        self.book("Anna Autorin - Einmal", "Einmal")
        self.assertEqual(self.job("Anna Autorin - Einmal")["status"], "done")
        self.book("Anna Autorin - Einmal (2)", "Einmal")
        second = self.job("Anna Autorin - Einmal (2)")
        self.assertEqual(second["status"], "review")
        out = self.admin.call("POST", f'/api/audiobooks/{second["id"]}/duplicate', {"action": "discard"})
        self.assertEqual(out["status"], "cancelled")
        self.assertTrue(os.path.isdir(os.path.join(self.server.inbox, "_fertig", "Anna Autorin - Einmal (2)")))
        status, _ = self.admin.call("POST", f'/api/audiobooks/{second["id"]}/duplicate', {"action": "wegwerfen"},
                                    expect=None)
        self.assertEqual(status, 400)

    def test_broken_download_fails_and_comes_back_on_retry(self):
        folder = os.path.join(self.server.dir, ".incoming", "Kaputt - Buch")
        os.makedirs(folder)
        with open(os.path.join(folder, "01.mp3"), "wb") as fh:
            fh.write(b"ID3" + b"\x00" * 4000)                               # looks like MP3, holds no audio
        os.rename(folder, os.path.join(self.server.inbox, "Kaputt - Buch"))
        job = self.job("Kaputt - Buch")
        self.assertEqual(job["status"], "failed")
        self.assertTrue(job["error"])
        failed = os.path.join(self.server.inbox, "_failed", "Kaputt - Buch")
        self.assertTrue(os.path.isfile(os.path.join(failed, "FEHLER.txt")))
        self.assertEqual(job["dest"], failed)
        detail = self.admin.get(f'/api/audiobooks/{job["id"]}')
        self.assertTrue(any("Fehler" in line for line in detail["log"]))
        again = self.admin.call("POST", f'/api/audiobooks/{job["id"]}/retry')
        self.assertEqual(again["status"], "queued")
        self.assertTrue(os.path.isdir(os.path.join(self.server.inbox, "Kaputt - Buch")))
        self.assertFalse(os.path.exists(os.path.join(self.server.inbox, "Kaputt - Buch", "FEHLER.txt")))
        self.assertEqual(self.job("Kaputt - Buch")["status"], "failed")
        self.admin.call("DELETE", f'/api/audiobooks/{job["id"]}')
        self.assertNotIn(job["id"], [j["id"] for j in self.admin.get("/api/audiobooks")["jobs"]])

    def test_pack_becomes_one_job_per_volume(self):
        tmp = os.path.join(self.server.dir, ".incoming", "Paula Pack - Reihe 01-02")
        for vol, title in ((1, "Erster Fall"), (2, "Zweiter Fall")):
            tone(os.path.join(tmp, f"0{vol} - {title}", "01.mp3"),
                 tags={"artist": "Paula Pack", "album": title, "title": "Teil 1"})
        os.rename(tmp, os.path.join(self.server.inbox, "Paula Pack - Reihe 01-02"))
        a, b = self.job("01 - Erster Fall"), self.job("02 - Zweiter Fall")
        self.assertEqual((a["status"], b["status"]), ("done", "done"), (a, b))
        self.assertEqual((a["pack"]["volume"], b["pack"]["volume"]), (1, 2))
        self.assertEqual(a["pack"]["series"], "Reihe")
        m4b = sorted(os.path.relpath(os.path.join(r, f), self.server.library)
                     for r, _, fs in os.walk(self.server.library) for f in fs if f.endswith(".m4b") and "Paula" in r)
        self.assertEqual(m4b, ["Paula Pack/Erster Fall/Erster Fall.m4b", "Paula Pack/Zweiter Fall/Zweiter Fall.m4b"])

    def test_metadata_file_wins_and_book_can_be_taken_back(self):
        tmp = os.path.join(self.server.dir, ".incoming", "Unbekannt - Eigenes")
        tone(os.path.join(tmp, "01.mp3"), tags={"artist": "Irgendwer", "album": "Falscher Titel"})
        with open(os.path.join(tmp, "metadata.json"), "w", encoding="utf-8") as fh:
            json.dump({"title": "Richtiger Titel", "authors": ["Ruth Richtig"], "series": ["Meine Reihe #2"],
                       "narrators": "Sina Sprecherin, Sven Sprecher", "publishedYear": 2001}, fh)
        os.rename(tmp, os.path.join(self.server.inbox, "Unbekannt - Eigenes"))
        job = self.job("Unbekannt - Eigenes")
        self.assertEqual(job["status"], "done", job)
        self.assertEqual(job["match"]["source"], "metadata.json")
        self.assertEqual(job["match"]["series_position"], "2")
        self.assertEqual(job["match"]["narrators"], ["Sina Sprecherin", "Sven Sprecher"])
        self.assertEqual(job["dest"], os.path.join(self.server.library, "Ruth Richtig", "Meine Reihe",
                                                   "02 - Richtiger Titel", "Richtiger Titel.m4b"))
        meta = json.load(open(os.path.join(os.path.dirname(job["dest"]), "metadata.json"), encoding="utf-8"))
        self.assertEqual((meta["series"], meta["publishedYear"]), (["Meine Reihe #2"], "2001"))
        back = self.admin.call("POST", f'/api/audiobooks/{job["id"]}/reopen')
        self.assertEqual(back["status"], "review")
        self.assertFalse(os.path.exists(job["dest"]))
        self.assertFalse(os.path.exists(os.path.join(self.server.library, "Ruth Richtig")))   # empty folders go
        # own data on top of the file's tags: an empty field keeps what the tags say (here: the series)
        self.admin.call("POST", f'/api/audiobooks/{job["id"]}/manual',
                        {"title": "Neuer Titel", "authors": "Ruth Richtig, Rolf Richtig", "series": ""})
        again = self.job("Unbekannt - Eigenes", until=("done", "failed"))
        self.assertEqual(again["status"], "done", again)
        self.assertEqual(again["match"]["authors"], ["Ruth Richtig", "Rolf Richtig"])
        self.assertEqual(again["dest"], os.path.join(self.server.library, "Ruth Richtig", "Meine Reihe",
                                                     "02 - Neuer Titel", "Neuer Titel.m4b"))
        books = self.admin.get("/api/library/books?q=richtig")["books"]
        self.assertEqual([b["title"] for b in books], ["Neuer Titel"])

    def test_media_servers_are_asked_to_scan_after_filing(self):
        """Audiobookshelf and Plex (a local fake): connection test picks the library, tokens stay on the server,
        one scan each after a book was filed."""
        fake = FakeMediaServers()
        try:
            saved = self.admin.call("PUT", "/api/sources", {
                "abs": {"url": fake.url, "token": "abs-geheim-123"},
                "plex": {"url": fake.url + "/web/index.html", "token": "plex-geheim-456"}})
            self.assertEqual((saved["abs"]["token_set"], saved["plex"]["token_set"]), (True, True))
            self.assertEqual(saved["plex"]["url"], fake.url)
            abs_test = self.admin.call("POST", "/api/sources/test", {"kind": "abs"})
            plex_test = self.admin.call("POST", "/api/sources/test", {"kind": "plex"})
            self.assertTrue(abs_test["ok"] and "Hörbücher" in abs_test["message"], abs_test)
            self.assertTrue(plex_test["ok"] and "Hörbücher" in plex_test["message"], plex_test)
            sources = self.admin.get("/api/sources")
            self.assertEqual((sources["abs"]["library"], sources["abs"]["configured"]), ("lib-books", True))
            self.assertEqual((sources["plex"]["section"], sources["plex"]["configured"]), ("7", True))
            self.assertNotIn("geheim", json.dumps(sources))
            self.book("Anna Autorin - Gescannt", "Gescannt")
            self.assertEqual(self.job("Anna Autorin - Gescannt")["status"], "done")

            def scanned():
                paths = [c[1] for c in fake.calls]
                return "/api/libraries/lib-books/scan" in paths and "/library/sections/7/refresh" in paths
            self.admin.wait(scanned, 75, what="scans after filing")
            scan = next(c for c in fake.calls if c[1] == "/api/libraries/lib-books/scan")
            self.assertEqual((scan[0], scan[2]), ("POST", "Bearer abs-geheim-123"))
            refresh = next(c for c in fake.calls if c[1] == "/library/sections/7/refresh")
            self.assertEqual(refresh[3], "plex-geheim-456")
            # another server: the stored token does not go there
            moved = self.admin.call("PUT", "/api/sources", {"abs": {"url": "http://127.0.0.2:9", "library": "x"}})
            self.assertFalse(moved["abs"]["token_set"])
        finally:
            self.admin.call("PUT", "/api/sources", {"abs": {"url": ""}, "plex": {"url": ""}})
            fake.close()

    def test_family_links_audiobookshelf_and_sees_what_they_heard(self):
        fake = FakeMediaServers()
        try:
            self.admin.call("PUT", "/api/sources", {"abs": {"url": fake.url, "token": "abs-geheim-123", "library": "lib-books"}})
            eva = Client(self.server, *self._family())
            mine = eva.get("/api/me/listening")
            self.assertEqual((mine["available"]["abs"], mine["links"]["abs"], mine["recent"]), (True, None, None))
            status, _ = eva.call("POST", "/api/me/listening/abs", {"username": "eva", "password": "falsch"}, expect=None)
            self.assertEqual(status, 400)
            linked = eva.call("POST", "/api/me/listening/abs", {"username": "eva", "password": "richtig"})
            self.assertEqual(linked["links"]["abs"], {"id": "abs-eva", "name": "eva"})
            recent = eva.get("/api/me/listening")["recent"]
            self.assertEqual((recent["seconds"], recent["books"], recent["finished"]), (5400, 2, 1))
            accounts = self.admin.get("/api/listening/accounts")
            self.assertEqual([a["id"] for a in accounts["abs"]], ["abs-eva", "abs-max"])
            uid = next(u["id"] for u in accounts["users"] if u["links"]["abs"])
            stats = self.admin.get("/api/stats?period=30")
            self.assertEqual((stats["listen"]["seconds"], stats["listen"]["linked"]), (5400, 1))
            heard = self.admin.get(f"/api/stats/accounts/{uid}?period=30")["heard"]
            self.assertEqual(sorted(h["title"] for h in heard), ["Gehört", "Halb"])
            self.admin.call("PUT", "/api/listening/link", {"kind": "abs", "user_id": uid, "target_id": ""})
            self.assertIsNone(eva.get("/api/me/listening")["links"]["abs"])
            status, _ = self.admin.call("PUT", "/api/listening/link", {"kind": "abs", "user_id": uid, "target_id": "nicht-da"},
                                        expect=None)
            self.assertEqual(status, 400)
        finally:
            self.admin.call("PUT", "/api/sources", {"abs": {"url": ""}})
            fake.close()

    def _family(self):
        name, pw = "Familie Hören", "Kaffee Garten Hund Sonne 3"
        if not any(u["name"] == name for u in self.admin.get("/api/users")["users"]):
            self.admin.call("POST", "/api/users", {"name": name, "password": pw, "role": "user"})
        return name, pw


class Live(ContractTest):
    def test_events_stream(self):
        c = http.client.HTTPConnection("127.0.0.1", self.server.port, timeout=10)
        c.request("GET", "/api/events?topics=audiobooks,library",
                  headers={"Host": f"127.0.0.1:{self.server.port}", "Cookie": self.admin.cookie})
        r = c.getresponse()
        self.assertEqual(r.status, 200)
        self.assertTrue(r.getheader("Content-Type").startswith("text/event-stream"))
        seen = set()
        for _ in range(40):
            line = r.fp.readline().decode()
            if line.startswith("event:"):
                seen.add(line.split(":", 1)[1].strip())
            if line.startswith("data:") and seen:
                json.loads(line[5:])                                       # every event carries JSON
            if seen >= {"audiobooks", "library"}:
                break
        self.assertEqual(seen, {"audiobooks", "library"})
        c.close()

    def test_logs(self):
        res = self.admin.get("/api/logs?after=0&level=info")
        self.assertIn("entries", res)
        self.assertIsInstance(res["last"], int)


class Lifecycle(unittest.TestCase):
    def test_sigterm_ends_in_order(self):
        s = Server().start()
        try:
            t0 = time.monotonic()
            code = s.stop(signal.SIGTERM, timeout=10)
            self.assertLess(time.monotonic() - t0, 9)                     # within Docker's grace time
            self.assertEqual(code, 0, s.output())
            wal = os.path.join(s.config, "hoerwerk.db-wal")
            self.assertTrue(not os.path.exists(wal) or os.path.getsize(wal) == 0)
        finally:
            s.cleanup()

    def test_schema(self):
        """A fresh database has exactly the schema of contract/schema.sql (tables, indexes, user_version)."""
        s = Server().start()
        try:
            s.stop()
            con = sqlite3.connect(os.path.join(s.config, "hoerwerk.db"))
            have = [r[0].strip() + ";" for r in con.execute(
                "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type, name")]
            version = con.execute("PRAGMA user_version").fetchone()[0]
            con.close()
            spec = open(os.path.join(os.path.dirname(__file__), "schema.sql"), encoding="utf-8").read()
            body = "\n".join(line for line in spec.splitlines() if not line.startswith("--"))
            norm = lambda x: re.sub(r"\s+", " ", x).strip().rstrip(";")      # noqa: E731
            self.assertEqual([norm(x) for x in have], [norm(x) for x in body.split(";\n") if x.strip()])
            self.assertEqual(version, int(re.search(r"user_version = (\d+)", spec).group(1)))
        finally:
            s.cleanup()

    def test_invalid_setting_stops_the_start(self):
        s = Server(env={"ALLOWED_HOSTS": "https://hoerbuecher.example.de/"})
        try:
            with self.assertRaises(RuntimeError):
                s.start()
            self.assertIn("ALLOWED_HOSTS", s.output())
        finally:
            s.cleanup()

    @unittest.skipUnless(os.path.exists(FIXTURE_DB), "fixture missing")
    def test_takes_over_an_existing_database(self):
        """A database written by Hörwerk 7.13 (Python): accounts, settings, requests and library survive."""
        s = Server(env={"AUTH_USER": "", "AUTH_PASS": ""})
        shutil.copy(FIXTURE_DB, os.path.join(s.config, "hoerwerk.db"))
        try:
            s.start()
            admin = Client(s, "Alt-Admin", "Schimmelreiter-Kaffee-713")
            self.assertEqual(admin.get("/api/settings")["audio_quality"], "original")
            users = {u["name"]: u for u in admin.get("/api/users")["users"]}
            self.assertEqual(users["Eva"]["role"], "user")
            Client(s, "Eva", "Kaffee Garten Hund Fahrrad")                # her password still works
            titles = [r["book"]["title"] for r in admin.get("/api/requests")["items"]]
            self.assertIn("Aus der alten Datenbank", titles)
            version = sqlite3.connect(os.path.join(s.config, "hoerwerk.db")).execute("PRAGMA user_version").fetchone()[0]
            self.assertGreaterEqual(version, 5)
        finally:
            s.cleanup()


class Library(unittest.TestCase):
    """An existing collection is read the same way: folder layouts, tags, CD folders, M4B, covers."""

    def test_reads_an_existing_collection(self):
        s = Server()
        lib = s.library
        tone(os.path.join(lib, "Theodor Storm", "Der Schimmelreiter", "01.mp3"),
             tags={"artist": "Gert Westphal", "album": "Der Schimmelreiter"})                   # artist = narrator
        tone(os.path.join(lib, "Anna Autorin", "Die Reihe", "03 - Dritter Band", "01.mp3"))
        tone(os.path.join(lib, "Max Muster - Hörbuch", "CD1", "01.mp3"))
        tone(os.path.join(lib, "Max Muster - Hörbuch", "CD2", "01.mp3"))
        tone(os.path.join(lib, "Erika Beispiel", "Eigenes", "Eigenes.m4b"),
             tags={"album": "Mein Titel", "album_artist": "Erika Beispiel", "artist": "Erika Beispiel"})
        tone(os.path.join(lib, ".versteckt", "x.mp3"))
        tone(os.path.join(lib, "_geparkt", "x.mp3"))
        shutil.copy(os.path.join(os.path.dirname(__file__), "golden", "cover.jpg"),
                    os.path.join(lib, "Theodor Storm", "Der Schimmelreiter", "cover.jpg"))
        try:
            s.start()
            admin = Client(s, ADMIN["name"], ADMIN["password"])
            admin.wait(lambda: admin.get("/api/library")["books"] == 4, 30, what="library read")
            keys = ("path", "title", "authors", "series", "series_position", "format", "files", "cover")
            books = [{k: b[k] for k in keys} for b in admin.get("/api/library/books")["books"]]
            self.assertEqual(books, [
                {"path": "Anna Autorin/Die Reihe/03 - Dritter Band", "title": "Dritter Band", "authors": ["Anna Autorin"],
                 "series": "Die Reihe", "series_position": "03", "format": "mp3", "files": 1, "cover": False},
                {"path": "Erika Beispiel/Eigenes", "title": "Mein Titel", "authors": ["Erika Beispiel"], "series": None,
                 "series_position": None, "format": "m4b", "files": 1, "cover": False},
                {"path": "Max Muster - Hörbuch", "title": "Hörbuch", "authors": ["Max Muster"], "series": None,
                 "series_position": None, "format": "mp3", "files": 2, "cover": False},
                {"path": "Theodor Storm/Der Schimmelreiter", "title": "Der Schimmelreiter", "authors": ["Theodor Storm"],
                 "series": None, "series_position": None, "format": "mp3", "files": 1, "cover": True},
            ])
            found = admin.get("/api/library/books?q=westphal")["books"]              # narrators are found too
            self.assertEqual([b["path"] for b in found], ["Theodor Storm/Der Schimmelreiter"])
            status, h, body = s.request("GET", "/api/library/cover?path=" + quote("Theodor Storm/Der Schimmelreiter"),
                                        cookie=admin.cookie)
            self.assertEqual((status, h["content-type"]), (200, "image/jpeg"))
            tracks = admin.get("/api/library/tracks?path=" + quote("Max Muster - Hörbuch"))
            self.assertEqual([t["file"] for t in tracks["tracks"]], ["CD1/01.mp3", "CD2/01.mp3"])
            self.assertEqual(admin.get("/api/library/unconverted")["books"][0]["path"],
                             "Anna Autorin/Die Reihe/03 - Dritter Band")
        finally:
            s.cleanup()


def _mark_pending() -> None:
    """HW_CONTRACT_PENDING=file: tests a server under construction does not pass yet (one "Class.test" per line).
    They run as expected failures – one that starts passing fails the run, so the list can only shrink."""
    path = os.environ.get("HW_CONTRACT_PENDING")
    if not path:
        return
    with open(path, encoding="utf-8") as f:
        names = [line.split("#")[0].strip() for line in f]
    for name in filter(None, names):
        cls, _, test = name.partition(".")
        target = globals()[cls]
        setattr(target, test, unittest.expectedFailure(getattr(target, test)))


_mark_pending()

if __name__ == "__main__":
    unittest.main()

_ = ROOT
