"""Indexer search -> SABnzbd, with the library cross-check."""
from __future__ import annotations

import re
from typing import List, Optional

import audible
import releases
import bookrequests as br
import links
import packs
import usenet
from abs import AbsError
from plex import PlexError
from usenet import UsenetError
from i18n import t
from web.router import HTTPException, Req, route


def _book_key(b: dict) -> str:
    first = (b.get("authors") or [""])[0].split()
    return f"{' '.join(usenet.match_words(b.get('title') or ''))}|{(usenet.match_words(first[-1]) or [''])[0] if first else ''}"


def _owned(entries: List[dict]) -> Optional[dict]:
    if not entries:
        return None
    e = entries[0]
    return {"path": e["path"], "title": e.get("title"), "authors": e.get("authors"),
            "why": e.get("why", "similar"), "count": len(entries)}


@route("GET", "/api/sources")
def sources(req: Req):
    return req.app.usenet.public_config() | {"abs": req.app.abs.public_config(), "plex": req.app.plex.public_config()}


@route("PUT", "/api/sources")
def write_sources(req: Req):
    if not isinstance(req.body, dict):
        raise HTTPException(400, t("Ungültige Daten"))
    if isinstance(req.body.get("abs"), dict):
        req.app.abs.update_config(req.body["abs"])
    if isinstance(req.body.get("plex"), dict):
        try:
            req.app.plex.update_config(req.body["plex"])
        except PlexError as e:
            raise HTTPException(400, str(e))
    return req.app.usenet.update_config(req.body) | {"abs": req.app.abs.public_config(), "plex": req.app.plex.public_config()}


@route("POST", "/api/sources/test")
def test_source(req: Req):
    try:
        if req.get("kind") == "abs":
            return {"ok": True, "message": req.app.abs.test()}
        if req.get("kind") == "plex":
            return {"ok": True, "message": req.app.plex.test()}
        return {"ok": True, "message": req.app.usenet.test(req.get("kind", ""), req.get("id", ""))}
    except (UsenetError, AbsError, PlexError) as e:
        return {"ok": False, "message": str(e)}
    except Exception as e:  # noqa: BLE001 – unexpected answers (HTML instead of XML …)
        return {"ok": False, "message": t("Unerwartete Antwort: {error}", error=e)}


@route("POST", "/api/search")
def search_indexers(req: Req):
    query = (req.get("query") or "").strip()
    if len(query) < 2:
        raise HTTPException(400, t("Suchbegriff fehlt"))
    extra = [str(x) for x in (req.get("extra") or []) if isinstance(x, str)][:3]
    try:
        res = req.app.usenet.search(query, req.get("language") or "", extra)
    except UsenetError as e:
        raise HTTPException(400, str(e))
    index = req.app.library_index
    for r in res["results"]:
        r["owned"] = _owned(index.match_release(r["title"]))
        pack = packs.pack_info(r["title"])
        if pack:
            # several volumes in one download: which of them are here already?
            owned = index.series_owned(pack["series"], pack["author"])
            r["pack"] = {**pack, "owned": {str(v): owned[v] for v in pack["volumes"] if v in owned}}
            r["owned"] = None
    # what the user typed may already be in the library, even if no release name says so
    res["library"] = [{k: e.get(k) for k in ("path", "title", "authors", "format")}
                      for e in index.match_release(query, lenient=True)[:3]]
    return res


@route("POST", "/api/search/books", access="user")
def search_books(req: Req):
    """Look the book up at Audible first – German title and author as the shop lists them."""
    query = (req.get("query") or "").strip()
    if len(query) < 2:
        raise HTTPException(400, t("Suchbegriff fehlt"))
    region = req.get("region") or req.app.settings()["ab_region"]
    if region not in audible.REGIONS:
        raise HTTPException(400, t("Unbekannte Region"))
    language = req.get("language") or ""             # "de" / "en" / "" – which edition is wanted
    if language not in ("", "de", "en"):
        raise HTTPException(400, t("Ungültige Daten"))
    hints = {"query": audible.clean_query(query) or query, "asin": audible.find_asin(query)}
    error = None
    try:
        books = req.app.pipeline.providers.search(hints, region)[:8]
    except audible.AudibleError as e:
        books, error = [], str(e)
    # BookBeat as a second shop: only what Audible doesn't have (same title and author)
    seen = {_book_key(b) for b in books}
    extra = [b for b in links.bookbeat_search(hints["query"], region)
             if _book_key(b) not in seen and (b.get("language") or "german") in ("german", "deutsch")]
    books += audible.rank(extra, hints)[:6]
    if language:
        wanted = [b for b in books if releases.book_language(b) in (language, "")]
        if not wanted and language == "en" and region not in ("us", "uk"):
            # an English edition the German store doesn't list: ask the US store
            try:
                wanted = [b for b in req.app.pipeline.providers.search(hints, "us")[:8] if releases.book_language(b) == "en"]
            except audible.AudibleError:
                wanted = []
        books = wanted
    if not books and error:
        raise HTTPException(502, error)
    return {"books": _for_browser(req, books), "region": region}


BOOK_OUT = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_asin", "series_position",
            "runtime_min", "release_date", "cover_url", "format_type", "url", "source", "language", "isbn", "id",
            "region", "genres")


def _for_browser(req: Req, books: List[dict]) -> List[dict]:
    requested = req.app.requests.status_by_key()
    out = []
    for b in books:
        item = {k: b.get(k) for k in BOOK_OUT}
        owned = _owned(req.app.library_index.find(b))
        # family accounts learn *that* a book is there, not where it lies on the server
        item["owned"] = owned if req.is_admin else ({"title": owned["title"]} if owned else None)
        item["requested"] = requested.get(br.book_key(b))
        out.append(item)
    return out


SERIES_ASIN = re.compile(r"^[A-Z0-9]{10}$")


@route("POST", "/api/search/series", access="user")
def search_series(req: Req):
    """All released volumes of a series (from the store the book came from), in order – for a series request."""
    asin = str(req.get("asin") or "").strip().upper()
    region = req.get("region") or req.app.settings()["ab_region"]
    language = {"de": "german", "en": "english"}.get(req.get("language") or "", "")
    if not SERIES_ASIN.match(asin) or region not in audible.REGIONS:
        raise HTTPException(400, t("Ungültige Daten"))
    try:
        series = req.app.pipeline.client.series(asin, region, language)
    except audible.AudibleError as e:
        raise HTTPException(502, str(e))
    return {"series": {"asin": series["asin"], "title": series["title"], "region": region},
            "books": _for_browser(req, series["books"])}


@route("POST", "/api/search/send")
def send_to_sab(req: Req):
    hints = {k: req.get(k) for k in ("title", "author", "asin") if req.get(k)}
    volumes = req.get("volumes")
    if volumes is not None:
        if not isinstance(volumes, list) or not volumes or not all(isinstance(v, int) and 0 <= v < 1000 for v in volumes):
            raise HTTPException(400, t("Bitte mindestens einen Band auswählen"))
        hints["volumes"] = sorted(set(volumes))
    try:
        res = req.app.usenet.send(req.get("id", ""), hints)
    except UsenetError as e:
        raise HTTPException(400, str(e))
    n = req.app.notifier
    n.send("download_sent", t("Download an SABnzbd geschickt"), res["title"])
    return res


@route("GET", "/api/downloads")
def downloads(req: Req):
    usenet = req.app.usenet
    if not usenet.configured()["sab"]:
        return {"configured": False, "items": []}
    try:
        return {"configured": True, **usenet.activity()}
    except UsenetError as e:
        return {"configured": True, "error": str(e), "items": []}
