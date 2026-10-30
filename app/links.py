"""Book data from a shop link: Audible, BookBeat, Thalia.

Audible links resolve to the ASIN (catalog API), BookBeat links to its public
book API, Thalia (and similar pages) through the schema.org data embedded in
the product page.  Only these shops are fetched – no arbitrary URLs.
"""
from __future__ import annotations

import json
import re
from html import unescape
from typing import List, Optional
from urllib.parse import urlparse

import audible
from i18n import t

SHOP_RX = {
    "audible": re.compile(r"(?:^|\.)audible\.(de|com|co\.uk|fr|it|es|ca|com\.au|in|co\.jp)$"),
    "bookbeat": re.compile(r"(?:^|\.)bookbeat\.[a-z]{2,3}$"),
    "thalia": re.compile(r"(?:^|\.)thalia\.(de|at|ch)$"),
}
TLD_REGION = {tld: region for region, tld in audible.REGIONS.items()}
BOOKBEAT_API = "https://api.bookbeat.com/api/books/"


class LinkError(Exception):
    pass


def is_link(text: str) -> bool:
    return bool(re.match(r"(?i)^\s*(https?://|www\.)", text or ""))


def shop_of(url: str) -> Optional[str]:
    host = (urlparse(url).hostname or "").lower()
    for shop, rx in SHOP_RX.items():
        if rx.search(host):
            return shop
    return None


def _split_names(value) -> List[str]:
    if not value:
        return []
    if isinstance(value, list):
        out: List[str] = []
        for v in value:
            out += _split_names(v)
        return out
    if isinstance(value, dict):
        return _split_names(value.get("name"))
    parts = re.split(r"\s*(?:,|;|&|\bund\b|\band\b)\s*", str(value))
    return [unescape(p).strip() for p in parts if p.strip()]


def _empty_book(source: str, url: str) -> dict:
    return {"asin": None, "title": "", "subtitle": None, "authors": [], "narrators": [], "translators": [],
            "series": None, "series_position": None, "publisher": None, "release_date": None,
            "language": None, "runtime_min": 0, "format_type": None, "genres": [], "tags": [],
            "description": "", "cover_url": None, "region": None, "url": url, "source": source,
            "isbn": None}


# --------------------------------------------------------------------------- #
# Audible
# --------------------------------------------------------------------------- #

def audible_link(url: str, client: audible.AudibleClient) -> dict:
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    tld = SHOP_RX["audible"].search(host).group(1)
    asin = audible.ASIN_RX.search(parsed.path.upper() + "/" + parsed.query.upper())
    if not asin:
        raise LinkError(t("Keine ASIN im Audible-Link gefunden"))
    return client.product(asin.group(1), TLD_REGION.get(tld, "us"))


# --------------------------------------------------------------------------- #
# BookBeat
# --------------------------------------------------------------------------- #

def bookbeat_id(url: str) -> Optional[str]:
    m = re.search(r"/book/(?:[^/?#]*-)?(\d+)(?:[/?#]|$)", urlparse(url).path + "/")
    return m.group(1) if m else None


def normalize_bookbeat(d: dict, url: str = "") -> dict:
    book = _empty_book("bookbeat", url)
    editions = d.get("editions") or []
    audio = next((e for e in editions if str(e.get("format", "")).lower().startswith("audio")), None)
    main = audio or (editions[0] if editions else {})
    series = d.get("series") or {}
    cover = d.get("cover")
    if cover:
        cover = re.sub(r"([?&])w=\d+", r"\1w=1400", cover) if "w=" in cover else cover
    book.update({
        "title": (d.get("title") or "").strip(),
        "subtitle": (d.get("subtitle") or "").strip() or None,
        "authors": _split_names(d.get("author")),
        "narrators": _split_names(d.get("narrator")),
        "translators": _split_names(d.get("translator")),
        "series": series.get("name"),
        "series_position": audible.series_position(str(series.get("displaypartnumber") or series.get("partnumber") or "")),
        "publisher": main.get("publisher"),
        "release_date": (main.get("published") or d.get("published") or "")[:10] or None,
        "language": d.get("language"),
        "runtime_min": round((d.get("audiobooklength") or 0) / 60),
        "genres": [g.get("name") for g in d.get("genres") or [] if g.get("name")],
        "description": audible.html_to_text(d.get("summary") or ""),
        "cover_url": cover,
        "isbn": main.get("isbn"),
        "format_type": "unabridged" if audio else None,
        "id": f"bookbeat:{d.get('id')}",
    })
    if d.get("id"):
        book["url"] = url or f"https://www.bookbeat.com/de/book/{d['id']}"
    return book


