"""Search Newznab indexers and send NZBs to SABnzbd – the Readarr part.

Indexers are plain Newznab APIs and optional – without one only the search is
missing, SABnzbd and the inbox work the same.  SABnzbd downloads into a category whose
folder is the audiobook inbox, so finished downloads flow straight into the
pipeline.  API keys stay on the server: the UI only ever sees whether one is
set, and NZB links (which embed the indexer key) never leave the backend.
"""
from __future__ import annotations

import json
import re
import threading
import time
import unicodedata
import uuid
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor
from email.utils import parsedate_to_datetime
from typing import Callable, Dict, List, Optional
from urllib import parse, request
from urllib.error import HTTPError, URLError

import db as dbm
from i18n import t

NEWZNAB_NS = "{http://www.newznab.com/DTD/2010/feeds/attributes/}"
DEFAULT_CATEGORIES = "3030"          # Newznab: Audio > Audiobook
USER_AGENT = "Hoerwerk/5"


class UsenetError(Exception):
    pass


MAX_RESPONSE = 20 * 1024 * 1024


def _get(url: str, timeout: float = 25) -> bytes:
    req = request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with request.urlopen(req, timeout=timeout) as resp:
            data = resp.read(MAX_RESPONSE + 1)
            if len(data) > MAX_RESPONSE:
                raise UsenetError(t("Antwort zu groß"))
            return data
    except HTTPError as e:
        raise UsenetError(f"HTTP {e.code}") from e
    except (URLError, TimeoutError, OSError) as e:
        raise UsenetError(t("nicht erreichbar ({reason})", reason=getattr(e, "reason", e))) from e


def _redact(text: str) -> str:
    return re.sub(r"(?i)(apikey|api_key|r|i)=[0-9a-z]{6,}", r"\1=…", text)


# --------------------------------------------------------------------------- #
# Release names
# --------------------------------------------------------------------------- #

def release_info(title: str) -> dict:
    """What a release name tells us: format, language, bitrate, abridged …"""
    t = title.replace("_", " ").replace(".", " ")
    fmt = next((f for f in ("M4B", "M4A", "MP3", "FLAC", "AAC", "OPUS", "OGG") if re.search(rf"(?i)\b{f}\b", t)), None)
    lang = None
    if re.search(r"(?i)\b(german|deutsch|ger)\b|(?<![a-z])DE(?![a-z])", t):
        lang = "de"
    elif re.search(r"(?i)\b(english|eng)\b|(?<![a-z])EN(?![a-z])", t):
        lang = "en"
    br = re.search(r"(?i)\b(\d{2,3})\s*k(?:bps|bit)?\b", t)
    return {
        "format": fmt,
        "language": lang,
        "bitrate": int(br.group(1)) if br else None,
        "abridged": bool(re.search(r"(?i)\bgek(ü|ue)rzt|\babridged", t)) and not re.search(r"(?i)ungek(ü|ue)rzt|unabridged", t),
        "drama": bool(re.search(r"(?i)h(ö|oe)rspiel|dramati[sz]", t)),
    }


# --------------------------------------------------------------------------- #
# German titles: umlauts in release names
# --------------------------------------------------------------------------- #

_DIGRAPHS = {"ä": "ae", "ö": "oe", "ü": "ue", "Ä": "Ae", "Ö": "Oe", "Ü": "Ue", "ß": "ss",
             "ø": "oe", "Ø": "Oe", "å": "aa", "Å": "Aa", "æ": "ae", "Æ": "Ae", "œ": "oe", "Œ": "Oe"}
_PLAIN = str.maketrans({"ä": "a", "ö": "o", "ü": "u", "Ä": "A", "Ö": "O", "Ü": "U"})
# letters that Unicode does not split into letter + accent: "Nesbø" -> "Nesbo", "Łódź" -> "Lodz"
_SPECIAL = str.maketrans({"ø": "o", "Ø": "O", "æ": "ae", "Æ": "Ae", "œ": "oe", "Œ": "Oe", "ł": "l", "Ł": "L",
                          "đ": "d", "Đ": "D", "ð": "d", "þ": "th", "Þ": "Th", "ı": "i"})


def _no_accents(s: str) -> str:
    s = unicodedata.normalize("NFKD", s.translate(_SPECIAL))
    return "".join(c for c in s if not unicodedata.combining(c))


