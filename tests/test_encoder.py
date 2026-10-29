import os
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))
sys.path.insert(0, os.path.dirname(__file__))

import encoder  # noqa: E402
from media import tone  # noqa: E402


def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                         capture_output=True, text=True).stdout
    return float(out)


class ChaptersTextTest(unittest.TestCase):
    def test_mp4chaps(self):
        ch = encoder.parse_chapters_txt("00:00:00.000 Prolog\n00:01:02.500 Kapitel 1\n1:02:03 Kapitel 2\n")
        self.assertEqual([c["start"] for c in ch], [0.0, 62.5, 3723.0])
        self.assertEqual(ch[1]["title"], "Kapitel 1")

    def test_ogm(self):
        ch = encoder.parse_chapters_txt("CHAPTER02=00:10:00.000\nCHAPTER02NAME=Zwei\nCHAPTER01=00:00:00.000\nCHAPTER01NAME=Eins\n")
        self.assertEqual([(c["start"], c["title"]) for c in ch], [(0.0, "Eins"), (600.0, "Zwei")])

    def test_roundtrip(self):
        ch = [{"start": 0.0, "title": "A"}, {"start": 3725.25, "title": "B"}]
        self.assertEqual(encoder.parse_chapters_txt(encoder.format_chapters_txt(ch)), ch)

    def test_chapter_title(self):
        self.assertEqual(encoder.chapter_title("CD1/01 - Der Anfang.mp3", {}, "auto"), "Der Anfang")
        self.assertEqual(encoder.chapter_title("01.mp3", {"title": "Track 01"}, "auto"), "01")
        self.assertEqual(encoder.chapter_title("01.mp3", {"title": "Im Wald"}, "auto"), "Im Wald")
        self.assertEqual(encoder.chapter_title("07 Im Wald.mp3", {"title": "x"}, "filename"), "07 Im Wald")
        self.assertEqual(encoder.chapter_title("07_Im_Wald.mp3", {}, "auto"), "Im Wald")
        self.assertEqual(encoder.chapter_title("1 Kapitel.mp3", {}, "auto"), "1 Kapitel")
        self.assertTrue(encoder.looks_generic([{"title": "Track 1"}, {"title": "Track 2"}]))
        self.assertFalse(encoder.looks_generic([{"title": "Prolog"}, {"title": "Track 2"}]))

    def test_bitrate(self):
        self.assertEqual(encoder.pick_bitrate("auto", [{"bit_rate": 320000}], 2), "128k")
        self.assertEqual(encoder.pick_bitrate("auto", [{"bit_rate": 64000}], 1), "64k")
        self.assertEqual(encoder.pick_bitrate("auto", [{"bit_rate": 32000}], 1), "32k")
        self.assertEqual(encoder.pick_bitrate("96k", [{"bit_rate": 320000}], 2), "96k")
        self.assertEqual(encoder.pick_rate([{"sample_rate": 22050}, {"sample_rate": 22050}, {"sample_rate": 96000}]), 22050)


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg fehlt")
class MergeTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="enc-")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_collect_audio_natural_and_nested(self):
        for p in ("CD2/01.mp3", "CD1/10.mp3", "CD1/2.mp3", ".hidden.mp3", "cover.jpg", "_mp3/x.mp3"):
            path = os.path.join(self.dir, p)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "w").close()
        self.assertEqual(encoder.collect_audio(self.dir), ["CD1/2.mp3", "CD1/10.mp3", "CD2/01.mp3"])
        self.assertEqual(os.path.basename(encoder.find_cover(self.dir)), "cover.jpg")

    def test_mixed_inputs_exact_chapters(self):
        tone(os.path.join(self.dir, "01 Anfang.mp3"), 3.0, 44100, 2)
        tone(os.path.join(self.dir, "02 Mitte.mp3"), 2.0, 22050, 1)
        tone(os.path.join(self.dir, "03 Ende.m4a"), 1.5, 48000, 2)
        out = os.path.join(self.dir, "book.m4b")
        seen = []
        info = encoder.merge_to_m4b(self.dir, encoder.collect_audio(self.dir), out, jobs=3,
                                    progress=lambda p, m: seen.append(p))
        self.assertTrue(os.path.exists(out))
        self.assertFalse(info["copied"])
        self.assertEqual([c["title"] for c in info["chapters"]], ["Anfang", "Mitte", "Ende"])
        self.assertAlmostEqual(duration(out), info["duration"], delta=0.05)
        self.assertAlmostEqual(info["duration"], 6.5, delta=0.2)
        embedded = encoder.file_chapters(out)
        self.assertEqual(len(embedded), 3)
        for a, b in zip(embedded, info["chapters"]):
            self.assertAlmostEqual(a["start"], b["start"], delta=0.01)
        self.assertEqual(seen[-1], 1.0)
        # no leftovers next to the output
        self.assertEqual(sorted(os.listdir(self.dir)), ["01 Anfang.mp3", "02 Mitte.mp3", "03 Ende.m4a", "book.m4b"])

    def test_merge_is_warning_free(self):
        """The joined file must not trigger DTS warnings (the m4b-tool classic)."""
        tone(os.path.join(self.dir, "a.mp3"), 2.0)
        tone(os.path.join(self.dir, "b.mp3"), 2.0)
        out = os.path.join(self.dir, "x.m4b")
        encoder.merge_to_m4b(self.dir, ["a.mp3", "b.mp3"], out)
        res = subprocess.run(["ffmpeg", "-nostdin", "-v", "warning", "-i", out, "-map", "0:a", "-f", "null", "-"],
                             capture_output=True, text=True)
        self.assertEqual(res.returncode, 0)
        self.assertNotIn("Non-monotonic", res.stderr)

    def test_aac_sources_are_copied(self):
        tone(os.path.join(self.dir, "p1.m4a"), 2.0, 44100, 2)
        tone(os.path.join(self.dir, "p2.m4a"), 2.0, 44100, 2)
        out = os.path.join(self.dir, "x.m4b")
        info = encoder.merge_to_m4b(self.dir, ["p1.m4a", "p2.m4a"], out)
        self.assertTrue(info["copied"])
        self.assertEqual(info["bitrate"], "copy")

    def test_broken_file_reports_name(self):
        with open(os.path.join(self.dir, "kaputt.mp3"), "wb") as fh:
            fh.write(b"\x00" * 2048)
        with self.assertRaises(encoder.EncodeError) as cm:
            encoder.merge_to_m4b(self.dir, ["kaputt.mp3"], os.path.join(self.dir, "x.m4b"))
        self.assertIn("kaputt.mp3", str(cm.exception))
        self.assertFalse(os.path.exists(os.path.join(self.dir, "x.m4b")))

    def test_cancel(self):
        tone(os.path.join(self.dir, "a.mp3"), 2.0)
        with self.assertRaises(encoder.Cancelled):
            encoder.merge_to_m4b(self.dir, ["a.mp3"], os.path.join(self.dir, "x.m4b"), cancelled=lambda: True)

    def test_remux_with_new_chapters(self):
        tone(os.path.join(self.dir, "a.m4b"), 4.0)
        out = os.path.join(self.dir, "b.m4b")
        encoder.remux(os.path.join(self.dir, "a.m4b"), out, [{"start": 0, "title": "Eins"}, {"start": 2, "title": "Zwei"}])
        self.assertEqual([c["title"] for c in encoder.file_chapters(out)], ["Eins", "Zwei"])


if __name__ == "__main__":
    unittest.main()
