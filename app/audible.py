"""Audible metadata (search, details, chapters) – what beets-audible does, without beets.

Search and details come straight from the Audible catalog API of the chosen
marketplace; chapter data comes from Audnexus (optional, failures are fine).
"""
from __future__ import annotations

import html
import ipaddress
import json
import re
import socket
import time
import unicodedata
from datetime import date
from difflib import SequenceMatcher
from html.parser import HTMLParser
from typing import Callable, Dict, List, Optional, Sequence
from urllib import parse, request
from urllib.error import HTTPError, URLError
from i18n import t

REGIONS = {
    "de": "de", "us": "com", "uk": "co.uk", "au": "com.au", "ca": "ca",
    "fr": "fr", "it": "it", "es": "es", "in": "in", "jp": "co.jp",
}
RESPONSE_GROUPS = ("contributors,product_attrs,product_desc,product_extended_attrs,"
                   "series,media,category_ladders,rating")
AUDNEXUS = "https://api.audnex.us"
USER_AGENT = "Hoerwerk/5 (+https://github.com/Larosen/rename_merge)"
ASIN_RX = re.compile(r"(?<![A-Z0-9])(B0[0-9A-Z]{8}|\d{9}[\dX])(?![A-Z0-9])")
AUTHOR_ROLES_SKIP = re.compile(r"(?i)(translator|übersetz|editor|herausgeb|illustrat|foreword|vorwort|"
                               r"introduction|einleitung|adapt|bearbeit|contributor|afterword|nachwort)")


class AudibleError(Exception):
    pass


def api_host(region: str) -> str:
    return f"https://api.audible.{REGIONS.get(region, 'com')}"


def store_url(asin: str, region: str) -> str:
    return f"https://www.audible.{REGIONS.get(region, 'com')}/pd/{asin}"


# --------------------------------------------------------------------------- #
# HTTP
# --------------------------------------------------------------------------- #

MAX_RESPONSE = 8 * 1024 * 1024
MAX_IMAGE = 15 * 1024 * 1024


def is_public_url(url: str) -> bool:
    """http(s) URL whose host resolves only to public addresses (no LAN, localhost, metadata IPs)."""
    p = parse.urlparse(url)
    if p.scheme not in ("http", "https") or not p.hostname:
        return False
    try:
        infos = socket.getaddrinfo(p.hostname, p.port or (443 if p.scheme == "https" else 80),
                                   proto=socket.IPPROTO_TCP)
    except (socket.gaierror, UnicodeError, OSError):
        return False
    try:
        return bool(infos) and all(ipaddress.ip_address(i[4][0].split("%")[0]).is_global for i in infos)
    except ValueError:
        return False