def query_variants(query: str) -> List[str]:
    """Release names spell "Fräulein" as "Fräulein", "Fraeulein" or "Fraulein", "Nesbø" as "Nesbø", "Nesboe" or
    "Nesbo" and "Céline" as "Céline" or "Celine" – search every spelling."""
    q = " ".join(query.split())
    out = [q]
    if re.search(r"[^\x00-\x7f]", q):
        out.append(_no_accents("".join(_DIGRAPHS.get(c, c) for c in q)))
        out.append(_no_accents(q.replace("ß", "ss").translate(_PLAIN)))
    return list(dict.fromkeys(out))


def match_words(text: str) -> List[str]:
    """Spelling-insensitive words: "Fräulein", "Fraeulein", "Fraulein" and "FRAULEIN" all become "fraulein"."""
    s = re.sub(r"(?<=\w)\((\w{1,3})\)(?=\W|$)", r"\1", text or "")    # "Bis(s)" is one word: "biss"
    s = s.lower().replace("ß", "ss").translate(_PLAIN).translate(_SPECIAL)
    s = re.sub(r"([aou])e", r"\1", s)
    s = _no_accents(s)
    return re.findall(r"[a-z0-9]+", s)


# --------------------------------------------------------------------------- #
# Newznab
# --------------------------------------------------------------------------- #

def api_url(base: str) -> str:
    base = base.strip().rstrip("/")
    if not re.match(r"(?i)^https?://", base):
        base = "http://" + base
    return base if base.lower().endswith("/api") else base + "/api"


def _check_error(root: ET.Element) -> None:
    if root.tag == "error":
        raise UsenetError(root.get("description") or t("Fehler {code}", code=root.get("code")))


def _xml(data: bytes) -> ET.Element:
    """Parse an indexer answer; DTDs/entities are refused (entity-expansion bombs, XXE)."""
    if re.search(rb"<!(DOCTYPE|ENTITY)", data[:4096], re.I) or b"<!ENTITY" in data:
        raise UsenetError(t("Antwort enthält eine DTD – abgelehnt"))
    return ET.fromstring(data)


def _http_url(url) -> Optional[str]:
    return url if isinstance(url, str) and re.match(r"(?i)^https?://[^\s\"'<>]+$", url) else None


def safe_release_name(title: str) -> str:
    """Name SABnzbd gets for the download folder: no paths, no hidden or relative names."""
    name = re.sub(r'[\\/:*?"<>|\x00-\x1f]', "_", title).strip(" .")
    return name[:200] or "download"


def parse_results(xml: bytes, indexer: str) -> List[dict]:
    try:
        root = _xml(xml)
    except ET.ParseError as e:
        raise UsenetError(t("Antwort ist kein Newznab-XML")) from e
    _check_error(root)
    out = []
    for item in root.iter("item"):
        attrs = {a.get("name"): a.get("value") for a in item.findall(f"{NEWZNAB_NS}attr")}
        cats = [a.get("value") for a in item.findall(f"{NEWZNAB_NS}attr") if a.get("name") == "category" and a.get("value")]
        enc = item.find("enclosure")
        link = (enc.get("url") if enc is not None else None) or item.findtext("link")
        if not _http_url(link):
            continue
        size = int(attrs.get("size") or (enc.get("length") if enc is not None else 0) or 0)
        pub = item.findtext("pubDate")
        try:
            ts = parsedate_to_datetime(pub).timestamp() if pub else None
        except (TypeError, ValueError):
            ts = None
        title = (item.findtext("title") or "").strip()
        out.append({
            "title": title,
            "link": link,
            "guid": item.findtext("guid") or link,
            "size": size,
            "published": ts,
            "grabs": int(attrs["grabs"]) if (attrs.get("grabs") or "").isdigit() else None,
            "category": attrs.get("category") or item.findtext("category"),
            "categories": cats,
            "category_name": (item.findtext("category") or "").strip(),
            "details": _http_url(item.findtext("comments")) or _http_url(item.findtext("guid")),
            "indexer": indexer,
            **release_info(title),
        })
    return out


AUDIOBOOK_NAME = re.compile(r"(?i)audio\s*-?\s*book|h(?:ö|oe|o)rbu(?:ch|echer|cher)|h(?:ö|oe|o)rspiel|spoken|abook")
_CAPS: Dict[str, tuple] = {}            # indexer url -> (time, audiobook category ids)
CAPS_TTL = 24 * 3600


