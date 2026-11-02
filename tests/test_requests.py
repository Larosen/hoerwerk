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


class VolumeTest(unittest.TestCase):
    """Band 1 was asked for – "Twilight-Saga 5" must not come instead (a real case)."""
    TWILIGHT = {"title": "Twilight", "authors": ["Stephenie Meyer"], "series": "The Twilight Saga", "series_position": "1",
                "runtime_min": 771}

    def test_other_volume_is_rejected(self):
        v = releases.evaluate({"title": "Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010",
                               "language": "de"}, self.TWILIGHT)
        self.assertFalse(v["ok"])
        self.assertIn("anderer Band (5 statt 1)", v["reason"])

    def test_same_volume_and_no_number_pass(self):
        for name in ("Stephenie Meyer - Twilight Saga 1 - Twilight - Hoerbuch", "Stephenie_Meyer-Twilight-Band_01-DE",
                     "Stephenie Meyer - Twilight (2010)", "Stephenie Meyer - Twilight - 12CD - 2008"):
            self.assertTrue(releases.evaluate({"title": name}, self.TWILIGHT, check_size=False)["ok"], name)

    def test_the_real_list(self):
        """The releases the indexer really returned for "meyer Twilight": Band 1 wins, the others are out."""
        names = {
            "Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010": ({"size": 252 << 20, "format": "MP3", "language": "de"}, False),
            "Meyer Edwards-Unplugged Tribute to Twilight-16BIT-WEB-FLAC-2010": ({"size": 158 << 20, "format": "FLAC"}, False),
            "Stephenie.Meyer.-.Twilight.4.-.Bis(s).Zum.Ende.Der.Nacht": ({"size": 493 << 20}, False),
            "Stephenie.Meyer.-.Twilight.1.-.Bis(s).zum.Morgengrauen": ({"size": 858 << 20}, True),
            "Stephenie.Meyer.-.Twilight.2.-.Bis(s).zur.Mittagsstunde": ({"size": 924 << 20}, False),
            "Stephanie Meyer - Twilight 03 - Bis(s) zum Abendrot": ({"size": 943 << 20}, False),
            "Stephanie Meyer - Twilight 01 - Bis(s) zum Morgengrauen": ({"size": 814 << 20}, True),
            "Stephenie Meyer-Twilight 01 - 05-AUDiOBOOK-5CD-MP3-DE": ({"size": 3800 << 20, "format": "MP3", "language": "de"}, True),
        }
        ranked = releases.rank([{"title": n, **extra} for n, (extra, _) in names.items()], self.TWILIGHT)
        for r in ranked:
            self.assertEqual(r["verdict"]["ok"], names[r["title"]][1], (r["title"], r["verdict"]))
        self.assertIn("Twilight.1", ranked[0]["title"].replace(" ", "."))   # a single Band 1 first, not the pack

    def test_english_request_gets_english_releases(self):
        """Theresa wanted Twilight in English: German releases – tagged or just German words – are out."""
        book = {**self.TWILIGHT, "language": "English"}
        lang = releases.book_language(book)
        self.assertEqual(lang, "en")
        judge = lambda name, **kw: releases.evaluate({"title": name, **kw}, book, prefer_language=lang, check_size=False)  # noqa: E731
        self.assertIn("vermutlich andere Sprache (de)", judge("Stephenie.Meyer.-.Twilight.1.-.Bis(s).zum.Morgengrauen")["reason"])
        self.assertIn("andere Sprache (de)", judge("Stephenie Meyer - Twilight 01 - 05-AUDiOBOOK-5CD-MP3-DE", language="de")["reason"])
        self.assertTrue(judge("Stephenie Meyer - Twilight - The Twilight Saga Book 1 - Unabridged")["ok"])
        self.assertTrue(judge("Stephenie.Meyer.Twilight.1.M4B")["ok"])               # no hint either way: allowed
        # and the other way round: a German request does not take an English edition
        de_book = {**self.TWILIGHT, "language": "German"}
        v = releases.evaluate({"title": "Stephenie Meyer - Twilight - The Twilight Saga Book 1 - Unabridged"}, de_book,
                              prefer_language="de", check_size=False)
        self.assertIn("vermutlich andere Sprache (en)", v["reason"])

    def test_volume_words(self):
        book = {"series": "Harry Potter"}
        self.assertEqual(releases.release_volumes("J.K. Rowling - Harry Potter 3 - Der Gefangene von Askaban", book), {3})
        self.assertEqual(releases.release_volumes("Die drei Fragezeichen Folge 012 - 2019", {}), {12})
        self.assertEqual(releases.release_volumes("Autor - Titel - Band 7 - 4CD - 2010", {}), {7})
        self.assertEqual(releases.release_volumes("Autor - Titel - 4CD - 2010", {}), set())


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