def _bookbeat_api_ok(url: str) -> bool:
    p = urlparse(url)
    return p.scheme == "https" and (p.hostname or "").lower() == "api.bookbeat.com"


def bookbeat_link(url: str) -> dict:
    book_id = bookbeat_id(url)
    if not book_id:
        raise LinkError(t("Keine Buch-ID im BookBeat-Link gefunden (…/book/titel-123456)"))
    try:
        data = json.loads(audible.http_get(BOOKBEAT_API + str(int(book_id)), allow=_bookbeat_api_ok))
    except (audible.AudibleError, ValueError) as e:
        raise LinkError(t("BookBeat-Daten nicht abrufbar: {error}", error=e)) from e
    if not audio_edition(data):
        raise LinkError(t("Bei BookBeat gibt es dieses Buch nur als E-Book, nicht als Hörbuch"))
    return normalize_bookbeat(data, url)


def audio_edition(d: dict) -> bool:
    return bool(d.get("audiobooklength")) or any(
        str(e.get("format", "")).lower().startswith("audio") for e in d.get("editions") or [])


# --------------------------------------------------------------------------- #
# Product pages with schema.org data (Thalia & co.)
# --------------------------------------------------------------------------- #

def _iso_minutes(value: str) -> int:
    m = re.fullmatch(r"P(?:\d+D)?T?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?", str(value or "").strip())
    if not m:
        return 0
    h, mi, s = (int(x or 0) for x in m.groups())
    return round(h * 60 + mi + s / 60)


def _nodes(data) -> List[dict]:
    if isinstance(data, list):
        out: List[dict] = []
        for d in data:
            out += _nodes(d)
        return out
    if isinstance(data, dict):
        return [data] + _nodes(data.get("@graph") or [])
    return []


def _types(node: dict) -> List[str]:
    t = node.get("@type") or []
    return [x.lower() for x in (t if isinstance(t, list) else [t])]


def _image(value) -> Optional[str]:
    if isinstance(value, list):
        return _image(value[0]) if value else None
    if isinstance(value, dict):
        return value.get("url") or value.get("contentUrl")
    return value or None