AUDIO_NAME = re.compile(r"(?i)\baudio\b|\bmusi[ck]\b|\bmp3\b|lossless|flac")


def audiobook_categories(caps_xml: bytes) -> List[str]:
    """Category ids where an indexer files audiobooks, read from its capabilities (t=caps).

    Not only "Audiobook": old free readings (Goethe & co.) often sit under "Audio > Lossless" or
    "Audio > MP3". So the whole audio area counts – the standard 3000 range, every main category called
    "Audio" with all its sub-categories, and the indexer's own "Hörbuch"/"Hörspiel" categories anywhere.
    The release verdict (author and title) sorts out the music albums that come along."""
    root = _xml(caps_xml)
    out = []
    for cat in root.iter("category"):
        cid, name = cat.get("id") or "", cat.get("name") or ""
        audio_main = cid.isdigit() and (3000 <= int(cid) < 4000 or AUDIO_NAME.search(name))
        if cid.isdigit() and (audio_main or AUDIOBOOK_NAME.search(name)):
            out.append(cid)
        for sub in cat.findall("subcat"):
            sid = sub.get("id") or ""
            if sid.isdigit() and (audio_main or AUDIOBOOK_NAME.search(sub.get("name") or "")):
                out.append(sid)
    return list(dict.fromkeys(out))


def is_audio(result: dict, cats: List[str]) -> bool:
    """Result from a search over all categories: keep it only if it is in the audio area."""
    ids = [str(c) for c in (result.get("categories") or [result.get("category")]) if c]
    if any(c in cats for c in ids) or any(c.isdigit() and 3000 <= int(c) < 4000 for c in ids):
        return True
    name = result.get("category_name") or ""
    if name:
        return bool(AUDIOBOOK_NAME.search(name) or AUDIO_NAME.search(name))
    return not ids                      # the indexer says nothing about the category: let the verdict decide


class Newznab:
    def __init__(self, name: str, url: str, apikey: str, categories: str = DEFAULT_CATEGORIES,
                 get: Callable[[str], bytes] = _get) -> None:
        self.name, self.url, self.apikey = name, api_url(url), apikey
        self.categories = re.sub(r"[^\d,]", "", categories or DEFAULT_CATEGORIES) or DEFAULT_CATEGORIES
        self._get = get

    def _call(self, **params) -> bytes:
        params = {k: v for k, v in params.items() if v not in (None, "")}
        params["apikey"] = self.apikey
        try:
            return self._get(f"{self.url}?{parse.urlencode(params)}")
        except UsenetError as e:
            raise UsenetError(f"{self.name}: {_redact(str(e))}") from e

    def test(self) -> str:
        raw = self._call(t="caps")
        root = _xml(raw)
        _check_error(root)
        server = root.find("server")
        title = (server.get("title") if server is not None else None) or "OK"
        found = audiobook_categories(raw)
        _CAPS[self.url] = (time.time(), found)
        cats = list(dict.fromkeys([c for c in self.categories.split(",") if c] + found))
        return t("{title} · Hörbuch-Kategorien: {cats}", title=title, cats=", ".join(cats))

    def audiobook_cats(self) -> List[str]:
        """The configured categories plus the indexer's own audiobook categories (asked once a day)."""
        own = [c for c in self.categories.split(",") if c]
        hit = _CAPS.get(self.url)
        if not hit or time.time() - hit[0] > CAPS_TTL:
            try:
                found = audiobook_categories(self._call(t="caps"))
            except (UsenetError, ET.ParseError):
                found = []
            hit = (time.time(), found)
            _CAPS[self.url] = hit
        return list(dict.fromkeys(own + hit[1] + ["3000"]))   # 3000 = the whole audio area (Newznab standard)

    def search(self, query: str, limit: int = 100) -> List[dict]:
        cats = self.audiobook_cats()
        found = parse_results(self._call(t="search", q=query, cat=",".join(cats), limit=limit, extended=1), self.name)
        if found:
            return found
        # Nothing in the audiobook categories: many indexers file German audiobooks elsewhere –
        # search all categories and keep what lies in the audio area (the verdict checks the rest)
        everything = parse_results(self._call(t="search", q=query, limit=limit, extended=1), self.name)
        return [r for r in everything if is_audio(r, cats)]