def volume(n, title=None, **kw):
    return {"asin": f"B0SER0000{n}", "title": title or f"Das stille Haus {n}", "authors": ["Anna Autorin"],
            "series": "Haus-Reihe", "series_asin": "B0SERIES01", "series_position": str(n), "runtime_min": 600,
            "language": "german", **kw}


class RequestLanguageVolumeTableTest(unittest.TestCase):
    """The release choice for requests, for many names and both languages – what is asked for decides."""
    EN1 = {"title": "Twilight", "authors": ["Stephenie Meyer"], "series": "The Twilight Saga", "series_position": "1",
           "runtime_min": 771, "language": "English"}
    DE1 = {"title": "Bis(s) zum Morgengrauen", "authors": ["Stephenie Meyer"], "series": "Twilight", "series_position": "1",
           "runtime_min": 812, "language": "German"}
    HP3_EN = {"title": "Harry Potter and the Prisoner of Azkaban", "authors": ["J.K. Rowling"], "series": "Harry Potter",
              "series_position": "3", "runtime_min": 720, "language": "English"}
    HP3_DE = {"title": "Harry Potter und der Gefangene von Askaban", "authors": ["J.K. Rowling"], "series": "Harry Potter",
              "series_position": "3", "runtime_min": 760, "language": "German"}

    def verdict(self, name, book, **rel):
        lang = releases.book_language(book) or "de"
        return releases.rank([{"title": name, "size": 600 << 20, **rel}], book, prefer_language=lang)[0]["verdict"]

    def test_table(self):
        cases = [
            # (request, release name, release fields, accepted?)
            (self.EN1, "Stephenie Meyer - Twilight (Unabridged) MP3", {}, True),
            (self.EN1, "Stephenie.Meyer.Twilight.1.Unabridged.M4B", {"format": "M4B"}, True),
            (self.EN1, "Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010", {"language": "de"}, False),
            (self.EN1, "Stephenie.Meyer.-.Twilight.1.-.Bis(s).zum.Morgengrauen", {}, False),          # German by its words
            (self.EN1, "Stephenie Meyer - Twilight - German - Hoerbuch", {"language": "de"}, False),
            (self.EN1, "Stephenie Meyer - Twilight 4 - Breaking Dawn - EN", {"language": "en"}, False),  # other volume
            (self.DE1, "Stephenie Meyer - Twilight 1 - Bis(s) zum Morgengrauen - Hoerbuch - DE", {"language": "de"}, True),
            (self.DE1, "Stephenie Meyer - Bis(s) zum Morgengrauen (ungekürzt)", {}, True),
            (self.DE1, "Stephenie Meyer - Twilight - Bis(s) zum Morgengrauen - ENGLISH", {"language": "en"}, False),
            (self.DE1, "Stephenie Meyer - Twilight 2 - Bis(s) zum Morgengrauen Fortsetzung", {}, False),
            (self.HP3_EN, "J.K. Rowling - Harry Potter and the Prisoner of Azkaban (Unabridged) - EN", {"language": "en"}, True),
            (self.HP3_EN, "J.K. Rowling - Harry Potter und der Gefangene von Askaban - DE", {"language": "de"}, False),
            (self.HP3_DE, "J.K. Rowling - Harry Potter 3 - Harry Potter und der Gefangene von Askaban - Hörbuch", {}, True),
            (self.HP3_DE, "J.K. Rowling - Harry Potter and the Prisoner of Azkaban - Unabridged", {}, False),
        ]
        for book, name, rel, ok in cases:
            with self.subTest(name):
                v = self.verdict(name, book, **rel)
                self.assertEqual(v["ok"], ok, (name, v["reason"]))


