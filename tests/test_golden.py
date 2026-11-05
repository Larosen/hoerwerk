"""The tag writer still produces exactly contract/golden/tags.json – the yardstick a Rust writer must meet too."""
import os
import shutil
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))
sys.path.insert(0, os.path.join(HERE, "..", "contract"))

import tagger  # noqa: E402


@unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "ffmpeg fehlt")
class GoldenTagsTest(unittest.TestCase):
    def test_python_writer_matches_golden(self):
        import make_golden
        want = make_golden._load(os.path.join(HERE, "..", "contract", "golden", "tags.json"))
        self.assertEqual(make_golden.golden(tagger.write_tags), want)


class SettingsContractTest(unittest.TestCase):
    def test_settings_json_matches_config(self):
        """contract/settings.json is what the Rust backend reads – it must follow app/config.py."""
        import json
        import make_settings
        with open(os.path.join(HERE, "..", "contract", "settings.json"), encoding="utf-8") as fh:
            self.assertEqual(json.load(fh), json.loads(json.dumps(make_settings.build())),
                             "contract/settings.json veraltet: python3 contract/make_settings.py")


if __name__ == "__main__":
    unittest.main()