# --------------------------------------------------------------------------- #
# SABnzbd
# --------------------------------------------------------------------------- #

class Sabnzbd:
    def __init__(self, url: str, apikey: str, category: str = "",
                 get: Callable[[str], bytes] = _get) -> None:
        url = url.strip().rstrip("/")
        if not re.match(r"(?i)^https?://", url):
            url = "http://" + url
        self.url = url if url.endswith("/api") else url + "/api"
        self.apikey, self.category = apikey, category.strip()
        self._get = get

    def _call(self, **params) -> dict:
        params = {k: v for k, v in params.items() if v not in (None, "")}
        params.update(apikey=self.apikey, output="json")
        try:
            data = json.loads(self._get(f"{self.url}?{parse.urlencode(params)}"))
        except UsenetError as e:
            raise UsenetError(f"SABnzbd: {_redact(str(e))}") from e
        except ValueError as e:
            raise UsenetError(t("SABnzbd: unerwartete Antwort")) from e
        if isinstance(data, dict) and data.get("status") is False and data.get("error"):
            raise UsenetError(f"SABnzbd: {data['error']}")
        return data

    def test(self) -> str:
        version = self._call(mode="version").get("version")
        cats = self._call(mode="get_cats").get("categories") or []
        if self.category and self.category not in cats:
            raise UsenetError(t("SABnzbd: Kategorie „{category}“ gibt es nicht (vorhanden: {available})", category=self.category, available=", ".join(cats)))
        return f"SABnzbd {version}"

    def add(self, nzb_url: str, name: str) -> str:
        data = self._call(mode="addurl", name=nzb_url, nzbname=name, cat=self.category or None)
        ids = data.get("nzo_ids") or []
        if not data.get("status") or not ids:
            raise UsenetError(t("SABnzbd hat den Download nicht angenommen"))
        return ids[0]

    def activity(self, limit: int = 15) -> dict:
        queue = self._call(mode="queue", limit=50).get("queue") or {}
        hist = self._call(mode="history", limit=limit, category=self.category or None).get("history") or {}
        items = []
        for s in queue.get("slots") or []:
            if self.category and s.get("cat") not in (self.category, None):
                continue
            items.append({"id": s.get("nzo_id"), "name": s.get("filename"), "status": s.get("status"),
                          "progress": float(s.get("percentage") or 0) / 100,
                          "size": s.get("size"), "left": s.get("timeleft"), "done": False})
        for s in hist.get("slots") or []:
            items.append({"id": s.get("nzo_id"), "name": s.get("name"), "status": s.get("status"),
                          "progress": 1.0 if s.get("status") == "Completed" else 0.0,
                          "size": s.get("size"), "error": s.get("fail_message") or None,
                          "path": s.get("storage"), "done": True, "finished": s.get("completed")})
        return {"paused": bool(queue.get("paused")), "speed": queue.get("speed"), "items": items}


# --------------------------------------------------------------------------- #
# Configuration + search service
# --------------------------------------------------------------------------- #

SECRET = "apikey"


def _host(url) -> str:
    """scheme://host:port of a configured URL – keys are bound to it."""
    if not url:
        return ""
    u = str(url).strip()
    if not re.match(r"(?i)^https?://", u):
        u = "http://" + u
    p = parse.urlparse(u)
    return f"{p.scheme}://{(p.hostname or '').lower()}:{p.port or ''}"


