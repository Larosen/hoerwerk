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

from harness import ADMIN, OWN, ROOT, Client, Server, deliver, quote, tone

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
