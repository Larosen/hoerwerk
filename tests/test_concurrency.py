"""Database, engine and pipeline under parallel load and after a crash: consistent, nothing lost or done twice."""
import errno
import os
import shutil
import sqlite3
import sys
import tempfile
import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import db as dbm  # noqa: E402
import library  # noqa: E402
import pipeline  # noqa: E402
from engine import Engine  # noqa: E402


class DatabaseTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.db = dbm.Database(os.path.join(self.dir, "t.db"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.addCleanup(self.db.close)

    def test_parallel_operations_stay_consistent(self):
        self.db.put("counter", 0)

        def work(i):
            with self.db.tx() as c:                        # read-modify-write must not lose an update
                n = self.db.get("counter")
                c.execute("INSERT INTO audit(time, user, ip, action) VALUES(?,?,?,?)", (time.time(), f"u{i}", "", "x"))
                self.db.put("counter", n + 1)
            if i % 5 == 0:
                try:
                    with self.db.tx() as c:
                        c.execute("INSERT INTO audit(time, user, ip, action) VALUES(?,?,?,?)", (0, "lost", "", "x"))
                        raise RuntimeError("boom")             # this one is rolled back completely
                except RuntimeError:
                    pass
            return self.db.get("counter")
        with ThreadPoolExecutor(64) as pool:
            list(pool.map(work, range(200)))
        self.assertEqual(self.db.get("counter"), 200)
        self.assertEqual(self.db.one("SELECT COUNT(*) AS n FROM audit WHERE user!='lost'")["n"], 200)
        self.assertEqual(self.db.one("SELECT COUNT(*) AS n FROM audit WHERE user='lost'")["n"], 0)
        self.assertFalse(self.db.conn.in_transaction)

    def test_failed_commit_does_not_leave_a_transaction_open(self):
        real = self.db.conn

        class Flaky:                                       # COMMIT fails once (disk full, I/O error …)
            failed = False

            def execute(self, sql, *args):
                if sql == "COMMIT" and not Flaky.failed:
                    Flaky.failed = True
                    raise sqlite3.OperationalError("disk I/O error")
                return real.execute(sql, *args)

            def __getattr__(self, name):
                return getattr(real, name)
        self.db.conn = Flaky()
        with self.assertRaises(sqlite3.OperationalError):
            self.db.put("a", 1)
        self.assertFalse(real.in_transaction)              # rolled back, not dangling
        self.db.put("b", 2)                                 # the next write is really saved
        self.db.conn = real
        other = dbm.Database(os.path.join(self.dir, "t.db"))
        self.assertEqual((other.get("a"), other.get("b")), (None, 2))
        other.close()

    def test_migrations_on_an_existing_database(self):
        path = os.path.join(self.dir, "old.db")
        conn = sqlite3.connect(path)
        conn.executescript(dbm.MIGRATIONS[0] + "; PRAGMA user_version=1;")
        conn.execute("INSERT INTO kv(key, value) VALUES('settings', '{\"language\":\"en\"}')")
        conn.commit()
        conn.close()
        d = dbm.Database(path)
        self.assertEqual(d.one("PRAGMA user_version")[0], len(dbm.MIGRATIONS))
        self.assertEqual(d.get("settings"), {"language": "en"})                 # data kept
        self.assertIsNotNone(d.one("SELECT origin, last_seen FROM users LIMIT 1") or True)
        d.close()
        d = dbm.Database(path)                                                    # and opening again changes nothing
        self.assertEqual(d.one("PRAGMA user_version")[0], len(dbm.MIGRATIONS))
        d.close()

    def test_snapshot_is_consistent_and_daily(self):
        import services
        import config
        cfg_dir = os.path.join(self.dir, "cfg")
        app = services.App(config.Config.from_env({"CONFIG_DIR": cfg_dir, "INBOX_DIR": self.dir,
                                                   "LIBRARY_DIR": self.dir, "AUDIOBOOK_WATCH": "false"}),
                           start_workers=False)
        app.db.put("probe", {"x": 1})
        target = app.maintain_db(now=1_800_000_000)
        self.assertTrue(target and os.path.isfile(target))
        self.assertEqual(oct(os.stat(target).st_mode & 0o777), "0o600")
        copy = dbm.Database(target)
        self.assertEqual(copy.get("probe"), {"x": 1})
        copy.close()
        self.assertIsNone(app.maintain_db(now=1_800_000_000 + 3600))           # once a day
        app.db.close()


class EngineTest(unittest.TestCase):
    def test_steps_of_one_job_never_overlap(self):
        e = Engine({"encode": 2, "io": 4})
        e.start()
        running, overlaps, runs = {}, [], []
        lock = threading.Lock()

        def step(key):
            with lock:
                if running.get(key):
                    overlaps.append(key)
                running[key] = True
                runs.append(key)
            time.sleep(0.01)
            with lock:
                running[key] = False
        keys = [f"job{i % 10}" for i in range(100)]
        for k in keys:                                      # the same jobs queued again and again, both queues
            e.submit("io" if len(runs) % 2 else "encode", k, lambda k=k: step(k))
            e.submit("io", k, lambda k=k: step(k))
        self.assertTrue(e.wait_idle(30))
        self.assertEqual(overlaps, [])
        self.assertGreaterEqual(len(runs), 10)


class PipelineCrashTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="crash-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.inbox, self.lib, self.work = (os.path.join(self.root, n) for n in ("inbox", "lib", "inbox/.work"))
        for d in (self.inbox, self.lib, self.work):
            os.makedirs(d, exist_ok=True)
        self.db = dbm.Database(os.path.join(self.root, "t.db"))
        self.addCleanup(self.db.close)
        self.settings = {"ab_originals": "archive", "write_abs": False}

    def pipe(self):
        return pipeline.Pipeline(lambda: self.settings, self.db, self.inbox, self.lib, self.work)

    def test_one_job_runs_once_even_when_started_twice(self):
        p = self.pipe()
        src = os.path.join(self.inbox, "Buch")
        os.makedirs(src)
        job = pipeline.Job(source=src)
        p.jobs.append(job)
        calls = []

        def prepare(j):
            calls.append(j.id)
            time.sleep(0.2)
        p._prepare = prepare
        p._match = lambda j: setattr(j, "match", {"title": "x"})
        p._skip_owned_volume = lambda j: True
        barrier = threading.Barrier(8)

        def go():
            barrier.wait()
            p.step(job)
        threads = [threading.Thread(target=go) for _ in range(8)]
        for th in threads:
            th.start()
        for th in threads:
            th.join()
        self.assertEqual(calls, [job.id])

    def crashed_job(self, placed: bool):
        """A job that was filing when the process died: old copy parked, new M4B either in place or not."""
        old = os.path.join(self.lib, "Storm", "Schimmelreiter")
        os.makedirs(old)
        with open(os.path.join(old, "alt.mp3"), "wb") as fh:
            fh.write(b"ID3old")
        p = self.pipe()
        src = os.path.join(self.inbox, "Schimmelreiter")
        os.makedirs(src)
        job = pipeline.Job(source=src, status="tagging", stage="finish", match={"title": "Der Schimmelreiter"})
        job.m4b = os.path.join(self.work, job.id, "book.m4b")
        os.makedirs(os.path.dirname(job.m4b))
        with open(job.m4b, "wb") as fh:
            fh.write(b"\0" * 1000)
        parked = p._park(job, "Storm/Schimmelreiter", "archive")
        folder = os.path.join(self.lib, "Storm", "Der Schimmelreiter")
        os.makedirs(folder)
        p.jobs.append(job)
        p._journal(job, [parked], folder, "Der Schimmelreiter.m4b")
        with open(os.path.join(folder, library.PARTIAL + "Der Schimmelreiter.m4b"), "wb") as fh:
            fh.write(b"\0" * 10)                                   # a half copy from the crash
        if placed:
            os.replace(job.m4b, os.path.join(folder, "Der Schimmelreiter.m4b"))
        return job, old, folder

    def test_restart_after_the_book_was_placed(self):
        job, old, folder = self.crashed_job(placed=True)
        again = self.pipe().get(job.id)                            # the restart
        self.assertEqual(again.status, "done", again.log)
        self.assertEqual(again.dest, os.path.join(folder, "Der Schimmelreiter.m4b"))
        self.assertFalse(os.path.exists(old))
        self.assertTrue(os.path.isfile(os.path.join(self.inbox, "_ersetzt", "Storm", "Schimmelreiter", "alt.mp3")))
        self.assertEqual(os.listdir(folder), ["Der Schimmelreiter.m4b"])          # half copy gone
        self.assertIsNone(again.filing)
        self.assertEqual(self.pipe().get(job.id).status, "done")                  # and it stays so

    def test_restart_before_the_book_was_placed(self):
        job, old, folder = self.crashed_job(placed=False)
        again = self.pipe().get(job.id)
        self.assertEqual((again.status, again.stage), ("queued", "finish"))      # filing starts over
        self.assertTrue(os.path.isfile(os.path.join(old, "alt.mp3")))             # the old copy is back
        self.assertEqual([n for n in os.listdir(os.path.dirname(old)) if n.startswith(".")], [])
        self.assertEqual(os.listdir(folder), [])
        self.assertTrue(os.path.isfile(again.m4b))


class AtomicMoveTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root, True)
        self.src = os.path.join(self.root, "a.m4b")
        with open(self.src, "wb") as fh:
            fh.write(b"x" * 5000)

    def cross_device(self):
        real = os.rename

        def rename(a, b):
            raise OSError(errno.EXDEV, "cross-device link")
        library.os.rename = rename
        self.addCleanup(setattr, library.os, "rename", real)

    def test_other_disk_copies_then_renames(self):
        self.cross_device()
        dest = os.path.join(self.root, "lib.m4b")
        library.move_atomic(self.src, dest)
        self.assertEqual(os.path.getsize(dest), 5000)
        self.assertFalse(os.path.exists(self.src))
        self.assertEqual(sorted(os.listdir(self.root)), ["lib.m4b"])

    def test_failed_copy_leaves_no_half_book(self):
        self.cross_device()
        real = library.shutil.copy2

        def broken(a, b):
            with open(b, "wb") as fh:
                fh.write(b"half")
            raise OSError(errno.ENOSPC, "no space")
        library.shutil.copy2 = broken
        self.addCleanup(setattr, library.shutil, "copy2", real)
        with self.assertRaises(OSError):
            library.move_atomic(self.src, os.path.join(self.root, "lib.m4b"))
        self.assertEqual(sorted(os.listdir(self.root)), ["a.m4b"])              # source kept, nothing half-done


if __name__ == "__main__":
    unittest.main()


class RunnerCancelTest(unittest.TestCase):
    """Review 7.13: a cancel between starting ffmpeg and noting the process down left it running (up to 6 h)."""

    def test_cancel_while_the_process_starts(self):
        import encoder
        import subprocess
        flag = {"cancel": False}
        runner = encoder.Runner(lambda: flag["cancel"], timeout=60)
        real = subprocess.Popen

        def popen_then_cancel(*args, **kw):
            proc = real(["sleep", "30"], **{k: v for k, v in kw.items() if k != "text"}, text=True)
            flag["cancel"] = True                     # the user cancels exactly now …
            runner.kill()                             # … before run() has registered the process
            return proc
        encoder.subprocess.Popen = popen_then_cancel
        self.addCleanup(setattr, encoder.subprocess, "Popen", real)
        t0 = time.monotonic()
        with self.assertRaises(encoder.Cancelled):
            runner.run(["ffmpeg", "-version"])
        self.assertLess(time.monotonic() - t0, 5)     # killed at once, not after the sleep or the timeout

    def test_nothing_starts_after_a_kill(self):
        import encoder
        runner = encoder.Runner(lambda: False)
        runner.kill()
        with self.assertRaises(encoder.Cancelled):
            runner.run(["sleep", "30"])

    def test_kill_reaches_the_whole_process_group(self):
        import encoder
        import subprocess
        runner = encoder.Runner(lambda: False, timeout=60)
        marker = os.path.join(tempfile.mkdtemp(), "child-alive")
        # a wrapper that starts a child and waits – like a jail launcher in front of ffmpeg
        script = f"sleep 30 & echo $! > {marker}; wait"
        th = threading.Thread(target=lambda: self.assertRaises(encoder.Cancelled, runner.run, ["sh", "-c", script]))
        th.start()
        for _ in range(100):
            if os.path.exists(marker) and open(marker).read().strip():
                break
            time.sleep(0.05)
        child = int(open(marker).read().strip())
        runner.kill()
        th.join(5)
        self.assertFalse(th.is_alive())
        time.sleep(0.2)
        try:                                          # the grandchild is gone too (or a zombie waiting for init)
            with open(f"/proc/{child}/stat") as fh:
                state = fh.read().rsplit(")", 1)[1].split()[0]
        except FileNotFoundError:
            state = "gone"
        self.assertIn(state, ("gone", "Z", "X"))
        del subprocess


class LibraryScanRaceTest(unittest.TestCase):
    """Review 7.13: a book filed while a scan walked the disk vanished from the index (and the database)."""

    def setUp(self):
        import library_index
        self.li = library_index
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root, True)
        self.db = dbm.Database(os.path.join(self.root, "t.db"))
        self.addCleanup(self.db.close)
        self.lib = os.path.join(self.root, "lib")
        self.book("Z/Old")
        self.idx = library_index.LibraryIndex(self.lib, self.db)
        self.idx.scan()

    def book(self, rel):
        os.makedirs(os.path.join(self.lib, rel), exist_ok=True)
        path = os.path.join(self.lib, rel, "book.mp3")
        with open(path, "wb") as fh:
            fh.write(b"ID3")
        return path

    def scan_paused(self, during):
        """Run a scan whose walk is frozen after reading the disk; ``during`` runs in that gap."""
        real = self.li.os.walk
        reached, go = threading.Event(), threading.Event()

        def frozen(top, *a, **kw):
            seen = list(real(top, *a, **kw))     # the picture of the disk …
            reached.set()
            go.wait(10)                           # … is old by the time it is used
            yield from seen
        self.li.os.walk = frozen
        self.addCleanup(setattr, self.li.os, "walk", real)
        th = threading.Thread(target=self.idx.scan)
        th.start()
        reached.wait(10)
        during()
        go.set()
        th.join(10)

    def test_book_filed_during_a_scan_stays(self):
        def file_new():
            self.idx.add(self.book("A/New"), {"title": "New", "authors": ["Anna Autorin"]})
        self.scan_paused(file_new)
        self.assertEqual(sorted(self.idx.entries), ["A/New", "Z/Old"])
        stored = {r["path"] for r in self.db.all("SELECT path FROM library")}
        self.assertEqual(stored, {"A/New", "Z/Old"})

    def test_book_removed_during_a_scan_stays_removed(self):
        def remove_old():
            shutil.rmtree(os.path.join(self.lib, "Z"))
            self.idx.remove(os.path.join(self.lib, "Z/Old", "book.mp3"))
        self.scan_paused(remove_old)
        self.assertEqual(sorted(self.idx.entries), [])
        self.assertEqual(self.db.all("SELECT path FROM library"), [])
