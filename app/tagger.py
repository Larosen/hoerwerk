"""Audiobook tags (MP4/M4B) and sidecar files, as beets-audible writes them.

Tag layout follows seanap's audiobook guide / beets-audible so Audiobookshelf,
Plex (Audnexus agent), Prologue, BookPlayer & co. read everything.
"""
from __future__ import annotations

import json
import os
import re
from typing import Dict, List, Optional

from mutagen.mp4 import MP4, MP4Cover, MP4FreeForm
from i18n import t

FREEFORM = "----:com.apple.iTunes:"
OWN_KEYS = ("©nam", "©alb", "©ART", "aART", "©wrt", "©gen", "©day", "©cmt", "desc", "ldes",
            "©grp", "©mvn", "©mvi", "shwm", "soal", "sonm", "soaa", "soar", "stik", "trkn",
            "disk", "cprt", "©too", "pgap", "©lyr", "tmpo")


def _ff(value) -> List[MP4FreeForm]:
    vals = value if isinstance(value, list) else [value]
    return [MP4FreeForm(str(v).encode("utf-8")) for v in vals if v not in (None, "")]


def clean_title(book: dict, settings: dict) -> str:
    title = book.get("title") or ""
    pos = book.get("series_position")
    if not settings.get("keep_series_in_title", True) and pos:
        title = re.sub(rf"(?i),?\s*(?:book|band|teil|folge|vol\.?|volume|part)\s*{re.escape(pos)}\s*$",
                       "", title).strip() or title
    return title


def clean_subtitle(book: dict, settings: dict) -> Optional[str]:
    sub = book.get("subtitle")
    series = (book.get("series") or "").lower()
    if (sub and series and not settings.get("keep_series_in_subtitle", True)
            and series in sub.lower() and re.search(r"(?i)book|band|teil|folge|\d", sub)):
        return None
    return sub


def album_sort(book: dict, title: str, subtitle: Optional[str]) -> str:
    if book.get("series"):
        if book.get("series_position"):
            return f"{book['series']} {book['series_position']} - {title}"
        return f"{book['series']} - {title}"
    return f"{title} - {subtitle}" if subtitle else title


def build_tags(book: dict, settings: dict) -> Dict[str, list]:
    title = clean_title(book, settings)
    subtitle = clean_subtitle(book, settings)
    authors = ", ".join(book.get("authors") or []) or t("Unbekannt")
    narrators = ", ".join(book.get("narrators") or [])
    artist = ", ".join(filter(None, [authors, narrators])) if settings.get("narrator_in_artist") else authors
    desc = book.get("description") or ""
    tags: Dict[str, list] = {
        "©nam": [title], "©alb": [title], "©ART": [artist], "aART": [authors],
        "stik": [2], "trkn": [(1, 1)], "disk": [(1, 1)], "©too": ["Better Renamer"],
        "soal": [album_sort(book, title, subtitle)],
    }
    if narrators:
        tags["©wrt"] = [narrators]
    if book.get("genres"):
        tags["©gen"] = ["/".join(book["genres"])]
    if book.get("release_date"):
        tags["©day"] = [book["release_date"]]
    if desc:
        tags["©cmt"] = [desc]
        tags["desc"] = [desc]
        tags["ldes"] = [desc]
    if book.get("publisher"):
        tags["cprt"] = [book["publisher"]]
    if book.get("series"):
        tags["©mvn"] = [book["series"]]
        tags["shwm"] = [1]
        pos = book.get("series_position")
        if pos:
            tags["©grp"] = [f"{book['series']}, Book #{pos}"]
            if re.fullmatch(r"\d+", pos):
                tags["©mvi"] = [int(pos)]
    free = {
        "ASIN": book.get("asin"), "AUDIBLE_ASIN": book.get("asin"),
        "SERIES": book.get("series"), "SERIES-PART": book.get("series_position"),
        "SUBTITLE": subtitle, "PUBLISHER": book.get("publisher"),
        "LANGUAGE": book.get("language"), "NARRATOR": narrators or None,
        "WWWAUDIOFILE": book.get("url"), "REGION": book.get("region"),
        "RELEASETYPE": "audiobook",
    }
    for k, v in free.items():
        if v:
            tags[FREEFORM + k] = _ff(v)
    return tags


def _cover(data: bytes) -> MP4Cover:
    fmt = MP4Cover.FORMAT_PNG if data[:8] == b"\x89PNG\r\n\x1a\n" else MP4Cover.FORMAT_JPEG
    return MP4Cover(data, imageformat=fmt)


def write_tags(path: str, book: dict, settings: dict, cover: Optional[bytes] = None) -> None:
    """Replace the book's tags (keeps chapters; keeps the old cover unless a new one is given)."""
    mp4 = MP4(path)
    if mp4.tags is None:
        mp4.add_tags()
    old_cover = mp4.tags.get("covr")
    for k in list(mp4.tags.keys()):
        if k in OWN_KEYS or k.startswith(FREEFORM) or k == "covr":
            del mp4.tags[k]
    mp4.tags.update(build_tags(book, settings))
    if cover:
        mp4.tags["covr"] = [_cover(cover)]
    elif old_cover:
        mp4.tags["covr"] = old_cover
    mp4.save()


def mark_audiobook(path: str, cover: Optional[bytes] = None) -> None:
    """Minimal tagging for a plain conversion: media type "Hörbuch" and the cover."""
    mp4 = MP4(path)
    if mp4.tags is None:
        mp4.add_tags()
    mp4.tags["stik"] = [2]
    if cover:
        mp4.tags["covr"] = [_cover(cover)]
    mp4.save()


