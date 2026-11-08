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

    def test_words_json_matches_the_python_rules(self):
        """contract/golden/words.json: names, titles, track names – what other implementations must match."""
        import json
        import make_words
        with open(os.path.join(HERE, "..", "contract", "golden", "words.json"), encoding="utf-8") as fh:
            self.assertEqual(json.load(fh), json.loads(json.dumps(make_words.build())),
                             "contract/golden/words.json veraltet: python3 contract/make_words.py")

    def test_releases_json_matches_the_python_rules(self):
        """contract/golden/releases.json: verdicts on releases, packs, indexer answers."""
        import json
        import make_releases
        with open(os.path.join(HERE, "..", "contract", "golden", "releases.json"), encoding="utf-8") as fh:
            self.assertEqual(json.load(fh), json.loads(json.dumps(make_releases.build())),
                             "contract/golden/releases.json veraltet: python3 contract/make_releases.py")

    def test_audible_json_matches_the_python_rules(self):
        """contract/golden/audible.json: shop data, match scores, search terms."""
        import json
        import make_audible
        with open(os.path.join(HERE, "..", "contract", "golden", "audible.json"), encoding="utf-8") as fh:
            self.assertEqual(json.load(fh), json.loads(json.dumps(make_audible.build())),
                             "contract/golden/audible.json veraltet: python3 contract/make_audible.py")

    def test_discover_json_matches_the_python_rules(self):
        """contract/golden/discover.json: review headlines, feeds, best-of list PDFs; the trusted authors as text."""
        import json
        import make_discover
        with open(os.path.join(HERE, "..", "contract", "golden", "discover.json"), encoding="utf-8") as fh:
            self.assertEqual(json.load(fh), json.loads(json.dumps(make_discover.build())),
                             "contract/golden/discover.json veraltet: python3 contract/make_discover.py")
        with open(make_discover.TRUSTED_TXT, encoding="utf-8") as fh:
            self.assertEqual(fh.read(), make_discover.trusted_txt(),
                             "rust/hoerwerk/src/trusted_authors.txt veraltet: python3 contract/make_discover.py")


if __name__ == "__main__":
    unittest.main()