def parse_product_page(html: str, url: str, source: str) -> dict:
    """schema.org Book/Audiobook/Product (JSON-LD), falling back to OpenGraph tags."""
    nodes: List[dict] = []
    for m in re.finditer(r"<script[^>]*application/ld\+json[^>]*>(.*?)</script>", html, re.S | re.I):
        try:
            nodes += _nodes(json.loads(m.group(1).strip()))
        except ValueError:
            continue
    book = _empty_book(source, url)
    main = next((n for n in nodes if {"audiobook"} & set(_types(n))), None) \
        or next((n for n in nodes if {"book", "product"} & set(_types(n))), None)
    if main:
        # Book with an audiobook edition (workExample): edition details win
        editions = main.get("workExample") or []
        editions = editions if isinstance(editions, list) else [editions]
        audio = next((e for e in editions if isinstance(e, dict) and (
            "audiobook" in _types(e) or "audiobook" in str(e.get("bookFormat", "")).lower())), {})
        get = lambda k: audio.get(k) or main.get(k)  # noqa: E731
        part_of = main.get("isPartOf") or {}
        part_of = part_of[0] if isinstance(part_of, list) and part_of else part_of
        publisher = get("publisher") or get("brand")
        book.update({
            "title": unescape(str(main.get("name") or "")).strip(),
            "authors": _split_names(main.get("author")),
            "narrators": _split_names(get("readBy")),
            "translators": _split_names(get("translator")),
            "publisher": (publisher.get("name") if isinstance(publisher, dict) else publisher) or None,
            "release_date": str(get("datePublished") or "")[:10] or None,
            "language": get("inLanguage") if isinstance(get("inLanguage"), str) else None,
            "runtime_min": _iso_minutes(get("duration") or get("timeRequired") or ""),
            "description": audible.html_to_text(unescape(str(main.get("description") or ""))),
            "cover_url": _image(main.get("image")),
            "isbn": get("isbn") or get("gtin13") or get("gtin"),
            "genres": _split_names(main.get("genre")) if main.get("genre") else [],
        })
        if isinstance(part_of, dict) and part_of.get("name"):
            book["series"] = part_of["name"]
            book["series_position"] = audible.series_position(str(main.get("position") or part_of.get("position") or ""))
    if not book["title"]:
        og = dict(re.findall(r'<meta[^>]+property="og:(title|image|description)"[^>]+content="([^"]*)"', html))
        book["title"] = unescape(og.get("title", "")).split(" - ")[0].strip()
        book["cover_url"] = book["cover_url"] or unescape(og.get("image", "")) or None
        book["description"] = book["description"] or unescape(og.get("description", ""))
    if not book["title"]:
        raise LinkError(t("Auf der Seite wurden keine Buchdaten gefunden"))
    # "Titel (Band 3)" / "Serie, Band 3: Titel" – take the number when no series data came along
    if not book["series"]:
        m = re.search(r"(?i)^(.*?)[,:]?\s*(?:band|folge|teil|book)\s+(\d+(?:[.,]\d+)?)\s*[:\-–]\s*(.+)$", book["title"])
        if m:
            book["series"], book["series_position"], book["title"] = m.group(1).strip(" ,:"), m.group(2), m.group(3).strip()
    book["id"] = f"{source}:{book['isbn'] or url}"
    book["cover_url"] = safe_url(book["cover_url"])
    return book


def safe_url(url) -> Optional[str]:
    """Only http(s) links leave the server (no javascript:/data: from foreign pages)."""
    return url if isinstance(url, str) and re.match(r"(?i)^https?://[^\s\"'<>]+$", url) else None


def page_link(url: str, source: str) -> dict:
    try:
        html = audible.http_get(url, accept_json=False, timeout=25,
                                allow=lambda u: shop_of(u) == source).decode("utf-8", "replace")
    except audible.AudibleError as e:
        if "403" in str(e) or "429" in str(e):
            raise LinkError(t("{shop} hat den automatischen Abruf blockiert. Daten bitte von Hand eintragen oder einen Audible-/BookBeat-Link verwenden.",  # noqa: E501
                              shop=source.capitalize())) from e
        raise LinkError(t("Seite nicht abrufbar: {error}", error=e)) from e
    return parse_product_page(html, url, source)


def fetch(url: str, client: audible.AudibleClient) -> dict:
    url = url.strip()
    if url.lower().startswith("www."):
        url = "https://" + url
    if urlparse(url).scheme not in ("http", "https") or urlparse(url).username or urlparse(url).port:
        raise LinkError(t("Ungültiger Link"))
    shop = shop_of(url)
    if shop == "audible":
        return audible_link(url, client)
    if shop == "bookbeat":
        return bookbeat_link(url)
    if shop == "thalia":
        return page_link(url, "thalia")
    raise LinkError(t("Nur Links von Audible, BookBeat und Thalia werden unterstützt"))