class MismatchTest(unittest.TestCase):
    """What a downloaded folder clearly is not: another volume, another language, a far other runtime."""
    TWILIGHT = {"title": "Twilight", "authors": ["Stephenie Meyer"], "series": "The Twilight Saga", "series_position": "1",
                "runtime_min": 771, "language": "English"}
    NAME = "Stephenie Meyer-Twilight-Saga 5 - Biss zum ersten Sonnenstrahl-4CD-mp3-DE-2010"

    def test_reasons(self):
        self.assertEqual(releases.mismatch(self.NAME, self.TWILIGHT, 276 * 60), "anderer Band (5 statt 1)")
        no_volume = {k: v for k, v in self.TWILIGHT.items() if k != "series_position"}
        self.assertEqual(releases.mismatch(self.NAME, no_volume, 276 * 60), "andere Sprache (de)")
        plain = {k: v for k, v in no_volume.items() if k != "language"}
        self.assertIn("Laufzeit", releases.mismatch(self.NAME, plain, 276 * 60))

    def test_no_evidence_no_objection(self):
        self.assertIsNone(releases.mismatch("Stephenie Meyer - Twilight", self.TWILIGHT, 770 * 60))
        self.assertIsNone(releases.mismatch("Stephenie Meyer - Twilight 01 - 04", self.TWILIGHT, 0))   # a pack
        self.assertIsNone(releases.mismatch("Twilight.1.Unabridged", self.TWILIGHT, 0))

    def test_shop_search_falls_back(self):
        import audible
        qs = audible.fallback_queries("Twilight Saga 5 Biss zum ersten Sonnenstrahl HörbucHHamburg Meyer, Stephenie",
                                      {"author": "Meyer, Stephenie"})
        self.assertIn("Biss zum ersten Sonnenstrahl Meyer", qs)
        self.assertNotIn("HörbucHHamburg", " ".join(qs))
        self.assertIn("McDonald", " ".join(audible.fallback_queries("Anne McDonald Das Haus Extra", {})))  # kept

    def test_request_blocks_the_wrong_download(self):
        RequestsTest.setUp(self)
        self.usenet.results = [GOOD]
        req = self.r.create(self.tom, BOOK)
        self.assertEqual(self.r.get(req["id"])["status"], "downloading")
        self.usenet.results = [ALSO]
        self.r.wrong_download(req["id"], GOOD["title"], "anderer Band (5 statt 1)")
        after = self.r.get(req["id"])
        self.assertIn(GOOD["title"], after["blocked"])
        self.assertEqual((after["status"], self.usenet.sent[-1][0]), ("downloading", "r2"))
        self.assertTrue(any("Falscher Inhalt" in h["text"] for h in after["history"]))

    def test_old_wrong_download_does_not_stop_the_current_one(self):
        """The old wrong folder turns up again while the request already loads another release: only block it."""
        RequestsTest.setUp(self)
        self.usenet.results = [GOOD]
        req = self.r.create(self.tom, BOOK)
        sent = len(self.usenet.sent)
        self.r.wrong_download(req["id"], "Anna Autorin - Das stille Haus 5 - Altes Release", "anderer Band (5 statt 1)")
        after = self.r.get(req["id"])
        self.assertEqual((after["status"], after["release"], len(self.usenet.sent)), ("downloading", GOOD["title"], sent))
        self.assertIn("Anna Autorin - Das stille Haus 5 - Altes Release", after["blocked"])
        self.assertNotIn(GOOD["title"], after["blocked"])
        self.r.wrong_download(req["id"], "Anna Autorin - Das stille Haus 5 - Altes Release", "anderer Band (5 statt 1)")
        self.assertEqual(len(self.r.get(req["id"])["history"]), len(after["history"]))      # said once


class SeriesRequestTest(unittest.TestCase):
    """A whole series at once: one request against the quota, approved together, fetched one after the other."""
    setUp = RequestsTest.setUp

    def series(self, user, books, title="Haus-Reihe"):
        self.r.search_group_soon = lambda ids: [self.r.search(i) for i in ids]
        return self.r.create_series(user, {"asin": "B0SERIES01", "title": title}, books)

    def test_counts_once_and_is_approved_together(self):
        self.settings["request_quota"] = 1
        self.index.owned = {"Das stille Haus 2"}
        res = self.series(self.eva, [volume(1), volume(2), volume(3), volume(4)])
        self.assertEqual(res["status"], "pending")
        self.assertEqual(res["skipped"], ["Das stille Haus 2"])                  # already in the library
        self.assertEqual([i["book"]["series_position"] for i in res["items"]], ["1", "3", "4"])
        self.assertEqual(self.notifier.sent, ["request_new"])                   # one message, not three
        self.assertEqual(self.r.quota(self.eva), {"limit": 1, "used": 1, "left": 0})
        with self.assertRaises(br.RequestError) as e:
            self.r.create(self.eva, {**BOOK, "asin": None, "title": "Noch eins"})
        self.assertEqual(e.exception.status, 429)
        self.usenet.results = [{"id": f"v{n}", "title": f"Anna.Autorin-Das.stille.Haus.{n}.German.M4B", "size": 400 << 20,
                                "language": "de"} for n in (1, 3, 4)]
        self.r.approve(res["items"][1]["id"], self.admin)                      # any volume approves the series
        states = [self.r.get(i["id"])["status"] for i in res["items"]]
        self.assertEqual(states, ["downloading"] * 3)
        self.assertEqual([s[0] for s in self.usenet.sent], ["v1", "v3", "v4"])   # each its own volume
        out = br.summary(self.r.get(res["items"][0]["id"]), self.eva, False)
        self.assertEqual((out["group"]["title"], out["group"]["count"]), ("Haus-Reihe", 3))

    def test_declined_together_and_withdrawn_together(self):
        res = self.series(self.eva, [volume(1), volume(2)])
        self.r.decline(res["items"][0]["id"], self.admin, "zu viel")
        self.assertEqual({self.r.get(i["id"])["status"] for i in res["items"]}, {"declined"})
        res = self.series(self.eva, [volume(5), volume(6)])
        with self.assertRaises(br.RequestError):
            self.r.withdraw(res["items"][0]["id"], self.tom, True)              # not Tom's
        self.assertEqual(self.r.withdraw(res["items"][0]["id"], self.eva, True), 2)
        self.assertIsNone(self.r.get(res["items"][1]["id"]))

    def test_nothing_new_is_refused(self):
        self.r.create(self.eva, volume(1))
        with self.assertRaises(br.RequestError) as e:
            self.series(self.tom, [volume(1)])
        self.assertEqual(e.exception.status, 409)
        with self.assertRaises(br.RequestError):
            self.series(self.tom, [])
        with self.assertRaises(br.RequestError):
            self.series(self.tom, [volume(n) for n in range(br.SERIES_MAX + 1)])

    def test_a_pack_is_loaded_once_for_all_volumes(self):
        self.usenet.results = [{"id": "pack", "title": "Anna Autorin - Das stille Haus 01 - 03 German", "size": 1200 << 20,
                                "language": "de"}]
        res = self.series(self.tom, [volume(1), volume(2), volume(3)])          # auto approve
        self.assertEqual(len(self.usenet.sent), 1)                               # one download, not three
        self.assertEqual(self.usenet.sent[0][1]["volumes"], [1, 2, 3])
        second = self.r.get(res["items"][1]["id"])
        self.assertEqual(second["status"], "searching")
        self.assertIn("Paket", second["history"][-1]["text"])
        self.index.owned = {"Das stille Haus 2", "Das stille Haus 3"}           # the pack arrived
        self.r.tick()
        self.assertEqual(self.r.get(res["items"][2]["id"])["status"], "available")


