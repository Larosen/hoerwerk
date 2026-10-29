import os
import struct
import sys
import tempfile
import unittest
from datetime import datetime

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

from meta import _resolution, kind_of, read_exif  # noqa: E402


def make_jpeg_with_exif(make="Canon", model="Canon EOS R6", taken="2021:07:04 18:30:05"):
    """Minimal big-endian EXIF block: IFD0 (Make, Model, ExifIFD) -> ExifIFD (DateTimeOriginal)."""
    def ascii_val(s):
        return s.encode() + b"\x00"

    make_b, model_b, date_b = ascii_val(make), ascii_val(model), ascii_val(taken)
    ifd0_off = 8
    ifd0_size = 2 + 3 * 12 + 4
    data_off = ifd0_off + ifd0_size
    make_off = data_off
    model_off = make_off + len(make_b)
    exif_ifd_off = model_off + len(model_b)
    exif_size = 2 + 1 * 12 + 4
    date_off = exif_ifd_off + exif_size

    tiff = b"MM\x00\x2a" + struct.pack(">I", ifd0_off)
    tiff += struct.pack(">H", 3)
    tiff += struct.pack(">HHII", 0x010F, 2, len(make_b), make_off)
    tiff += struct.pack(">HHII", 0x0110, 2, len(model_b), model_off)
    tiff += struct.pack(">HHII", 0x8769, 4, 1, exif_ifd_off)
    tiff += struct.pack(">I", 0)
    tiff += make_b + model_b
    tiff += struct.pack(">H", 1) + struct.pack(">HHII", 0x9003, 2, len(date_b), date_off) + struct.pack(">I", 0)
    tiff += date_b
    app1 = b"Exif\x00\x00" + tiff
    return b"\xff\xd8" + b"\xff\xe1" + struct.pack(">H", len(app1) + 2) + app1 + b"\xff\xd9"


class MetaTest(unittest.TestCase):
    def test_exif(self):
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as fh:
            fh.write(make_jpeg_with_exif())
        try:
            m = read_exif(fh.name)
        finally:
            os.unlink(fh.name)
        self.assertEqual(m["make"], "Canon")
        self.assertEqual(m["camera"], "Canon EOS R6")
        self.assertEqual(m["_taken"], datetime(2021, 7, 4, 18, 30, 5))

    def test_kind_and_resolution(self):
        self.assertEqual(kind_of("a.MP3"), "audio")
        self.assertEqual(kind_of("a.mkv"), "video")
        self.assertEqual(kind_of("a.CR2"), "image")
        self.assertEqual(kind_of("a.pdf"), "other")
        self.assertEqual(_resolution(1920, 1080), "1080p")
        self.assertEqual(_resolution(3840, 2160), "2160p")
        self.assertEqual(_resolution(1280, 720), "720p")


if __name__ == "__main__":
    unittest.main()
