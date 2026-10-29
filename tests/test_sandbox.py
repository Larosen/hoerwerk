"""ffmpeg jail: Landlock filesystem and network rules, limits, command lines."""
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import sandbox  # noqa: E402
from media import tone  # noqa: E402

ABI = sandbox.landlock_abi()
CAT = shutil.which("cat")


def jailed(cmd, read=(), write=()):
    argv = [sys.executable, "-I", "-S", sandbox.HERE]
    for p in read:
        argv += ["--read", p]
    for p in write:
        argv += ["--write", p]
    return subprocess.run(argv + ["--", *cmd], capture_output=True, text=True, timeout=60)


class CommandTest(unittest.TestCase):
    @unittest.skipIf(sandbox.MODE == "off", "FFMPEG_SANDBOX=off")
    def test_launcher_argv(self):
        cmd = sandbox.command("ffmpeg", ["-i", "a.mp3"], read=["/in/a.mp3"], write=["/work"], nice="10")
        self.assertEqual(cmd[:4], [sys.executable, "-I", "-S", sandbox.HERE])
        self.assertEqual(cmd[4:], ["--read", "/in/a.mp3", "--write", "/work", "--nice", "10", "--",
                                   "ffmpeg", "-i", "a.mp3"])

    def test_bad_launcher_arguments(self):
        self.assertEqual(sandbox.main(["--evil", "x", "--", "true"]), 2)
        self.assertEqual(sandbox.main(["--read", "/x"]), 2)


@unittest.skipUnless(ABI and CAT, "Landlock nicht verfügbar")
class JailTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="jail-")
        for d in ("in", "work", "config"):
            os.makedirs(os.path.join(self.root, d))
        self.secret = os.path.join(self.root, "config", "keys.txt")
        with open(self.secret, "w") as fh:
            fh.write("TOPSECRET")
        self.allowed = os.path.join(self.root, "in", "a.txt")
        with open(self.allowed, "w") as fh:
            fh.write("ok")

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_only_given_files_are_readable(self):
        res = jailed([CAT, self.allowed], read=[self.allowed])
        self.assertEqual(res.stdout, "ok")
        res = jailed([CAT, self.secret], read=[self.allowed])
        self.assertNotEqual(res.returncode, 0)
        self.assertNotIn("TOPSECRET", res.stdout)
        res = jailed([CAT, "/etc/hostname"], read=[self.allowed])
        self.assertNotEqual(res.returncode, 0)

    def test_writes_only_into_the_work_folder(self):
        sh = shutil.which("sh")
        ok = os.path.join(self.root, "work", "out.txt")
        bad = os.path.join(self.root, "config", "evil.txt")
        jailed([sh, "-c", f"echo x > '{ok}'; echo x > '{bad}'"], write=[os.path.join(self.root, "work")])
        self.assertTrue(os.path.exists(ok))
        self.assertFalse(os.path.exists(bad))

    @unittest.skipUnless(ABI >= 4, "Landlock ohne Netzwerkregeln (Kernel < 6.7)")
    def test_no_tcp_connections(self):
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        port = srv.getsockname()[1]
        threading.Thread(target=lambda: srv.accept(), daemon=True).start()
        code = f"import socket; socket.create_connection(('127.0.0.1', {port}), timeout=3); print('CONNECTED')"
        # this Python may live outside the system folders (CI: /opt/hostedtoolcache) – let it load itself
        res = jailed([sys.executable, "-I", "-c", code], read=[sys.prefix, sys.base_prefix])
        srv.close()
        self.assertNotIn("CONNECTED", res.stdout)
        self.assertIn("PermissionError", res.stderr)

    @unittest.skipUnless(shutil.which("ffprobe"), "ffmpeg fehlt")
    def test_ffprobe_cannot_open_config(self):
        probe = shutil.which("ffprobe")
        free = subprocess.run([probe, "-v", "error", self.secret], capture_output=True, text=True, timeout=30)
        self.assertNotIn("Permission denied", free.stderr)          # without the jail it can open it
        res = jailed([probe, "-v", "error", self.secret], read=[self.allowed])
        self.assertIn("Permission denied", res.stderr)

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg fehlt")
    def test_real_ffmpeg_call_works_inside(self):
        src = tone(os.path.join(self.root, "in", "t.mp3"), 1.0)
        out = os.path.join(self.root, "work", "t.aac")
        cmd = sandbox.command(shutil.which("ffmpeg"), ["-nostdin", "-v", "error", "-y", "-i", src, "-c:a", "aac",
                                                       "-f", "adts", out], read=[src], write=[os.path.dirname(out)])
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertGreater(os.path.getsize(out), 100)


if __name__ == "__main__":
    unittest.main()
