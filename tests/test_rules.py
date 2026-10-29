import os
import sys
import unittest
from datetime import datetime

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "app"))

from rules import apply_rules, needs_meta, validate_name  # noqa: E402


def run(name, *rules, **ctx):
    base = {"index": 0, "folder_index": 0, "original": name, "folder": "Ordner",
            "parent": "Eltern", "mtime": datetime(2024, 5, 17, 9, 30).timestamp(),
            "ctime": datetime(2023, 1, 2).timestamp(), "size": 2048, "meta": {}}
    base.update(ctx)
    return apply_rules(name, list(rules), base)


class TextRules(unittest.TestCase):
    def test_replace_literal_and_occurrences(self):
        self.assertEqual(run("a-a-a.txt", {"type": "replace", "find": "a", "replace": "b"}), "b-b-b.txt")
        self.assertEqual(run("a-a-a.txt", {"type": "replace", "find": "a", "replace": "b", "occurrence": "first"}), "b-a-a.txt")
        self.assertEqual(run("a-a-a.txt", {"type": "replace", "find": "a", "replace": "b", "occurrence": "last"}), "a-a-b.txt")

    def test_replace_regex_groups(self):
        r = {"type": "replace", "find": r"^(\d+)_", "replace": "Kapitel $1 - ", "regex": True}
        self.assertEqual(run("07_Intro.mp3", r), "Kapitel 07 - Intro.mp3")
        r["replace"] = r"Teil \1: "
        self.assertEqual(run("07_Intro.mp3", r), "Teil 07: Intro.mp3")

    def test_replace_whole_word_and_case(self):
        r = {"type": "replace", "find": "the", "replace": "X", "whole_word": True}
        self.assertEqual(run("The theme of the day.txt", r), "X theme of X day.txt")
        r["case_sensitive"] = True
        self.assertEqual(run("The theme of the day.txt", r), "The theme of X day.txt")

    def test_replace_extension_target(self):
        self.assertEqual(run("a.JPEG", {"type": "replace", "find": "jpeg", "replace": "jpg", "target": "ext"}), "a.jpg")

    def test_case_title_small_words(self):
        r = {"type": "case", "mode": "title"}
        self.assertEqual(run("der herr der ringe - die gefährten.mkv", r), "Der Herr der Ringe - Die Gefährten.mkv")
        self.assertEqual(run("the lord of the rings.mkv", r), "The Lord of the Rings.mkv")

    def test_case_keep_words(self):
        r = {"type": "case", "mode": "title", "keep": "MP3, McCartney"}
        self.assertEqual(run("best of mccartney mp3.txt", r), "Best of McCartney MP3.txt")

    def test_case_sentence_upper_lower(self):
        self.assertEqual(run("hELLO world.txt", {"type": "case", "mode": "sentence"}), "Hello world.txt")
        self.assertEqual(run("abc.txt", {"type": "case", "mode": "upper"}), "ABC.txt")

    def test_insert_positions(self):
        self.assertEqual(run("name.txt", {"type": "insert", "text": "X-", "position": "start"}), "X-name.txt")
        self.assertEqual(run("name.txt", {"type": "insert", "text": "-X", "position": "end"}), "name-X.txt")
        self.assertEqual(run("name.txt", {"type": "insert", "text": "_", "position": "index", "index": 2}), "na_me.txt")
        self.assertEqual(run("Artist - Title.mp3", {"type": "insert", "text": "(Live) ", "position": "after", "anchor": " - "}),
                         "Artist - (Live) Title.mp3")

    def test_insert_template(self):
        self.assertEqual(run("song.mp3", {"type": "insert", "text": "{folder} - ", "position": "start"}), "Ordner - song.mp3")

    def test_remove_range_and_anchor(self):
        self.assertEqual(run("abcdef.txt", {"type": "remove", "start": 1, "count": 2}), "adef.txt")
        self.assertEqual(run("abcdef.txt", {"type": "remove", "start": 0, "count": 2, "from_end": True}), "abcd.txt")
        self.assertEqual(run("Film [1080p] extra.mkv", {"type": "remove", "mode": "after", "anchor": " [", "keep_anchor": False}), "Film.mkv")
        self.assertEqual(run("01 - Song.mp3", {"type": "remove", "mode": "before", "anchor": " - "}), "Song.mp3")

    def test_remove_chars_and_words(self):
        self.assertEqual(run("a1b2.txt", {"type": "remove_chars", "digits": True}), "ab.txt")
        self.assertEqual(run("Movie 1080p x264.mkv", {"type": "remove_chars", "words": "1080p, x264"}), "Movie.mkv")

    def test_cleanup(self):
        r = {"type": "cleanup", "underscores": True, "dots": True, "collapse": True, "trim": True}
        self.assertEqual(run("my_file.name..here.txt", r), "my file name here.txt")
        self.assertEqual(run("Größe Ärger.txt", {"type": "cleanup", "ascii": True}), "Groesse Aerger.txt")
        self.assertEqual(run('a:b?c.txt', {"type": "cleanup", "windows": True}), "a-b-c.txt")

    def test_truncate(self):
        self.assertEqual(run("one two three.txt", {"type": "truncate", "length": 9, "word_boundary": True}), "one two.txt")
        self.assertEqual(run("abcdef.txt", {"type": "truncate", "length": 3, "keep": "end"}), "def.txt")