def copy_tags(src: str, dst: str) -> None:
    """Carry tags + cover over after an ffmpeg remux (ffmpeg drops freeform atoms)."""
    s = MP4(src)
    d = MP4(dst)
    if s.tags is None:
        return
    if d.tags is None:
        d.add_tags()
    for k, v in s.tags.items():
        d.tags[k] = v
    d.save()


def read_cover(path: str) -> Optional[bytes]:
    try:
        covr = (MP4(path).tags or {}).get("covr")
    except Exception:  # noqa: BLE001 – any unreadable file simply has no cover
        return None
    return bytes(covr[0]) if covr else None


def read_book(path: str) -> dict:
    """Existing tags of an M4B as a book dict (for "ohne Audible ablegen")."""
    try:
        tags = MP4(path).tags or {}
    except Exception:  # noqa: BLE001
        tags = {}

    def one(key: str) -> str:
        v = tags.get(key)
        if not v:
            return ""
        v = v[0]
        return v.decode("utf-8", "replace") if isinstance(v, (bytes, MP4FreeForm)) else str(v)

    def split(s: str) -> List[str]:
        return [x.strip() for x in re.split(r"\s*[,;&]\s*|\s+und\s+|\s+and\s+", s) if x.strip()]

    return {
        "asin": one(FREEFORM + "ASIN") or None,
        "title": one("©alb") or one("©nam"),
        "subtitle": one(FREEFORM + "SUBTITLE") or None,
        "authors": split(one("aART") or one("©ART")),
        "narrators": split(one("©wrt") or one(FREEFORM + "NARRATOR")),
        "series": one("©mvn") or one(FREEFORM + "SERIES") or None,
        "series_position": one(FREEFORM + "SERIES-PART") or (one("©mvi") or None),
        "publisher": one(FREEFORM + "PUBLISHER") or one("cprt") or None,
        "release_date": one("©day")[:10] or None,
        "language": one(FREEFORM + "LANGUAGE") or None,
        "genres": [g for g in one("©gen").split("/") if g],
        "description": one("ldes") or one("desc") or one("©cmt"),
        "region": one(FREEFORM + "REGION") or None,
        "url": one(FREEFORM + "WWWAUDIOFILE") or None,
        "source": "tags",
    }


def write_sidecars(folder: str, book: dict, settings: dict, cover: Optional[bytes]) -> List[str]:
    """desc.txt, reader.txt, cover.jpg – what Audiobookshelf/Booksonic pick up."""
    written = []

    def put(name: str, data, binary: bool = False) -> None:
        path = os.path.join(folder, name)
        with open(path, "wb" if binary else "w", **({} if binary else {"encoding": "utf-8"})) as fh:
            fh.write(data)
        written.append(name)

    if settings.get("write_desc", True) and book.get("description"):
        put("desc.txt", book["description"] + "\n")
    if settings.get("write_reader", True) and book.get("narrators"):
        put("reader.txt", ", ".join(book["narrators"]) + "\n")
    if settings.get("write_cover", True) and cover:
        put("cover.png" if cover[:8] == b"\x89PNG\r\n\x1a\n" else "cover.jpg", cover, binary=True)
    return written


# --------------------------------------------------------------------------- #
# metadata.yml / metadata.json – books that are not on Audible
# --------------------------------------------------------------------------- #

MANUAL_FILES = ("metadata.yml", "metadata.yaml", "metadata.json")


def load_manual(folder: str) -> Optional[dict]:
    """beets-audible compatible ``metadata.yml`` (or the same keys as JSON)."""
    for name in MANUAL_FILES:
        path = os.path.join(folder, name)
        if not os.path.isfile(path) or os.path.islink(path):
            continue
        if os.path.getsize(path) > 512 * 1024:
            raise ValueError(t("{file} ist zu groß", file=name))
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
        if name.endswith(".json"):
            data = json.loads(text)
        else:
            import yaml
            # YAML anchors/aliases allow "billion laughs" memory bombs – a metadata file needs none
            for event in yaml.parse(text, Loader=yaml.SafeLoader):
                if isinstance(event, yaml.AliasEvent) or getattr(event, "anchor", None):
                    raise ValueError(t("{file}: YAML-Anker (& / *) werden aus Sicherheitsgründen nicht unterstützt", file=name))
            data = yaml.safe_load(text)
        if not isinstance(data, dict) or not data.get("title"):
            raise ValueError(t("{file}: Feld „title“ fehlt", file=name))

        def lst(v) -> List[str]:
            if not v:
                return []
            return [str(x) for x in v] if isinstance(v, list) else [s.strip() for s in str(v).split(",")]

        rd = data.get("releaseDate") or data.get("release_date") or data.get("publishedDate") or data.get("publishedYear")
        series, position = data.get("series"), data.get("seriesPosition", data.get("series_position"))
        if isinstance(series, list):
            # Audiobookshelf metadata.json: ["Serie #3", …]
            first = str(series[0]) if series else ""
            m = re.match(r"^(.*?)\s*#\s*([\d.,]+)\s*$", first)
            series, position = (m.group(1), m.group(2)) if m else (first or None, position)
        return {
            "asin": data.get("asin"), "title": str(data["title"]),
            "subtitle": data.get("subtitle"), "authors": lst(data.get("authors") or data.get("author")),
            "narrators": lst(data.get("narrators") or data.get("narrator")),
            "series": series or None,
            "series_position": str(position) if position is not None else None,
            "publisher": data.get("publisher"), "release_date": str(rd)[:10] if rd else None,
            "language": data.get("language", "German"), "genres": lst(data.get("genres")),
            "description": (data.get("description") or "").strip(), "source": name,
        }
    return None
