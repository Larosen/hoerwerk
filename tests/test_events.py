"""Live updates: event bus and the /api/events stream."""
import http.client
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import config  # noqa: E402
import services  # noqa: E402
from events import EventBus  # noqa: E402
from web import server  # noqa: E402


def read_events(resp, count, timeout=5.0):
    """The next ``count`` SSE events of a response as (event, data)."""
    out, event, data = [], None, []
    resp.fp.raw._sock.settimeout(timeout)
    while len(out) < count:
        line = resp.fp.readline().decode().rstrip("\n")
        if line.startswith("event: "):
            event = line[7:]
        elif line.startswith("data: "):
            data.append(line[6:])
        elif line == "" and event:
            out.append((event, json.loads("\n".join(data))))
            event, data = None, []
    return out


class BusTest(unittest.TestCase):
    def test_changes_are_coalesced(self):
        bus = EventBus(interval=0.2)
        calls = []
        bus.provide("t", lambda: calls.append(1) or {"n": len(calls)})
        stream = bus.stream(["t"], heartbeat=5)
        it = iter(stream)
        self.assertEqual(next(it), b"retry: 3000\n\n")
        self.assertIn(b'"n":1', next(it))
        for _ in range(50):                  # a burst of progress ticks …
            bus.publish("t")
        self.assertIn(b'"n":2', next(it))    # … is one update
        stream.close()
        self.assertEqual(bus.clients, 0)

    def test_snapshot_is_shared_between_clients(self):
        bus = EventBus(interval=0)
        calls = []
        bus.provide("t", lambda: calls.append(1) or {})
        a, b = iter(bus.stream(["t"])), iter(bus.stream(["t"]))
        next(a), next(a), next(b), next(b)
        self.assertEqual(len(calls), 1)

    def test_client_limit_and_close(self):
        bus = EventBus(max_clients=1)
        bus.provide("t", dict)
        s = bus.stream(["t"])
        with self.assertRaises(OverflowError):
            bus.stream(["t"])
        it = iter(s)
        next(it), next(it)
        bus.close()
        self.assertEqual(list(it), [])        # closing the bus ends every stream


class StreamTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="sse-")
        env = {k: os.path.join(cls.tmp, v) for k, v in
               {"DATA_DIR": "data", "CONFIG_DIR": "config", "INBOX_DIR": "inbox", "LIBRARY_DIR": "lib"}.items()}
        for d in env.values():
            os.makedirs(d, exist_ok=True)
        cls.app = services.App(config.Config.from_env({**env, "AUDIOBOOK_WATCH": "false"}), start_workers=False)
        cls.app.events.interval = 0.05
        cls.server = server.make_server(cls.app, "127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.app.events.close()
        cls.server.shutdown()
        cls.server.server_close()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def open(self, path):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.request("GET", path, headers={"Host": f"127.0.0.1:{self.port}"})
        return c, c.getresponse()

    def test_initial_state_then_changes(self):
        c, r = self.open("/api/events?topics=jobs,audiobooks")
        try:
            self.assertEqual(r.status, 200)
            self.assertEqual(r.getheader("Content-Type"), "text/event-stream; charset=utf-8")
            self.assertEqual(r.getheader("X-Frame-Options"), "DENY")
            first = dict(read_events(r, 2))
            self.assertEqual(first["jobs"], [])
            self.assertIn("queues", first["audiobooks"])
            # a new conversion job is pushed without asking
            folder = os.path.join(self.tmp, "data", "Buch")
            os.makedirs(folder, exist_ok=True)
            open(os.path.join(folder, "a.mp3"), "wb").close()
            self.app.converter.enqueue(folder, "Buch", "Test")
            [(event, jobs)] = read_events(r, 1)
            self.assertEqual(event, "jobs")
            self.assertEqual(jobs[0]["folder"], "Buch")
        finally:
            c.close()

    def test_unknown_topic(self):
        c, r = self.open("/api/events?topics=secrets")
        self.assertEqual(r.status, 400)
        c.close()

    def test_closed_tab_frees_the_slot(self):
        before = self.app.events.clients
        c, r = self.open("/api/events?topics=library")
        read_events(r, 1)
        self.assertEqual(self.app.events.clients, before + 1)
        r.close()
        c.close()
        self.app.events.publish("library")       # the next write notices the closed socket
        deadline = time.time() + 5
        while self.app.events.clients > before and time.time() < deadline:
            self.app.events.publish("library")
            time.sleep(0.05)
        self.assertEqual(self.app.events.clients, before)


if __name__ == "__main__":
    unittest.main()