class PositionAndNumbers(unittest.TestCase):
    def test_move_and_swap(self):
        self.assertEqual(run("Matrix (1999).mkv", {"type": "move", "find": r"\(\d{4}\)", "regex": True, "to": "start"}),
                         "(1999) Matrix.mkv")
        self.assertEqual(run("Artist - Title.mp3", {"type": "swap", "separator": " - "}), "Title - Artist.mp3")
        self.assertEqual(run("a - b - c.mp3", {"type": "swap", "separator": " - ", "occurrence": "last"}), "c - a - b.mp3")

    def test_number_formats(self):
        r = {"type": "number", "start": 1, "padding": 3, "separator": " ", "position": "start"}
        self.assertEqual(run("x.txt", r, index=4), "005 x.txt")
        r.update(format="roman")
        self.assertEqual(run("x.txt", r, index=3), "IV x.txt")
        r.update(format="alpha")
        self.assertEqual(run("x.txt", r, index=27), "ab x.txt")

    def test_number_per_folder(self):
        r = {"type": "number", "position": "replace", "padding": 2, "per_folder": True}
        self.assertEqual(run("x.txt", r, index=10, folder_index=0), "01.txt")

    def test_pad_and_shift(self):
        self.assertEqual(run("Track 1 of 12.mp3", {"type": "pad_numbers", "width": 2, "which": "first"}), "Track 01 of 12.mp3")
        self.assertEqual(run("E009.mkv", {"type": "pad_numbers", "width": 2, "strip_zeros": True}), "E09.mkv")
        self.assertEqual(run("S01E09.mkv", {"type": "shift_numbers", "offset": 1, "which": "last"}), "S01E10.mkv")
        self.assertEqual(run("E00.mkv", {"type": "shift_numbers", "offset": -5}), "E00.mkv")


class DatesListsTemplates(unittest.TestCase):
    def test_date_rule(self):
        r = {"type": "date", "source": "modified", "format": "%Y-%m-%d", "position": "start", "separator": " "}
        self.assertEqual(run("Foto.jpg", r), "2024-05-17 Foto.jpg")
        r = {"type": "date", "source": "taken", "format": "%Y%m%d", "position": "replace"}
        self.assertEqual(run("IMG.jpg", r, meta={"_taken": datetime(2020, 8, 1, 12, 0)}), "20200801.jpg")
        # no EXIF date -> fallback to modification date
        self.assertEqual(run("IMG.jpg", r), "20240517.jpg")

    def test_list_rule(self):
        r = {"type": "list", "lines": "Eins\n\nZwei\n"}
        self.assertEqual(run("a.mp3", r, index=0), "Eins.mp3")
        self.assertEqual(run("b.mp3", r, index=1), "Zwei.mp3")
        self.assertEqual(run("c.mp3", r, index=2), "c.mp3")

    def test_template_tags_and_formats(self):
        meta = {"track": "3/12", "title": "Der Anfang", "artist": "Autorin"}
        r = {"type": "template", "template": "{track:2} - {title} ({artist:upper})"}
        self.assertEqual(run("x.mp3", r, meta=meta), "03 - Der Anfang (AUTORIN).mp3")
        # files without the tags stay unchanged
        self.assertEqual(run("notes.txt", r), "notes.txt")
        r = {"type": "template", "template": "{modified:%Y}/{name}"}
        self.assertEqual(run("x.mp3", r, allow_subdirs=True), "2024/x.mp3")
        self.assertEqual(run("x.mp3", r), "2024-x.mp3")

    def test_template_builtins(self):
        r = {"type": "template", "template": "{parent} {folder} {n:3} {name:0-2}"}
        self.assertEqual(run("abcdef.txt", r, index=6), "Eltern Ordner 007 ab.txt")

    def test_extension(self):
        self.assertEqual(run("a.JPEG", {"type": "extension", "mode": "normalize"}), "a.jpg")
        self.assertEqual(run("a.txt", {"type": "extension", "mode": "append", "value": "bak"}), "a.txt.bak")
        self.assertEqual(run("Ordner.v2", {"type": "extension", "mode": "upper"}, is_dir=True), "Ordner.v2")


class Conditions(unittest.TestCase):
    def test_only_and_invert(self):
        r = {"type": "insert", "text": "X", "position": "start", "only": "mp3", "only_mode": "ext"}
        self.assertEqual(run("a.mp3", r), "Xa.mp3")
        self.assertEqual(run("a.txt", r), "a.txt")
        r["only_invert"] = True
        self.assertEqual(run("a.txt", r), "Xa.txt")

    def test_disabled_rule(self):
        self.assertEqual(run("a.txt", {"type": "insert", "text": "X", "enabled": False}), "a.txt")

    def test_needs_meta(self):
        self.assertFalse(needs_meta([{"type": "template", "template": "{name} {n}"}]))
        self.assertTrue(needs_meta([{"type": "template", "template": "{title}"}]))
        self.assertTrue(needs_meta([{"type": "date", "source": "taken"}]))

    def test_validate(self):
        self.assertIsNotNone(validate_name(""))
        self.assertIsNotNone(validate_name("a/../b"))
        self.assertIsNone(validate_name("2024/a.txt"))


if __name__ == "__main__":
    unittest.main()