class _CheckedRedirects(request.HTTPRedirectHandler):
    """Follow a redirect only if the new target passes the same check as the original URL."""

    def __init__(self, allow: Callable[[str], bool]) -> None:
        super().__init__()
        self.allow = allow

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not self.allow(newurl):
            raise AudibleError(t("Weiterleitung zu {host} abgelehnt", host=parse.urlparse(newurl).hostname or newurl))
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def http_get(url: str, *, timeout: float = 20, retries: int = 3, accept_json: bool = True,
             max_bytes: int = MAX_RESPONSE, allow: Optional[Callable[[str], bool]] = None) -> bytes:
    """GET with retries; honours Retry-After on 429, gives up at once on 404.

    ``allow`` checks the URL and every redirect target; responses are capped at ``max_bytes``.
    """
    if parse.urlparse(url).scheme not in ("http", "https"):
        raise AudibleError(t("Nur http(s)-Adressen erlaubt"))
    if allow and not allow(url):
        raise AudibleError(t("Adresse nicht erlaubt: {host}", host=parse.urlparse(url).hostname))
    opener = request.build_opener(_CheckedRedirects(allow or (lambda u: parse.urlparse(u).scheme in ("http", "https"))))
    delay = 1.5
    last: Optional[Exception] = None
    for attempt in range(retries):
        req = request.Request(url, headers={
            "User-Agent": USER_AGENT,
            "Accept": "application/json" if accept_json else "*/*",
        })
        try:
            with opener.open(req, timeout=timeout) as resp:
                data = resp.read(max_bytes + 1)
                if len(data) > max_bytes:
                    raise AudibleError(t("Antwort zu groß (> {mb} MB)", mb=max_bytes // (1 << 20)))
                return data
        except HTTPError as e:
            last = e
            if e.code in (400, 403, 404):
                raise AudibleError(t("HTTP {code} für {url}", code=e.code, url=url.split("?")[0])) from e
            retry_after = e.headers.get("Retry-After") if e.headers else None
            if retry_after and retry_after.isdigit():
                delay = min(int(retry_after) + 1, 30)
        except (URLError, TimeoutError, OSError) as e:
            last = e
        if attempt < retries - 1:
            time.sleep(delay)
            delay *= 2
    raise AudibleError(t("Audible nicht erreichbar: {error}", error=last))


def fetch_image(url: str) -> Optional[bytes]:
    """Cover download: public hosts only, size-capped, and it must really be a JPEG or PNG."""
    if not url or not is_public_url(url):
        return None
    data = http_get(url, accept_json=False, timeout=30, retries=2, max_bytes=MAX_IMAGE, allow=is_public_url)
    if data[:3] == b"\xff\xd8\xff" or data[:8] == b"\x89PNG\r\n\x1a\n":
        return data
    return None


def _json(url: str) -> dict:
    try:
        return json.loads(http_get(url))
    except ValueError as e:
        raise AudibleError(t("Ungültige Antwort von {url}", url=url.split("?")[0])) from e


# --------------------------------------------------------------------------- #
# Normalisation
# --------------------------------------------------------------------------- #

class _Text(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.parts: List[str] = []

    def handle_starttag(self, tag, attrs):
        if tag in ("p", "div"):
            self.parts.append("\n\n")
        elif tag in ("br", "li"):
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in ("p", "div"):
            self.parts.append("\n\n")

    def handle_data(self, data):
        self.parts.append(data)


def html_to_text(s: str) -> str:
    if not s:
        return ""
    p = _Text()
    p.feed(s)
    text = html.unescape("".join(p.parts))
    lines = [re.sub(r"[ \t]+", " ", ln).strip() for ln in text.splitlines()]
    out: List[str] = []
    for ln in lines:
        if ln or (out and out[-1]):
            out.append(ln)
    return "\n".join(out).strip()


def _split_role(name: str) -> tuple:
    for sep in (" — ", " - ", " – "):
        if sep in name:
            n, role = name.split(sep, 1)
            return n.strip(), role.strip()
    return name.strip(), ""


def series_position(seq: Optional[str]) -> Optional[str]:
    if not seq:
        return None
    m = re.search(r"\d+(?:[.,]\d+)?(?:\s*-\s*\d+(?:[.,]\d+)?)?", str(seq))
    return m.group(0).replace(",", ".").replace(" ", "") if m else None


def normalize_product(p: dict, region: str) -> dict:
    """Audible catalog product -> flat book dict used everywhere else."""
    authors, translators = [], []
    for a in p.get("authors") or []:
        name, role = _split_role(a.get("name", ""))
        if not name:
            continue
        if role and AUTHOR_ROLES_SKIP.search(role):
            if re.search(r"(?i)translator|übersetz", role):
                translators.append(name)
            continue
        if name not in authors:
            authors.append(name)
    narrators = []
    for n in p.get("narrators") or []:
        name = _split_role(n.get("name", ""))[0]
        if name and name not in narrators:
            narrators.append(name)
    series_list = p.get("series") or []
    series = series_list[0] if series_list else None
    genres, tags = [], []
    for ladder in p.get("category_ladders") or []:
        steps = [s.get("name") for s in ladder.get("ladder") or [] if s.get("name")]
        if steps and steps[0] not in genres:
            genres.append(steps[0])
        for s in steps[1:]:
            if s not in tags:
                tags.append(s)
    images = p.get("product_images") or {}
    cover = images.get("1024") or images.get("500") or next(iter(images.values()), None)
    if cover:
        # Amazon image URLs: dropping the size suffix gives the original resolution
        cover = re.sub(r"\._[A-Z0-9_,]+_(?=\.(jpg|png))", "", cover)
    summary_html = p.get("publisher_summary") or p.get("merchandising_summary") or ""
    language = (p.get("language") or "").capitalize()
    return {
        "id": p.get("asin"),
        "asin": p.get("asin"),
        "title": (p.get("title") or "").strip(),
        "subtitle": (p.get("subtitle") or "").strip() or None,
        "authors": authors,
        "translators": translators,
        "narrators": narrators,
        "series": (series or {}).get("title"),
        "series_asin": (series or {}).get("asin"),
        "series_position": series_position((series or {}).get("sequence")),
        "all_series": [{"name": s.get("title"), "position": series_position(s.get("sequence"))}
                       for s in series_list],
        "publisher": p.get("publisher_name"),
        "release_date": (p.get("release_date") or p.get("issue_date") or "")[:10] or None,
        "language": language,
        "runtime_min": int(p.get("runtime_length_min") or 0),
        "format_type": p.get("format_type"),
        "genres": genres,
        "tags": tags,
        "description": html_to_text(summary_html),
        "summary_html": summary_html,
        "cover_url": cover,
        "region": region,
        "url": store_url(p.get("asin", ""), region),
        "is_adult": bool(p.get("is_adult_product")),
        "rating": float(((p.get("rating") or {}).get("overall_distribution") or {}).get("display_average_rating") or 0),
        "ratings": int(((p.get("rating") or {}).get("overall_distribution") or {}).get("num_ratings") or 0),
        "source": "audible",
    }


# --------------------------------------------------------------------------- #
# Client
# --------------------------------------------------------------------------- #

class AudibleClient:
    def __init__(self, get: Callable[[str], dict] = _json) -> None:
        self._get = get
        self._cache: Dict[str, tuple] = {}

    def _cached(self, url: str) -> dict:
        hit = self._cache.get(url)
        if hit and time.time() - hit[0] < 3600:
            return hit[1]
        data = self._get(url)
        if len(self._cache) > 500:
            self._cache.clear()
        self._cache[url] = (time.time(), data)
        return data

    def search(self, region: str, keywords: str = "", title: str = "", author: str = "",
               limit: int = 10) -> List[dict]:
        params = {"response_groups": RESPONSE_GROUPS, "image_sizes": "500,1024",
                  "num_results": limit, "products_sort_by": "Relevance"}
        if keywords:
            params["keywords"] = keywords
        if title:
            params["title"] = title
        if author:
            params["author"] = author
        data = self._cached(f"{api_host(region)}/1.0/catalog/products?{parse.urlencode(params)}")
        return self._products(data.get("products"), region)

    @staticmethod
    def _products(products, region: str) -> List[dict]:
        """Released audiobooks only – no pre-orders, podcasts or magazines."""
        today = date.today().isoformat()
        out = []
        for p in products or []:
            if not p.get("asin") or not p.get("title"):
                continue
            if (p.get("release_date") or "0") > today:
                continue  # not released yet – Audnexus/Audible data is incomplete
            if p.get("content_delivery_type") in ("PodcastParent", "Periodical"):
                continue
            out.append(normalize_product(p, region))
        return out

    def genres(self, region: str) -> List[dict]:
        """The store's genres with their subcategories: [{id, name, children: [{id, name}]}]."""
        data = self._cached(f"{api_host(region)}/1.0/catalog/categories?root=Genres")
        return [{"id": str(c["id"]), "name": c.get("name") or "",
                 "children": [{"id": str(x["id"]), "name": x.get("name") or ""} for x in c.get("children") or [] if x.get("id")]}
                for c in data.get("categories") or [] if c.get("id")]

    def genre_products(self, region: str, genre_id: str, sort: str = "BestSellers", limit: int = 50,
                       page: int = 1) -> List[dict]:
        """Books of one genre, sorted by Audible (BestSellers, -ReleaseDate, AvgRating)."""
        params = {"response_groups": RESPONSE_GROUPS, "image_sizes": "500", "num_results": limit,
                  "products_sort_by": sort, "category_id": genre_id}
        if page > 1:
            params["page"] = page
        data = self._cached(f"{api_host(region)}/1.0/catalog/products?{parse.urlencode(params)}")
        return self._products(data.get("products"), region)

    def similar(self, asin: str, region: str, limit: int = 20) -> List[dict]:
        """Books that Audible's listeners connect with this one."""
        params = {"response_groups": RESPONSE_GROUPS, "image_sizes": "500", "num_results": limit,
                  "similarity_type": "RawSimilarities"}
        data = self._cached(f"{api_host(region)}/1.0/catalog/products/{asin}/sims?{parse.urlencode(params)}")
        return self._products(data.get("similar_products"), region)

    def product(self, asin: str, region: str) -> dict:
        params = {"response_groups": RESPONSE_GROUPS, "image_sizes": "500,1024"}
        data = self._cached(f"{api_host(region)}/1.0/catalog/products/{asin}?{parse.urlencode(params)}")
        p = data.get("product")
        if not p or not p.get("title"):
            raise AudibleError(t("ASIN {asin} im Audible-Shop ({region}) nicht gefunden", asin=asin, region=region))
        return normalize_product(p, region)

    def chapters(self, asin: str, region: str) -> Optional[dict]:
        """Audnexus chapter data: {accurate, runtime_ms, intro_ms, outro_ms, chapters: [{start, title}]}"""
        try:
            data = self._cached(f"{AUDNEXUS}/books/{asin}/chapters?region={region}")
        except AudibleError:
            return None
        chs = data.get("chapters") or []
        if not chs:
            return None
        return {
            "accurate": bool(data.get("isAccurate")),
            "runtime_ms": int(data.get("runtimeLengthMs") or 0),
            "intro_ms": int(data.get("brandIntroDurationMs") or 0),
            "outro_ms": int(data.get("brandOutroDurationMs") or 0),
            "chapters": [{"start": (c.get("startOffsetMs") or 0) / 1000,
                          "title": (c.get("title") or "").strip()} for c in chs],
        }


# --------------------------------------------------------------------------- #
# Query building and matching
# --------------------------------------------------------------------------- #

_NOISE_WORDS = (r"ungek(?:ü|ue)rzt|gek(?:ü|ue)rzt|unabridged|abridged|h(?:ö|oe)rbuch|h(?:ö|oe)rspiel|"
                r"audiobook|audio\s*book|mp3|m4b|m4a|aac|flac|\d{2,3}\s*kbps|\d{2,3}k|retail|"
                r"web|rip|german|deutsch|english|vbr|cbr")
_NOISE_RX = re.compile(r"(?i)\b(?:%s)\b" % _NOISE_WORDS)
_BRACKET_RX = re.compile(r"[\[({]([^\])}]*)[\])}]")


def clean_query(s: str) -> str:
    """Folder/tag text -> search keywords: drop rip cruft, years, CD numbers, brackets."""
    s = ASIN_RX.sub(" ", s or "")
    s = s.replace("_", " ")
    s = re.sub(r"(?<=\w)\.(?=\w{2,})", " ", s)

    def bracket(m: re.Match) -> str:
        inner = m.group(1)
        if _NOISE_RX.search(inner) or re.fullmatch(r"[\s\d.,:/-]*", inner):
            return " "
        return " " + inner + " "

    s = _BRACKET_RX.sub(bracket, s)
    s = re.sub(r"(?i)\b(?:cd|disc|disk|dvd)\s*\d+\b", " ", s)
    s = _NOISE_RX.sub(" ", s)
    s = re.sub(r"\b(?:19|20)\d{2}\b", " ", s)
    s = re.sub(r"\s*[-–—:|]+\s*", " ", s)
    return " ".join(s.split())


def find_asin(*texts: str) -> Optional[str]:
    for text in texts:
        m = ASIN_RX.search((text or "").upper())
        if m and (m.group(1).startswith("B0") or "ASIN" in (text or "").upper() or len(text) <= 12):
            return m.group(1)
    return None


def _fold(s: str) -> str:
    s = (s or "").lower()
    s = s.replace("ß", "ss").replace("ä", "ae").replace("ö", "oe").replace("ü", "ue")
    s = unicodedata.normalize("NFKD", s)
    s = "".join(ch for ch in s if not unicodedata.combining(ch))
    s = re.sub(r"[^\w\s]", " ", s)
    return " ".join(s.split())


_STOP = {"der", "die", "das", "the", "a", "an", "und", "and", "of", "von", "ein", "eine", "teil",
         "band", "book", "buch", "folge", "vol", "volume", "part"}


def _tokens(s: str) -> set:
    return {t for t in _fold(s).split() if t not in _STOP and (len(t) > 1 or t.isdigit())}


def _ratio(a: str, b: str) -> float:
    a, b = _fold(a), _fold(b)
    if not a or not b:
        return 0.0
    return SequenceMatcher(None, a, b).ratio()


def _coverage(needle: str, hay: set) -> float:
    toks = _tokens(needle)
    if not toks:
        return 0.0
    return len(toks & hay) / len(toks)


def score(book: dict, hints: dict) -> dict:
    """How well a candidate fits what we know about the files. 0..1 plus parts.

    hints: query (free text), title, author, narrator, duration (seconds)
    """
    query = hints.get("query") or ""
    qtok = _tokens(query + " " + (hints.get("title") or "") + " " + (hints.get("author") or ""))
    title = book.get("title") or ""
    variants = [title, f"{title} {book.get('subtitle') or ''}",
                f"{book.get('series') or ''} {book.get('series_position') or ''} {title}"]
    t_cov = max(_coverage(v, qtok) for v in variants)
    t_ratio = _ratio(title, hints["title"]) if hints.get("title") else 0.0
    # Penalise candidates whose title adds lots of words that aren't in the query
    extra = len(_tokens(title) - qtok)
    t_score = max(t_cov - 0.08 * max(0, extra - 1), t_ratio)

    authors = book.get("authors") or []
    if hints.get("author"):
        a_score = max((_ratio(a, hints["author"]) for a in authors), default=0.0)
        a_score = max(a_score, max((_coverage(a, _tokens(hints["author"])) for a in authors), default=0.0))
    else:
        a_score = max((_coverage(a.split()[-1] if a.split() else a, qtok) for a in authors), default=0.0)
    n_score = 0.0
    if hints.get("narrator"):
        n_score = max((_ratio(n, hints["narrator"]) for n in book.get("narrators") or []), default=0.0)

    d_score = None
    dur, rt = hints.get("duration") or 0, (book.get("runtime_min") or 0) * 60
    if dur and rt:
        diff = abs(dur - rt) / rt
        d_score = 1.0 if diff <= 0.02 else max(0.0, 1 - (diff - 0.02) / 0.18)

    weights = {"title": 0.55, "author": 0.25, "duration": 0.20}
    parts = {"title": t_score, "author": a_score, "duration": d_score}
    if d_score is None:
        weights = {"title": 0.65, "author": 0.35}
    total = sum(parts[k] * w for k, w in weights.items())
    if n_score > 0.8:
        total = min(1.0, total + 0.03)
    return {"score": round(total, 3), "title": round(t_score, 3), "author": round(a_score, 3),
            "duration": None if d_score is None else round(d_score, 3)}


def rank(books: List[dict], hints: dict) -> List[dict]:
    out = []
    for b in books:
        s = score(b, hints)
        out.append({**b, "match": s})
    # ties: unabridged before abridged/dramatised editions
    out.sort(key=lambda b: (-b["match"]["score"], b.get("format_type") != "unabridged"))
    return out


def best_match(ranked: Sequence[dict], threshold: float) -> Optional[dict]:
    """The top candidate if it is confident and clearly ahead of the runner-up."""
    if not ranked:
        return None
    top = ranked[0]
    if top["match"]["score"] < threshold:
        return None
    # Runtime far off (other edition, abridged, dramatisation, incomplete files): ask
    if top["match"].get("duration") is not None and top["match"]["duration"] < 0.5:
        return None
    if len(ranked) > 1:
        second = ranked[1]
        same_book = (_fold(second.get("title")) == _fold(top.get("title"))
                     and second.get("authors") == top.get("authors"))
        close = top["match"]["score"] - second["match"]["score"] < 0.03
        if close and not same_book:
            return None
    return top


def lookup(client: AudibleClient, hints: dict, region: str, limit: int = 10) -> List[dict]:
    """Search strategy: ASIN -> title+author -> keywords; results ranked."""
    asin = hints.get("asin")
    if asin:
        try:
            return rank([client.product(asin, region)], {**hints, "query": ""})
        except AudibleError:
            pass
    results: Dict[str, dict] = {}
    if hints.get("title") and hints.get("author"):
        for b in client.search(region, title=hints["title"], author=hints["author"], limit=limit):
            results.setdefault(b["asin"], b)
    query = hints.get("query") or " ".join(filter(None, [hints.get("title"), hints.get("author")]))
    if query and len(results) < 3:
        for b in client.search(region, keywords=query, limit=limit):
            results.setdefault(b["asin"], b)
    if not results and query:
        # Last resort: fewer words (drop the first chunk, often the author or a number)
        words = query.split()
        if len(words) > 3:
            for b in client.search(region, keywords=" ".join(words[-3:]), limit=limit):
                results.setdefault(b["asin"], b)
    if not results and hints.get("alt_query"):
        # e.g. the term typed into the search when the release name led nowhere: then it is the query
        for b in client.search(region, keywords=hints["alt_query"], limit=limit):
            results.setdefault(b["asin"], b)
        hints = {**hints, "query": hints["alt_query"]}
    return rank(list(results.values()), hints)
