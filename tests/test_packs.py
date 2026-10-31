"""Release and folder names with several volumes."""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import packs  # noqa: E402


class PackTest(unittest.TestCase):
    def test_pack_info(self):
        self.assertEqual(packs.pack_info("Theodor_Storm-Novellen_01_-_07-AUDiOBOOK-7CD-MP3-DE-2024-GRP"),
                         {"volumes": [1, 2, 3, 4, 5, 6, 7], "series": "Novellen", "author": "Theodor Storm"})
        self.assertEqual(packs.pack_info("Doyle - Sherlock Holmes Folgen 10–12 (German)")["volumes"], [10, 11, 12])
        self.assertEqual(packs.pack_info("Hoffmann-Serapionsbrueder Band 1 bis 4")["series"], "Serapionsbrueder")
        for single in ("Storm - Der Schimmelreiter 2024 MP3", "Grimm-Maerchen-3CD-DE", "Kafka 128kbps", "Buch 3-1"):
            self.assertIsNone(packs.pack_info(single), single)

    def test_volume_and_title_of_a_folder(self):
        p = packs.pack_info("Theodor Storm-Novellen 01 - 07-DE")
        for name, vol, title in (("01 - Immensee", 1, "Immensee"), ("Novellen 02 - Pole Poppenspäler", 2, "Pole Poppenspäler"),
                                 ("Band 3 - Der Schimmelreiter", 3, "Der Schimmelreiter"),
                                 ("Theodor_Storm-Novellen_04-Aquis_submersus", 4, "Aquis submersus")):
            self.assertEqual((packs.volume_of(name, p), packs.title_of(name, p)), (vol, title), name)
        self.assertIsNone(packs.volume_of("CD1", p))
        self.assertIsNone(packs.volume_of("12 - Zu weit", p))          # not one of the pack's volumes


if __name__ == "__main__":
    unittest.main()
