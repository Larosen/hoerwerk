"""The documents that name a version keep up with the code: CHANGELOG.md has a section for the current version
(the release notes are cut from it), and the security pages say which version they apply to."""
import json
import os
import re
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def read(name: str) -> str:
    with open(os.path.join(ROOT, name), encoding="utf-8") as fh:
        return fh.read()


class Docs(unittest.TestCase):
    def setUp(self) -> None:
        self.version = json.loads(read("contract/settings.json"))["version"]
        self.minor = ".".join(self.version.split(".")[:2])

    def test_one_version_everywhere(self):
        cargo = re.search(r'^version = "([^"]+)"', read("rust/hoerwerk/Cargo.toml"), re.M).group(1)
        self.assertEqual(cargo, self.version, "rust/hoerwerk/Cargo.toml and contract/settings.json")

    def test_changelog_has_this_version(self):
        self.assertRegex(read("CHANGELOG.md"), rf"(?m)^## {re.escape(self.version)} – \d{{4}}-\d\d-\d\d$",
                         "CHANGELOG.md needs a section for this version (the release notes come from it)")

    def test_security_pages_name_this_version(self):
        for name, word in (("SECURITY.md", "version"), ("SECURITY.de.md", "Version")):
            head = "\n".join(read(name).splitlines()[:8])
            self.assertIn(f"**{word} {self.minor}**", head, f"{name}: look it through and name {self.minor}")


if __name__ == "__main__":
    unittest.main()
