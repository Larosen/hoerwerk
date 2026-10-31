"""Clients that hang up are not errors; real handler crashes still reach the log."""
import io
import os
import socket
import struct
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import applog  # noqa: E402
from web import server  # noqa: E402


class HangUpTest(unittest.TestCase):
    def setUp(self):
        self.srv = server.Server(("127.0.0.1", 0), app=None)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.port = self.srv.server_address[1]
        self.stderr, sys.stderr = sys.stderr, io.StringIO()
        self.mark = applog.log.seq

    def tearDown(self):
        sys.stderr = self.stderr
        self.srv.shutdown()
        self.srv.server_close()

    def test_reset_connection_is_quiet(self):
        s = socket.create_connection(("127.0.0.1", self.port))
        s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))   # close with RST
        s.send(b"GE")
        time.sleep(0.1)
        s.close()
        time.sleep(0.3)
        self.assertNotIn("Traceback", sys.stderr.getvalue())
        self.assertEqual([e for e in applog.log.since(self.mark) if e["level"] == "error"], [])

    def test_other_errors_are_logged(self):
        try:
            raise ValueError("kaputt")
        except ValueError:
            self.srv.handle_error(None, ("10.0.0.5", 1234))
        errors = [e for e in applog.log.since(self.mark) if e["level"] == "error"]
        self.assertEqual(len(errors), 1)
        self.assertIn("ValueError('kaputt')", errors[0]["text"])
        self.assertIn("10.0.0.5", errors[0]["text"])


if __name__ == "__main__":
    unittest.main()
