"""Downloads with several volumes of a series ("Sneijder und Nemez 01 - 07", "Band 1-3").

``pack_info`` reads series, author and volume range from a release or folder name;
``volume_of`` finds the volume number of one book folder inside such a download.
"""
from __future__ import annotations

import re
from typing import List, Optional

# "01 - 07", "1-7", "1 bis 7", "Band 1-3", "Folgen 10–12" – not "7CD", not "128kbps"
RANGE = re.compile(
    r"(?<![\w])(\d{1,3})\s*(?:-|–|bis|to)\s*(\d{1,3})(?![\d])(?!\s*(?:cd|kbps|kbit|k\b|disc))", re.I)
VOLUME_WORD = r"(?:b(?:a|ä|ae)nde?|folgen?|teile?|vol(?:ume)?s?|books?|b(?:u|ü|ue)cher|buch|episoden?|fall|f(?:a|ä|ae)lle)"
MAX_VOLUMES = 60


def _plain(name: str) -> str:
    return re.sub(r"[._]+", " ", name or "").strip()


def pack_info(name: str) -> Optional[dict]:
    """{volumes, series, author} when the name announces a range of volumes, else None."""
    text = _plain(name)
    m = RANGE.search(text)
    if not m:
        return None
    first, last = int(m.group(1)), int(m.group(2))
    if not (0 <= first < last <= first + MAX_VOLUMES):
        return None
    prefix = re.sub(rf"(?i)\b{VOLUME_WORD}\.?\s*$", "", text[:m.start()]).strip(" -–:,")
    parts = [p.strip() for p in re.split(r"\s+-\s+|(?<=\w)-(?=\w)", prefix) if p.strip()]
    if len(parts) >= 2:
        author, series = parts[0], " ".join(parts[1:])
    else:
        author, series = "", parts[0] if parts else ""
    return {"volumes": list(range(first, last + 1)), "series": series, "author": author}


def volume_of(folder_name: str, pack: Optional[dict] = None) -> Optional[int]:
    """Volume number of one book folder of a pack: "03 - Todesmal", "Band 3", "Sneijder und Nemez 3 - …"."""
    text = _plain(folder_name)
    for known in ((pack or {}).get("author"), (pack or {}).get("series")):
        if known:
            text = re.sub(re.escape(known), " ", text, flags=re.I)
    m = re.search(rf"(?i)\b{VOLUME_WORD}\.?\s*(\d{{1,3}})\b", text) or re.search(
        r"(?<![\w])(\d{1,3})(?![\d])(?!\s*(?:cd|kbps|kbit|k\b|disc))", text)
    if not m:
        return None
    n = int(m.group(1))
    volumes: List[int] = (pack or {}).get("volumes") or []
    return n if not volumes or n in volumes else None


def title_of(folder_name: str, pack: Optional[dict] = None) -> str:
    """The book title in a pack folder name, without author, series and volume number."""
    text = _plain(folder_name)
    for known in ((pack or {}).get("author"), (pack or {}).get("series")):
        if known:
            text = re.sub(re.escape(known), " ", text, flags=re.I)
    text = re.sub(rf"(?i)\b{VOLUME_WORD}\.?\s*\d{{1,3}}\b", " ", text)
    text = re.sub(r"^\W*\d{1,3}(?![\d])\W*", "", text.strip())
    return re.sub(r"\s+", " ", text).strip(" -–:,")