class SeriesLookupTest(unittest.TestCase):
    """Audible's series: released single volumes in order, one edition each, no box sets."""
    def test_volumes(self):
        def prod(asin, pos, title, **kw):
            return {"asin": asin, "title": title, "authors": [{"name": "Anna Autorin"}], "language": kw.pop("language", "german"),
                    "format_type": kw.pop("format_type", "unabridged"), "release_date": kw.pop("release_date", "2015-01-01"),
                    "series": [{"asin": "B0SERIES01", "sequence": pos, "title": "Haus-Reihe"}], **kw}
        products = [prod("B0A0000001", "2", "Zwei"), prod("B0A0000002", "1", "Eins"),
                    prod("B0A0000003", "1-2", "Sammelband"), prod("B0A0000004", "1", "Eins gekürzt", format_type="abridged"),
                    prod("B0A0000005", "3", "Drei", language="english"), prod("B0A0000006", "4", "Vier", release_date="2999-01-01")]
        rel = [{"asin": p["asin"], "relationship_type": "series", "relationship_to_product": "child", "sort": str(i)}
               for i, p in enumerate(products)]

        def get(url):
            if "/products/B0SERIES01" in url:
                return {"product": {"asin": "B0SERIES01", "title": "Haus-Reihe", "relationships": rel}}
            return {"products": products}
        import audible
        res = audible.AudibleClient(get=get).series("B0SERIES01", "de", "german")
        self.assertEqual(res["title"], "Haus-Reihe")
        self.assertEqual([(b["series_position"], b["title"]) for b in res["books"]], [("1", "Eins"), ("2", "Zwei")])
        self.assertEqual(res["books"][0]["series_asin"], "B0SERIES01")


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
        status, lst = self.req("Admin", "GET", "/api/requests")                # admins always see all
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

    def test_series_through_the_web(self):
        self.assertEqual(self.req("Eva", "POST", "/api/search/series", {"asin": "../../x", "region": "de"})[0], 400)
        self.assertEqual(self.req("Eva", "POST", "/api/search/series", {"asin": "B0SERIES01", "region": "mars"})[0], 400)
        status, res = self.req("Eva", "POST", "/api/requests/series", {"series": {"asin": "B0WEBSER01", "title": "Web-Reihe"},
                                                                     "books": [volume(7, "Web 7"), volume(8, "Web 8")]})
        self.assertEqual((status, res["status"], len(res["items"])), (200, "pending", 2))
        self.assertEqual(self.req("Eva", "POST", "/api/requests/series", {"series": {}, "books": "alles"})[0], 400)
        status, lst = self.req("Admin", "GET", "/api/requests")
        mine = [i for i in lst["items"] if (i.get("group") or {}).get("title") == "Web-Reihe"]
        self.assertEqual(len(mine), 2)
        self.assertEqual(self.req("Tom", "DELETE", f"/api/requests/{mine[0]['id']}?group=1")[0], 404)
        status, out = self.req("Eva", "DELETE", f"/api/requests/{mine[0]['id']}?group=1")
        self.assertEqual((status, out["deleted"]), (200, 2))

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
