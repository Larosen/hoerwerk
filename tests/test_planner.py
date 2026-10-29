import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

import planner  # noqa: E402


def tree(base):
    out = []
    for root, dirs, files in os.walk(base):
        rel = os.path.relpath(root, base)
        for f in files:
            out.append(f if rel == "." else f"{rel}/{f}")
        for d in dirs:
            out.append((d if rel == "." else f"{rel}/{d}") + "/")
    return sorted(out)


class PlannerTest(unittest.TestCase):
    def setUp(self):
        self.base = tempfile.mkdtemp(prefix="planner-")

    def tearDown(self):
        shutil.rmtree(self.base, ignore_errors=True)

    def touch(self, *names):
        for n in names:
            p = os.path.join(self.base, n)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as fh:
                fh.write(n)

    def plan(self, items, rules, **opts):
        return planner.compute_plan(self.base, items, rules, opts)

    def apply(self, items, rules, **opts):
        plan = self.plan(items, rules, **opts)
        self.assertFalse([p for p in plan if p["error"]], plan)
        phases = planner.build_phases(plan)
        created = planner.execute(self.base, phases)
        return phases, created

    def test_swap_names(self):
        self.touch("a.txt", "b.txt")
        rules = [{"type": "replace", "find": "^a$", "replace": "TMP", "regex": True},
                 {"type": "replace", "find": "^b$", "replace": "a", "regex": True},
                 {"type": "replace", "find": "TMP", "replace": "b", "case_sensitive": True}]
        phases, created = self.apply(["a.txt", "b.txt"], rules)
        with open(os.path.join(self.base, "a.txt")) as fh:
            self.assertEqual(fh.read(), "b.txt")
        planner.undo(self.base, phases, created)
        with open(os.path.join(self.base, "a.txt")) as fh:
            self.assertEqual(fh.read(), "a.txt")

    def test_conflicts(self):
        self.touch("a.txt", "b.txt", "c.txt")
        plan = self.plan(["a.txt", "b.txt"], [{"type": "number", "position": "replace", "step": 0, "padding": 1}])
        self.assertTrue(all(p["error"] == "Doppelter Zielname" for p in plan))
        plan = self.plan(["a.txt"], [{"type": "replace", "find": "a", "replace": "c"}])
        self.assertEqual(plan[0]["error"], "Datei existiert bereits")
        plan = self.plan(["a.txt", "b.txt"], [{"type": "replace", "find": "a|b", "replace": "c", "regex": True}],
                         conflict="number")
        self.assertEqual([p["new"] for p in plan], ["c (2).txt", "c (3).txt"])
        self.assertFalse(any(p["error"] for p in plan))

    def test_case_only_rename(self):
        self.touch("abc.txt")
        self.apply(["abc.txt"], [{"type": "case", "mode": "upper"}])
        self.assertIn("ABC.txt", os.listdir(self.base))

    def test_sidecars_follow(self):
        self.touch("movie.mkv", "movie.srt", "movie.de.srt", "movie.nfo", "other.srt")
        rules = [{"type": "replace", "find": "movie", "replace": "Film"}]
        # sidecars not selected
        plan = self.plan(["movie.mkv"], rules, sidecars=True)
        self.assertEqual(sorted(p["new"] for p in plan), ["Film.de.srt", "Film.mkv", "Film.nfo", "Film.srt"])
        # sidecars selected: they follow instead of being renamed by the rules
        rules = [{"type": "number", "position": "start", "padding": 1, "separator": " "}]
        plan = self.plan(["movie.de.srt", "movie.mkv", "movie.srt"], rules, sidecars=True)
        self.assertEqual(sorted(p["new"] for p in plan), ["1 movie.de.srt", "1 movie.mkv", "1 movie.nfo", "1 movie.srt"])

    def test_recursive_and_folders_with_undo(self):
        self.touch("cd1/track_1.mp3", "cd1/track_2.mp3", "cd2/track_1.mp3")
        before = tree(self.base)
        items = ["cd1", "cd1/track_1.mp3", "cd1/track_2.mp3", "cd2", "cd2/track_1.mp3"]
        rules = [{"type": "cleanup"}, {"type": "case", "mode": "title"}]
        phases, created = self.apply(items, rules)
        self.assertEqual(tree(self.base), ["Cd1/", "Cd1/Track 1.mp3", "Cd1/Track 2.mp3", "Cd2/", "Cd2/Track 1.mp3"])
        planner.undo(self.base, phases, created)
        self.assertEqual(tree(self.base), before)

    def test_numbering_per_folder(self):
        self.touch("a/x.mp3", "a/y.mp3", "b/z.mp3")
        plan = self.plan(["a/x.mp3", "a/y.mp3", "b/z.mp3"],
                         [{"type": "number", "position": "replace", "padding": 2, "per_folder": True}])
        self.assertEqual([p["new"] for p in plan], ["a/01.mp3", "a/02.mp3", "b/01.mp3"])

    def test_subdirs_created_and_removed_on_undo(self):
        self.touch("a.txt", "b.txt")
        phases, created = self.apply(["a.txt", "b.txt"], [{"type": "insert", "text": "neu/", "position": "start"}],
                                     allow_subdirs=True)
        self.assertEqual(tree(self.base), ["neu/", "neu/a.txt", "neu/b.txt"])
        planner.undo(self.base, phases, created)
        self.assertEqual(tree(self.base), ["a.txt", "b.txt"])

    def test_failed_phase_rolls_back(self):
        self.touch("a.txt", "b.txt")
        plan = self.plan(["a.txt", "b.txt"], [{"type": "insert", "text": "x", "position": "start"}])
        phases = planner.build_phases(plan)
        # Something appears at a target between preview and execution
        self.touch("xb.txt")
        with self.assertRaises(OSError):
            planner.execute(self.base, phases)
        self.assertEqual(tree(self.base), ["a.txt", "b.txt", "xb.txt"])

    def test_rejects_path_escape(self):
        with self.assertRaises(planner.PlanError):
            self.plan(["../etc/passwd"], [])


if __name__ == "__main__":
    unittest.main()