class Usenet:
    """Indexers + SABnzbd (stored in the database, keys never leave the server), search cache, grab log."""

    def __init__(self, db: dbm.Database, get: Callable[[str], bytes] = _get) -> None:
        self.db = db
        self._get = get
        self._lock = threading.Lock()
        self._results: Dict[str, dict] = {}     # result id -> result incl. link (never sent to the UI)

    # -- config ------------------------------------------------------------
    def _load(self) -> dict:
        data = self.db.get("sources") or {}
        data = data if isinstance(data, dict) else {}
        data.setdefault("indexers", [])
        data.setdefault("sab", {})
        return data

    def _save(self, data: dict) -> None:
        self.db.put("sources", data)

    def public_config(self) -> dict:
        """Config for the UI – keys replaced by a flag."""
        data = self._load()
        idx = [{**{k: v for k, v in i.items() if k != SECRET}, "apikey_set": bool(i.get(SECRET))}
               for i in data["indexers"]]
        sab = {**{k: v for k, v in data["sab"].items() if k != SECRET}, "apikey_set": bool(data["sab"].get(SECRET))}
        return {"indexers": idx, "sab": sab, "configured": self.configured()}

    def update_config(self, new: dict) -> dict:
        """Empty or missing API keys keep the stored ones."""
        with self._lock:
            old = self._load()
            old_idx = {i.get("id"): i for i in old["indexers"]}
            indexers = []
            for i in new.get("indexers") or []:
                if not isinstance(i, dict) or not str(i.get("url") or "").strip():
                    continue
                prev = old_idx.get(i.get("id"), {})
                if _host(prev.get("url")) != _host(i["url"]):
                    prev = {}     # new address: the stored key must not travel to another server
                indexers.append({
                    "id": i.get("id") or uuid.uuid4().hex[:8],
                    "name": str(i.get("name") or "").strip() or parse.urlparse(api_url(i["url"])).hostname or "Indexer",
                    "url": str(i["url"]).strip(),
                    "categories": re.sub(r"[^\d,]", "", str(i.get("categories") or DEFAULT_CATEGORIES)) or DEFAULT_CATEGORIES,
                    "enabled": bool(i.get("enabled", True)),
                    SECRET: str(i.get(SECRET) or "").strip() or prev.get(SECRET, ""),
                })
            sab_new = new.get("sab") or {}
            sab_url = str(sab_new.get("url") or "").strip()
            old_key = old["sab"].get(SECRET, "") if _host(old["sab"].get("url")) == _host(sab_url) else ""
            sab = {
                "url": sab_url,
                "category": str(sab_new.get("category") or "").strip(),
                SECRET: str(sab_new.get(SECRET) or "").strip() or old_key,
            }
            self._save({"indexers": indexers, "sab": sab})
        return self.public_config()

    def configured(self) -> dict:
        data = self._load()
        return {"indexers": sum(1 for i in data["indexers"] if i.get("enabled", True) and i.get(SECRET)),
                "sab": bool(data["sab"].get("url") and data["sab"].get(SECRET))}

    def _indexers(self) -> List[Newznab]:
        return [Newznab(i["name"], i["url"], i.get(SECRET, ""), i.get("categories"), self._get)
                for i in self._load()["indexers"] if i.get("enabled", True)]

    def _sab(self) -> Sabnzbd:
        s = self._load()["sab"]
        if not s.get("url") or not s.get(SECRET):
            raise UsenetError(t("SABnzbd ist noch nicht eingerichtet"))
        return Sabnzbd(s["url"], s[SECRET], s.get("category", ""), self._get)

    def test(self, kind: str, index_id: str = "") -> str:
        if kind == "sab":
            return self._sab().test()
        for i in self._load()["indexers"]:
            if i.get("id") == index_id:
                return Newznab(i["name"], i["url"], i.get(SECRET, ""), i.get("categories"), self._get).test()
        raise UsenetError(t("Indexer nicht gefunden"))

    # -- search ------------------------------------------------------------
    def search(self, query: str, language: str = "", extra: Optional[List[str]] = None,
               variants: bool = True, limit: int = 6) -> dict:
        """Search every indexer for the query (plus ``extra`` queries), in all umlaut spellings
        (``variants=False``: exactly these queries – the caller made the spellings itself)."""
        indexers = self._indexers()
        if not indexers:
            raise UsenetError(t("Noch kein Indexer eingerichtet"))
        queries: List[str] = []
        for q in [query] + [e for e in extra or [] if e and e.strip()]:
            queries += query_variants(q) if variants else [" ".join(q.split())]
        queries = list(dict.fromkeys(queries))[:max(1, limit)]
        results, errors = [], {}
        with ThreadPoolExecutor(max_workers=min(8, len(indexers) * len(queries))) as pool:
            jobs = [(i, pool.submit(i.search, q)) for i in indexers for q in queries]
            for idx, fut in jobs:
                try:
                    results += fut.result()
                except (UsenetError, ET.ParseError) as e:
                    errors[idx.name] = str(e) if str(e).startswith(idx.name) else f"{idx.name}: {e}"
        # one entry per release (several indexers often carry the same one)
        seen: Dict[str, dict] = {}
        for r in results:
            key = re.sub(r"[\W_]+", "", r["title"].lower()) + str(r["size"] // (1 << 20))
            if key in seen:
                if r["indexer"] != seen[key]["indexer"] and r["indexer"] not in seen[key].get("also", []):
                    seen[key]["also"] = seen[key].get("also", []) + [r["indexer"]]
                continue
            seen[key] = r
        wanted = " ".join([query] + list(extra or []))
        ranked = sorted(seen.values(), key=lambda r: self._rank(r, wanted, language), reverse=True)
        with self._lock:
            if len(self._results) > 2000:
                self._results.clear()
            out = []
            for r in ranked:
                rid = uuid.uuid4().hex[:12]
                self._results[rid] = {**r, "query": query}
                out.append({**{k: v for k, v in r.items() if k != "link"}, "id": rid})
        return {"results": out, "errors": list(errors.values()), "queries": queries}

    def search_stages(self, stages: List[List[str]], fits: Callable[[List[dict]], bool], language: str = "",
                      budget: int = 9) -> dict:
        """Search round by round (see ``releases.search_stages``) until ``fits`` likes the results so far.
        All spellings of a query count against the budget – a book never costs more than ``budget`` queries
        per indexer. A round in which every indexer failed ends the search: the next one would fail as well."""
        results: Dict[str, dict] = {}
        errors: List[str] = []
        asked: List[str] = []
        rounds = 0
        for stage in stages:
            todo = [v for q in stage for v in query_variants(q)]
            todo = [q for q in dict.fromkeys(todo) if q not in asked][:budget - len(asked)]
            if not todo:
                continue
            res = self.search(todo[0], language, todo[1:], variants=False, limit=len(todo))
            rounds += 1
            asked += todo
            errors += [e for e in res["errors"] if e not in errors]
            for r in res["results"]:
                results.setdefault(re.sub(r"[\W_]+", "", r["title"].lower()) + str(r["size"] // (1 << 20)), r)
            found = list(results.values())
            all_failed = not res["results"] and len(res["errors"]) >= len(self._indexers())
            if (found and fits(found)) or all_failed or len(asked) >= budget:
                break
        return {"results": list(results.values()), "errors": errors, "queries": asked, "rounds": rounds}

    @staticmethod
    def _rank(r: dict, query: str, language: str) -> tuple:
        words = list(dict.fromkeys(w for w in match_words(query) if len(w) > 2))
        title = " ".join(match_words(r["title"]))
        hits = sum(1 for w in words if w in title) / max(1, len(words))
        lang = 1 if language and r.get("language") == language else 0 if not r.get("language") else -1 if language else 0
        fmt = {"M4B": 2, "M4A": 1, "MP3": 1, "FLAC": 1}.get(r.get("format") or "", 0)
        return (round(hits, 1), lang, -int(r.get("abridged") or 0), fmt, r.get("grabs") or 0, r.get("published") or 0)

    # -- grab --------------------------------------------------------------
    def send(self, result_id: str, hints: Optional[dict] = None) -> dict:
        with self._lock:
            r = self._results.get(result_id)
        if not r:
            raise UsenetError(t("Suchergebnis abgelaufen – bitte neu suchen"))
        name = safe_release_name(r["title"])
        nzo = self._sab().add(r["link"], name)
        self._remember(name, {"query": r.get("query"), **(hints or {}), "nzo_id": nzo, "time": time.time()})
        return {"nzo_id": nzo, "title": name}

    def _remember(self, name: str, info: dict) -> None:
        with self.db.tx() as c:
            dbm.save_grab(c, name, info)
            # keep the newest 500
            c.execute("DELETE FROM grabs WHERE name NOT IN (SELECT name FROM grabs ORDER BY time DESC LIMIT 500)")

    def hint_for(self, folder_name: str) -> Optional[dict]:
        """Search info for a download folder (SABnzbd names it after the release)."""
        row = self.db.one("SELECT data FROM grabs WHERE key=? ORDER BY time DESC LIMIT 1",
                          (dbm.grab_key(folder_name),))
        return dbm.loads(row["data"]) if row else None

    def activity(self) -> dict:
        return self._sab().activity()
