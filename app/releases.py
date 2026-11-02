"""Is this release the requested book – and how good is it? (Like Radarr's decision engine.)

``evaluate`` gives every indexer result a verdict with a reason a person can read:
  ✓ accepted with a score (format, language, popularity, a plausible size), or
  ✗ rejected: other author, other title, abridged, other language, size doesn't fit the runtime,
    a pack without this volume, or blocked after a failed download.
"""
from __future__ import annotations

import re
from typing import Iterable, List, Optional, Set

import packs
from i18n import t
from usenet import match_words, release_info

STOP = {"der", "die", "das", "und", "ein", "eine", "the", "and", "von", "mit", "roman", "band", "teil", "folge"}
FORMAT_SCORE = {"M4B": 30, "M4A": 20, "MP3": 15, "FLAC": 10}
MIN_KBPS = 20          # below that a "full" audiobook is too small – usually abridged or broken
MAX_KBPS = 1600        # above that it is not this book (or a video)
LOW_KBPS = 48          # below that it sounds thin – taken only when nothing better is there


def surname(name: str) -> str:
    words = match_words(name or "")
    return words[-1] if words else ""


def title_words(title: str) -> List[str]:
    return [w for w in match_words(clean_title(title or "")) if w not in STOP and len(w) > 1]


# "(Ein Bodenstein-Kirchhoff-Krimi 4)", "[Band 3]", ": Band 4", " - Teil 2" at the end of a shop title: the series, not the title
_TAIL_BRACKET = re.compile(r"\s*[\(\[][^\(\)\[\]]*[\)\]]\s*$")
_TAIL_VOLUME = re.compile(r"(?i)\s*[,:\-–—]\s*(?:band|bd|teil|folge|buch|book|vol|volume|nr)\.?\s*\d{1,3}\s*$")


def clean_title(title: str) -> str:
    """The title as release names carry it: without the series remark the shops append."""
    s = " ".join((title or "").split())
    while True:
        shorter = _TAIL_VOLUME.sub("", _TAIL_BRACKET.sub("", s)).strip()
        if shorter == s or not re.search(r"\w", shorter):
            return s
        s = shorter


def name_surname(name: str) -> str:
    """The surname as written ("Nesbø", not "nesbo") – for searching; the indexers get the other spellings too."""
    words = re.findall(r"[^\W\d_][\w'’-]*", name or "")
    return words[-1] if words else ""


_SERIES_NOISE = re.compile(r"(?i)^(?:ein|eine|der|die|das|a|an|the)\s+|[\s-]+(?:krimi|thriller|roman|reihe|serie|saga|"
                           r"series|fall|f(?:ä|ae)lle|mystery|novel)s?$")


def series_core(series: str) -> str:
    """ "Ein Bodenstein-Kirchhoff-Krimi" -> "Bodenstein Kirchhoff" – what release names call the series."""
    s = clean_title(series)
    for _ in range(3):
        s = _SERIES_NOISE.sub("", s).strip()
    return " ".join(re.sub(r"[-–&+/]", " ", s).split())


SEARCH_BUDGET = 9           # indexer queries per search at most, all spellings and fallbacks together


def search_stages(book: dict) -> List[List[str]]:
    """What to ask the indexers for a book, in rounds – the next round only when the one before found nothing
    that fits. Each round is broader; the last one is just the author: then every result is checked.
      1. surname + title (the title without the series remark the shop appends)
      2. the title alone, surname + the first words of a long title, surname + series
      3. only the author
    Every query is searched in all spellings (umlauts, ø, é …) – that is what the budget counts."""
    authors = book.get("authors") or [""]
    author = " ".join(str(authors[0] or "").split())
    sur = name_surname(author)
    title = clean_title(book.get("title") or "")
    words = title.split()
    short = " ".join(words[:3]) if len(words) > 4 else ""
    series = series_core(book.get("series") or "")
    stages: List[List[str]] = [[f"{sur} {title}".strip()]]
    second = []
    if len(words) >= 2 or len(title) >= 8:       # one short word alone finds everything
        second.append(title)
    if short and sur:
        second.append(f"{sur} {short}")
    if series and sur and len(series) >= 4 and match_words(series) != match_words(title):
        second.append(f"{sur} {series}")
    stages.append(second)
    plain = match_words(sur)
    stages.append([sur if plain and len(plain[-1]) >= 4 else author] if author else [])   # "Ng" alone: too much
    seen: set = set()
    out = []
    for stage in stages:
        stage = [q for q in stage if q and q.lower() not in seen]
        seen.update(q.lower() for q in stage)
        if stage:
            out.append(stage)
    return out


