"""Job engine: separate queues, audiobook stages and folder conversions through the engine."""
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import db as dbm  # noqa: E402
import pipeline  # noqa: E402
from engine import Engine  # noqa: E402
from media import tone  # noqa: E402
from test_pipeline import FakeClient, product, tree  # noqa: E402

HAVE_FFMPEG = bool(shutil.which("ffmpeg"))


class EngineTest(unittest.TestCase):
    def test_io_does_not_wait_for_encode(self):
        e = Engine({"encode": 1, "io": 1})
        e.start()
        release = threading.Event()
        order = []
        e.submit("encode", "long", lambda: (release.wait(5), order.append("encode")))
        e.submit("encode", "next", lambda: order.append("encode2"))
        e.submit("io", "quick", lambda: order.append("io"))
        deadline = time.time() + 5
        while "io" not in order and time.time() < deadline:
            time.sleep(0.01)
        self.assertEqual(order, ["io"])                 # io ran while the conversion was still busy
        self.assertEqual(e.stats()["encode"], {"workers": 1, "running": 1, "waiting": 1})
        release.set()
        self.assertTrue(e.wait_idle(5))
        self.assertEqual(order, ["io", "encode", "encode2"])

    def test_same_key_is_queued_once_and_can_be_discarded(self):
        e = Engine()
        self.assertTrue(e.submit("io", "a", lambda: None))
        self.assertFalse(e.submit("io", "a", lambda: None))
        self.assertTrue(e.is_waiting("a"))
        e.discard("a")
        self.assertFalse(e.is_waiting("a"))
        with self.assertRaises(ValueError):
            e.submit("gpu", "x", lambda: None)

    def test_failing_step_does_not_kill_the_worker(self):
        e = Engine({"io": 1})
        e.start()
        done = []
        e.submit("io", "bad", lambda: 1 / 0)
        e.submit("io", "good", lambda: done.append(1))
        self.assertTrue(e.wait_idle(5))
        self.assertEqual(done, [1])


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg fehlt")
class PipelineEngineTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="eng-")
        self.db = dbm.Database(os.path.join(self.root, "t.db"))
        self.inbox = os.path.join(self.root, "inbox")
        self.lib = os.path.join(self.root, "lib")
        os.makedirs(self.inbox)
        os.makedirs(self.lib)
        self.settings = {"ab_region": "de", "ab_match": "auto", "ab_threshold": 0.8, "ab_chapters": "files",
                         "ab_originals": "archive"}
        self.engine = Engine({"encode": 1, "io": 2})
        self.engine.start()
        self.p = pipeline.Pipeline(lambda: self.settings, self.db, self.inbox, self.lib,
                                   os.path.join(self.inbox, ".work"), engine=self.engine, client=FakeClient([
                                       product("B0STORM001", "Der Schimmelreiter", ["Theodor Storm"], 2.0),
                                       product("B0RAETSEL1", "Das Rätsel", ["Max Muster"], 2.0),
                                       product("B0RAETSEL2", "Das Rätsel", ["Eva Andere"], 2.0)]))

    def tearDown(self):
        self.assertTrue(self.engine.wait_idle(30))
        shutil.rmtree(self.root, ignore_errors=True)

    def test_inbox_to_library_through_the_queues(self):
        tone(os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter", "01.mp3"), 1.0)
        tone(os.path.join(self.inbox, "Theodor Storm - Der Schimmelreiter", "02.mp3"), 1.0)
        [job] = self.p.scan_once(settle=0)
        self.assertTrue(self.engine.wait_idle(30))
        self.assertEqual(job.status, "done", job.log)
        self.assertEqual(tree(self.lib)[0], "Theodor Storm/Der Schimmelreiter/Der Schimmelreiter.m4b")
        # Audiobookshelf reads metadata.json next to the book
        with open(os.path.join(self.lib, "Theodor Storm", "Der Schimmelreiter", "metadata.json"), encoding="utf-8") as fh:
            meta = json.load(fh)
        self.assertEqual((meta["title"], meta["authors"], meta["asin"]), ("Der Schimmelreiter", ["Theodor Storm"], "B0STORM001"))
        self.assertEqual(len(meta["chapters"]), 2)
        self.assertAlmostEqual(meta["chapters"][-1]["end"], 2.0, delta=0.2)

    def test_choose_is_filed_while_another_book_converts(self):
        tone(os.path.join(self.inbox, "Das Rätsel.mp3"), 2.0)
        [review] = self.p.scan_once(settle=0)
        self.assertTrue(self.engine.wait_idle(30))
        self.assertEqual(review.status, "review")
        # a long conversion blocks the encode queue …
        release = threading.Event()
        self.engine.submit("encode", "long-book", lambda: release.wait(10))
        time.sleep(0.1)
        # … and choosing a match still files the reviewed book right away (io queue)
        self.p.choose(review.id, "B0RAETSEL1", "de")
        deadline = time.time() + 10
        while review.status != "done" and time.time() < deadline:
            time.sleep(0.05)
        self.assertEqual(review.status, "done", review.log)
        self.assertEqual(self.engine.stats()["encode"]["running"], 1)
        release.set()

    def test_restart_requeues_into_the_right_stage(self):
        tone(os.path.join(self.inbox, "Das Rätsel.mp3"), 2.0)
        [job] = self.p.scan_once(settle=0)
        self.assertTrue(self.engine.wait_idle(30))
        self.assertEqual((job.status, job.stage), ("review", "finish"))
        self.p.choose(job.id, "B0RAETSEL2", "de")
        self.assertTrue(self.engine.wait_idle(30))
        self.assertEqual(job.status, "done", job.log)
        again = pipeline.Pipeline(lambda: self.settings, self.db, self.inbox, self.lib,
                                  os.path.join(self.inbox, ".work"))
        self.assertEqual((again.get(job.id).status, again.get(job.id).stage), ("done", "finish"))

    def test_cancel_while_waiting_drops_the_step(self):
        release = threading.Event()
        self.engine.submit("io", "block1", lambda: release.wait(10))
        self.engine.submit("io", "block2", lambda: release.wait(10))
        tone(os.path.join(self.inbox, "Der Schimmelreiter.mp3"), 1.0)
        [job] = self.p.scan_once(settle=0)
        self.assertTrue(self.engine.is_waiting(job.id))
        self.p.cancel(job.id)
        self.assertFalse(self.engine.is_waiting(job.id))
        release.set()
        self.assertTrue(self.engine.wait_idle(10))
        self.assertEqual(job.status, "cancelled")



if __name__ == "__main__":
    unittest.main()
