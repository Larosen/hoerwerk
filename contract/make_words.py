"""Builds contract/golden/words.json: how Hörwerk reads names, titles, track names – the Python answers to tricky
inputs. Other implementations (rust/hoerwerk) must give the same. tests/test_golden.py keeps the file current."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))

TEXTS = [
    "Fräulein", "Fraeulein", "FRAULEIN", "Straße", "Nesbø", "Łódź", "Œuvre", "Þór", "Bis(s) zum Morgengrauen",
    "Bis (s) zum", "Ab(cde)f", "Ab(cd)", "Ab(cd)e", "Café crème", "Jo Nesbø – Der Schneemann", "naïve", "İstanbul",
    "Ærø", "Grüße aus Köln", "Goethe, Johann Wolfgang von", "„Zitat“ (Hörbuch) [Band 3]", "1984", "Teil 2",
    "Der Herr der Ringe: Die Gefährten", "Das Buch", "A", "Die drei ??? und der Super-Papagei", "ﬁnal ﬀ ½",
    "Mutter Courage und ihre Kinder", "Ödön von Horváth", "Ἀθῆναι", "Чехов", "", "   ", "Toedliche.Saat.German.Hoerbuch",
    "Queue", "Poesie", "Aeon Flux", "Ruebe", "Eragon (2) - Der Auftrag des Ältesten", "Titel (unvollständig",
    "Titel [eins) (zwei] ende", "x(y)z", "(Anfang) Mitte [Ende]",
]
NAMES = [
    "Sebastian Fitzek, Simon Jäger", "Storm, Theodor", "Autor / Sprecher", "A & B", "A und B", "Hund und Katze",
    "A feat. B", "A feat B", "Featuring X", "Anna gelesen von Bert", "Anna read by Bert", "Storm, Theodor & Max Muster",
    "A, Bert Brecht", "A,Bert Brecht", "Rundfunk", "Sandra Undine", "A; B; C", "A | B + C", "  Theodor Storm  ",
    "Brand, Anders", "Müller und Söhne, Hans Peter", "Und Und", "X , Y Z", "A,  ", ", Ab Cd",
]
PAIRS = [
    "Storm, Theodor - Der Schimmelreiter", "01 - Titel", "Theodor Storm – Der Schimmelreiter", "A - B - C",
    "Kein Trenner", "A-B", "  Lead - Space  ", "A -  ", "A  -  B", "2001 - Odyssee", "A – ", "x\t-\ty",
]
SORT = ["Track 10.mp3", "Track 2.mp3", "track 1.mp3", "CD2/01.mp3", "CD1/10.mp3", "CD1/2.mp3", "a", "A10b2", "a10b10",
        "10", "9", "", "Kapitel 007", "Kapitel 7", "b", "B"]
TRACKS = [
    ("01 - Der Anfang.mp3", {}), ("01 - Der Anfang.mp3", {"title": "Track 01"}), ("02_das_ende.mp3", {}),
    ("CD1/03 Kapitel.mp3", {}), ("Track 07.mp3", {"title": ""}), ("x.mp3", {"title": "Echter Titel"}),
    ("12) Zwölf.flac", {}), ("chapter 3.m4a", {"title": "Kapitel 3"}), ("1234 Lang.mp3", {}), ("12345 Zu lang.mp3", {}),
    ("part.mp3", {}), ("007.mp3", {"title": "  Bond  "}),
]
TITLE_SETS = [
    (["Der Schimmelreiter - 01", "Der Schimmelreiter - 02", "Der Schimmelreiter - 03"], "Der Schimmelreiter"),
    (["Sie werden dich jagen - Kapitel 1", "Sie werden dich jagen - Kapitel 2"], ""),
    (["Track 1", "Track 2", "Track 3"], ""), (["001", "002", "004"], ""), (["3", "1", "2"], ""),
    (["Montag", "Sonntag"], ""), (["Prolog (2012)", "Das Ende (2012)"], ""), (["Einzig"], "Einzig"),
    (["Kapitel 1 - Anfang", "Kapitel 2 - Mitte"], ""), (["", ""], ""), (["ÜBER alles 1", "über ALLES 2"], "Über Alles"),
    (["A – Eins", "A – Zwei"], ""), (["Folge 1: Der Fall", "Folge 2: Die Wende"], "Folge"),
]


def build() -> dict:
    import encoder
    import i18n
    from library_index import names_of, split_author_title, title_words
    from natsort import natural_key
    from usenet import match_words
    i18n.set_language("de")
    return {
        "description": "How Hörwerk reads names, titles and track names (Python answers; see make_words.py).",
        "match_words": [[t, match_words(t)] for t in TEXTS + NAMES],
        "title_words": [[t, title_words(t)] for t in TEXTS],
        "names_of": [[n, names_of(n)] for n in NAMES],
        "split_author_title": [[p, list(r) if (r := split_author_title(p)) else None] for p in PAIRS],
        "natural_sort": {"input": SORT, "sorted": sorted(SORT, key=lambda p: [natural_key(x) for x in p.split("/")])},
        "chapter_title": [[path, tags, mode, encoder.chapter_title(path, tags, mode)]
                          for path, tags in TRACKS for mode in ("auto", "tag", "filename")],
        "chapter_titles": [[names, title, encoder.chapter_titles(names, title)] for names, title in TITLE_SETS],
    }


if __name__ == "__main__":
    with open(os.path.join(HERE, "golden", "words.json"), "w", encoding="utf-8") as fh:
        json.dump(build(), fh, ensure_ascii=False, indent=1)
        fh.write("\n")
