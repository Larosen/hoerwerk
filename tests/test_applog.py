"""The app log: levels, newest-first reading, stderr captured, still printed for docker logs."""
import io
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import applog  # noqa: E402


class AppLogTest(unittest.TestCase):
    def test_levels_and_reading_on(self):
        out = io.StringIO()
        log = applog.AppLog(size=3, out=out)
        log.add("eins", source="Buch")
        log.add("zwei", "warn")
        last = log.add("drei", "error")
        log.add("vier")
        self.assertEqual([e["text"] for e in log.since()], ["zwei", "drei", "vier"])      # only the newest 3 kept
        self.assertEqual([e["text"] for e in log.since(level="warn")], ["zwei", "drei"])
        self.assertEqual([e["text"] for e in log.since(after=last["id"])], ["vier"])
        self.assertIn("INFO  [Buch] eins", out.getvalue())                                # docker logs
        self.assertIn("ERROR drei", log.text())

    def test_stderr_is_captured_as_error(self):
        log = applog.AppLog(out=io.StringIO())
        real = io.StringIO()
        err = applog._Stderr(real, log)
        err.write("Traceback (most recent call last):\n  File \"x\"")
        err.write(", line 1\n")
        self.assertEqual([(e["level"], e["text"]) for e in log.since()],
                         [("error", "Traceback (most recent call last):"), ("error", '  File "x", line 1')])
        self.assertIn("Traceback", real.getvalue())


if __name__ == "__main__":
    unittest.main()