LANG_CODES = {"german": "de", "deutsch": "de", "de": "de", "english": "en", "englisch": "en", "en": "en"}
# words that only German (or only English) release names use – for names without a language tag
_DE_WORDS = {"der", "die", "das", "und", "zum", "zur", "vom", "bis", "biss", "ein", "eine", "des", "den", "dem", "im",
             "auf", "aus", "mit", "nach", "hoerbuch", "horbuch", "ungekurzt", "gekurzt", "gelesen", "teil", "folge", "band"}
_EN_WORDS = {"the", "of", "and", "in", "to", "a", "an", "unabridged", "narrated", "book", "audiobook", "edition"}


def book_language(book: dict) -> str:
    """"de", "en" or "" (unknown) – the language a request is for."""
    return LANG_CODES.get((book.get("language") or "").strip().lower(), "")


def guess_language(name: str) -> str:
    """"de"/"en" from the words of a release name without a language tag ("…Bis(s).zum.Morgengrauen" is German)."""
    if re.search(r"[äöüÄÖÜß]", name or ""):
        return "de"
    words = match_words(name)
    de = sum(1 for w in words if w in _DE_WORDS)
    en = sum(1 for w in words if w in _EN_WORDS)
    if de >= 2 and de > en:
        return "de"
    if en >= 2 and en > de:
        return "en"
    return ""


def fits(release: str, book: dict) -> bool:
    """Author's surname and (nearly) all words of the title in the release name."""
    return evaluate({"title": release}, book, check_size=False)["ok"]


def _volumes(book: dict) -> Optional[int]:
    try:
        return int(float(str(book.get("series_position") or "").replace(",", ".")))
    except ValueError:
        return None


# "Band 5", "Teil 05", "Folge 5", "Book 5", "Vol. 5", "#5", "Saga 5" – a volume number in a release name
_VOL_WORDS = r"band|bd|teil|folge|buch|book|vol|volume|nr|saga|reihe|serie|series|episode|ep"
_VOL = re.compile(rf"(?:\b(?:{_VOL_WORDS})\b\.?|#)\s*0*(\d{{1,3}})(?![\da-z])", re.I)


def release_volumes(name: str, book: dict) -> Set[int]:
    """Volume numbers a release name states – after a volume word, or right after the series name
    ("Twilight-Saga 5", "Harry Potter 3", "Die drei ??? Folge 12"). Years and "4CD" are not volumes."""
    text = re.sub(r"[-_.+]", " ", name or "")
    found = {int(m) for m in _VOL.findall(text)}
    # right after a word of the series or the title: "Twilight 4", "Twilight.1", "Harry Potter 3"
    anchors = {w for w in match_words(series_core(book.get("series") or "")) + title_words(book.get("title") or "")
               if len(w) >= 4 and not w.isdigit() and w not in STOP}
    plain = " ".join(match_words(text))
    for w in anchors:
        found |= {int(m) for m in re.findall(rf"\b{re.escape(w)}\s+0*(\d{{1,3}})(?![\da-z])", plain)}
    return {n for n in found if 0 < n < 500}


