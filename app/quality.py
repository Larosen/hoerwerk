"""How good a book should sound: radio play or reading, and how much bitrate each may keep.

* A reading is speech – AAC at 128 kbit/s stereo is transparent for it.
* A radio play (Hörspiel) has music, effects and space – it keeps up to 256 kbit/s, more on "original".
* Dolby Atmos / AC-3 sources are never mixed down: their audio is packed into the M4B as it is.
* Never more than the source has: a 64k reading stays 64k on every setting.
"""
from __future__ import annotations

import re
from typing import Optional

PROFILES = ("standard", "high", "original")
# cap in kbit/s for stereo when re-encoding: (reading, radio play); mono gets half
CAPS = {"standard": (128, 192), "high": (160, 256), "original": (320, 320)}
# codecs that are packed as they are (never re-encoded): Dolby always, lossless ALAC only on "original"
DOLBY = ("eac3", "ac3")
LOSSLESS_KEEP = ("alac",)

_DRAMA_WORDS = re.compile(r"(?i)h(?:ö|oe)rspiel|dramati[sz]|full[\s-]?cast|radio\s?(?:play|drama)|audio\s?drama|"
                          r"inszenierte\s+lesung")
_DRAMA_GENRES = re.compile(r"(?i)h(?:ö|oe)rspiel|radio\s?drama|dramatized|drama\s*&\s*plays")


def is_drama(book: Optional[dict], name: str = "") -> bool:
    """A radio play rather than a reading – from the shop data, the release or folder name, or many speakers."""
    book = book or {}
    text = " ".join(str(book.get(k) or "") for k in ("title", "subtitle", "format_type", "publisher")) + " " + (name or "")
    text = re.sub(r"[._]+", " ", text)          # scene names: "Full.Cast.Audio.Drama"
    if _DRAMA_WORDS.search(text):
        return True
    if any(_DRAMA_GENRES.search(g or "") for g in (book.get("genres") or []) + (book.get("tags") or [])):
        return True
    if (book.get("format_type") or "").lower() in ("dramatized", "radio_play", "original_recording"):
        return True
    return len([n for n in book.get("narrators") or [] if n]) >= 4      # a cast, not a reader


def cap(profile: str, drama: bool, channels: int) -> int:
    """Highest bitrate (kbit/s) a re-encode may use."""
    reading, play = CAPS.get(profile, CAPS["high"])
    top = play if drama else reading
    return top if channels > 1 else max(64, top // 2)


def keep_codec(codec: Optional[str], profile: str) -> bool:
    """Audio that is packed into the M4B as it is instead of being re-encoded."""
    return codec in DOLBY or (profile == "original" and codec in LOSSLESS_KEEP)