def evaluate(release: dict, book: dict, prefer_language: str = "de", unabridged: bool = True,
             blocked: Iterable[str] = (), check_size: bool = True) -> dict:
    """{ok, score, reason (why rejected), notes (why it scores), kbps}."""
    name = release.get("title") or ""
    words = set(match_words(name))
    notes: List[str] = []

    def no(reason: str) -> dict:
        return {"ok": False, "score": 0.0, "reason": reason, "notes": notes, "kbps": kbps}

    kbps = None
    runtime = int(book.get("runtime_min") or 0)
    size = int(release.get("size") or 0)
    pack = packs.pack_info(name)
    if runtime and size and not pack:
        kbps = round(size * 8 / 1000 / (runtime * 60))
    if name in set(blocked):
        return no(t("gesperrt (Download oder Import ging schief)"))
    author = surname((book.get("authors") or [""])[0])
    if author and author not in words:
        return no(t("Autor „{name}“ fehlt im Namen", name=(book.get("authors") or [""])[0]))
    title = title_words(book.get("title") or "")
    if not title:
        return no(t("Buch ohne Titel"))
    share = sum(1 for w in title if w in words) / len(title)
    if share < 0.8:
        return no(t("Titel passt nicht ({n} % der Wörter)", n=round(share * 100)))
    if pack:
        vol = _volumes(book)
        if vol is None or vol not in pack["volumes"]:
            return no(t("Paket ohne diesen Band ({first}–{last})", first=pack["volumes"][0], last=pack["volumes"][-1]))
        notes.append(t("Paket – nur Band {n} wird übernommen", n=vol))
    else:
        # a series book: a release that names another volume is another book ("Twilight-Saga 5" for Band 1)
        vol = _volumes(book)
        stated = release_volumes(name, book)
        if vol is not None and stated and vol not in stated:
            return no(t("anderer Band ({found} statt {vol})", found=", ".join(str(n) for n in sorted(stated)), vol=vol))
        if vol is not None and vol in stated:
            notes.append(t("Band {n} passt", n=vol))
    if unabridged and release.get("abridged"):
        return no(t("gekürzte Fassung"))
    lang = release.get("language")
    if prefer_language and lang and lang != prefer_language:
        return no(t("andere Sprache ({lang})", lang=lang))
    if prefer_language and not lang:
        guessed = guess_language(name)
        if guessed and guessed != prefer_language:
            return no(t("vermutlich andere Sprache ({lang})", lang=guessed))
    if check_size and kbps is not None and (release.get("format") or "").upper() in ("FLAC", "LOSSLESS", "WAV") and kbps < 200:
        # lossless audio of the whole book is far bigger – a small FLAC is something else (e.g. a music album)
        return no(t("verlustfrei, aber viel zu klein für {h} h Laufzeit ({kbps} kbit/s)", h=round(runtime / 60, 1), kbps=kbps))
    if check_size and kbps is not None:
        if kbps < MIN_KBPS:
            return no(t("zu klein für {h} h Laufzeit ({kbps} kbit/s)", h=round(runtime / 60, 1), kbps=kbps))
        if kbps > MAX_KBPS:
            return no(t("zu groß für {h} h Laufzeit ({kbps} kbit/s)", h=round(runtime / 60, 1), kbps=kbps))
    score = float(FORMAT_SCORE.get(release.get("format") or "", 5))
    if release.get("format"):
        notes.append(release["format"])
    if lang and lang == prefer_language:
        score += 10
        notes.append(t("Sprache passt"))
    if kbps is not None and (release.get("format") or "").upper() not in ("FLAC", "LOSSLESS", "WAV"):
        if kbps < LOW_KBPS:
            score -= 40                 # far behind anything that sounds right
            notes.append(t("geringe Qualität ({kbps} kbit/s)", kbps=kbps))
        else:
            score += min(kbps, 160) / 10     # more kbit/s sound better – up to 160, beyond that it makes no difference
    year = max((int(y) for y in re.findall(r"(?<!\d)((?:19|20)\d\d)(?!\d)", name)), default=0)
    if year:
        score += max(0, min((year - 2005) / 2, 8))   # newer releases come from better sources
    score += min(int(release.get("grabs") or 0), 200) / 20
    score += share * 5 - (8 if pack else 0)
    if not pack and _volumes(book) is not None and _volumes(book) in release_volumes(name, book):
        score += 20                     # names the very volume – worth more than a format or language tag
    return {"ok": True, "score": round(score, 2), "reason": None, "notes": notes, "kbps": kbps}


def mismatch(name: str, book: dict, duration: float = 0) -> Optional[str]:
    """Why a downloaded folder can't be this book – another volume, another language, a far other runtime –
    or None. Only clear evidence counts: a name without volume or language says nothing against it."""
    if not packs.pack_info(name or ""):
        vol = _volumes(book)
        stated = release_volumes(name, book)
        if vol is not None and stated and vol not in stated:
            return t("anderer Band ({found} statt {vol})", found=", ".join(str(n) for n in sorted(stated)), vol=vol)
    want = book_language(book)
    got = release_info(name or "").get("language") or guess_language(name or "")
    if want and got and got != want:
        return t("andere Sprache ({lang})", lang=got)
    runtime = int(book.get("runtime_min") or 0) * 60
    if duration and runtime and not 0.6 <= duration / runtime <= 1.67:
        return t("Laufzeit passt nicht ({have} statt {want} min)", have=round(duration / 60), want=runtime // 60)
    return None


def rank(results: List[dict], book: dict, **kw) -> List[dict]:
    """All results with their verdict, the accepted ones first (best first)."""
    out = [{**r, "verdict": evaluate(r, book, **kw)} for r in results]
    out.sort(key=lambda r: (not r["verdict"]["ok"], -r["verdict"]["score"]))
    return out
