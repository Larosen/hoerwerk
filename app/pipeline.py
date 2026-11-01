"""Audiobook pipeline: watch an inbox, convert to M4B, tag from Audible, file into the library.

auto-m4b (inbox -> m4b) and beets-audible (Audible tags -> library) in one
place, with a review queue in the web UI for books that cannot be matched
with confidence.
"""
from __future__ import annotations

import os
import re
import shutil
import threading
import time
import uuid
from typing import Callable, Dict, List, Optional, Tuple

import abs as absm
import audible
import db as dbm
import encoder
import library
import links
import packs
import providers as prov
import tagger
from meta import cache as meta_cache
from natsort import natural_key
import applog
from i18n import t

GRAB_SETTLE = 15        # seconds of quiet for downloads SABnzbd finished (they arrive in one move)

TEMP_SUFFIXES = (".part", ".!qb", ".crdownload", ".tmp", ".partial", ".download", ".filepart",
                 ".aria2", ".!ut", ".opdownload")
DISC_RX = re.compile(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)?[\s_.-]*\d+\b")
ACTIVE = ("queued", "matching", "converting", "tagging")
# A job runs in stages; each stage is one step in a queue of the job engine
STAGES = ("prepare", "convert", "finish")
STAGE_QUEUE = {"prepare": "io", "convert": "encode", "finish": "io"}
STAGE_STATUS = {"prepare": "matching", "convert": "converting", "finish": "tagging"}
# Things that have no business in an audiobook download. They are never copied into the
# library and never executed – but the user should know the download is suspicious.
SUSPICIOUS_EXTS = (".exe", ".com", ".bat", ".cmd", ".scr", ".pif", ".msi", ".dll", ".ps1", ".vbs",
                   ".vbe", ".js", ".jse", ".wsf", ".hta", ".lnk", ".jar", ".apk", ".sh", ".py",
                   ".app", ".dmg", ".iso", ".reg", ".docm", ".xlsm")
CANDIDATE_KEYS = ("asin", "title", "subtitle", "authors", "narrators", "series", "series_position",
                  "runtime_min", "release_date", "cover_url", "match", "region", "url", "format_type",
                  "publisher", "language", "genres", "description", "source", "isbn")
EDITABLE = ("title", "subtitle", "authors", "narrators", "series", "series_position", "release_date",
            "publisher", "genres", "description", "language")
SIDECARS = ("desc.txt", "reader.txt", "cover.jpg", "cover.png", absm.METADATA_FILE)


def candidate_id(book: dict) -> str:
    return book.get("id") or book.get("asin") or ""
FINAL = ("done", "failed", "cancelled")


def _now() -> float:
    return time.time()


# --------------------------------------------------------------------------- #
# Inbox scanning
# --------------------------------------------------------------------------- #

def _visible(name: str) -> bool:
    return not name.startswith((".", "_", "@eaDir", "#recycle"))


def signature(path: str) -> Optional[tuple]:
    """(files, bytes, newest mtime) – or None while something is still downloading."""
    count, size, newest = 0, 0, 0.0
    if os.path.islink(path):
        return (0, 0, 0.0)
    if os.path.isfile(path):
        walk = [(os.path.dirname(path), [], [os.path.basename(path)])]
    else:
        walk = os.walk(path)
    for root, dirs, files in walk:
        dirs[:] = [d for d in dirs if _visible(d) and not os.path.islink(os.path.join(root, d))]
        for f in files:
            if f.lower().endswith(TEMP_SUFFIXES):
                return None
            try:
                st = os.stat(os.path.join(root, f))
            except OSError:
                return None
            count += 1
            size += st.st_size
            newest = max(newest, st.st_mtime)
    return (count, size, newest)


_VOLUME_FOLDER = re.compile(r"(?i)\b(?:b(?:a|ä|ae)nde?|folgen?|vol(?:ume)?s?|books?|buch|episoden?|fall)\.?\s*\d")
_DISC_WORD = re.compile(r"(?i)^\s*(?:cd|disc|disk|dvd|teil|part|tape|seite|side|mc|kassette)[\s_.-]*\d+\b")


def are_discs(names: List[str]) -> bool:
    """Subfolders that are parts of one book (CD1, CD2 / 01, 02) – not volumes of a series.

    "01 - Todesurteil" and "02 - Kalte Asche" differ in more than the number: two books.
    "Band 1"/"Folge 2" are always volumes; "CD1"/"Disc 2"/"Teil 3" are always parts of one book.
    """
    if any(_VOLUME_FOLDER.search(n) for n in names):
        return False
    if all(_DISC_WORD.match(n) for n in names):
        return True
    return len({re.sub(r"\d+", "#", n).strip().lower() for n in names}) == 1


def book_units(path: str) -> List[str]:
    """Split an inbox entry into books.

    A folder with audio (or only CD1/CD2-like subfolders) is one book; a folder
    of book folders (e.g. "Autor/Buch 1", "Autor/Buch 2") becomes several.
    """
    if os.path.islink(path):
        return []
    if os.path.isfile(path):
        return [path] if encoder.is_audio(os.path.basename(path)) else []
    try:
        entries = sorted(os.listdir(path), key=natural_key)
    except OSError:
        return []
    direct = [e for e in entries if encoder.is_audio(e) and encoder.is_regular(os.path.join(path, e))]
    subs = [e for e in entries if _visible(e) and os.path.isdir(os.path.join(path, e))
            and not os.path.islink(os.path.join(path, e))
            and encoder.collect_audio(os.path.join(path, e))]
    if direct or not subs:
        return [path] if direct or subs else []
    if are_discs(subs):
        return [path]
    out: List[str] = []
    for s in subs:
        out += book_units(os.path.join(path, s))
    return out


# --------------------------------------------------------------------------- #
# Jobs
# --------------------------------------------------------------------------- #

class Job:
    KEYS = ("id", "source", "rel", "name", "origin", "status", "message", "progress", "files",
            "duration", "hints", "candidates", "match", "region", "m4b", "cover", "dest",
            "error", "log", "created", "updated", "converted_info", "duplicate", "dup_decision",
            "chapter_names", "rename_files", "stage", "pack")

    def __init__(self, **kw) -> None:
        self.id = kw.get("id") or uuid.uuid4().hex[:10]
        self.source: str = kw["source"]
        self.rel: str = kw.get("rel") or os.path.basename(self.source)
        self.name: str = kw.get("name") or os.path.splitext(os.path.basename(self.source))[0]
        self.origin: str = kw.get("origin", "inbox")        # inbox | manual
        self.status: str = kw.get("status", "queued")
        self.message: str = kw.get("message", t("Wartet …"))
        self.progress: float = kw.get("progress", 0.0)
        self.files: List[str] = kw.get("files") or []
        self.duration: float = kw.get("duration") or 0.0
        self.hints: dict = kw.get("hints") or {}
        self.candidates: List[dict] = kw.get("candidates") or []
        self.match: Optional[dict] = kw.get("match")
        self.region: Optional[str] = kw.get("region")
        self.m4b: Optional[str] = kw.get("m4b")
        self.cover: Optional[str] = kw.get("cover")
        self.dest: Optional[str] = kw.get("dest")
        self.error: Optional[str] = kw.get("error")
        self.log: List[str] = list(kw.get("log") or [])[-60:]
        self.created: float = kw.get("created") or _now()
        self.updated: float = kw.get("updated") or _now()
        self.converted_info: dict = kw.get("converted_info") or {}
        self.duplicate: Optional[dict] = kw.get("duplicate")        # library entry this book already exists as
        self.dup_decision: Optional[str] = kw.get("dup_decision")   # keep | replace
        self.chapter_names: Optional[List[str]] = kw.get("chapter_names")  # user's names, one per file
        self.rename_files: bool = bool(kw.get("rename_files"))       # rename the tracks to them first
        self.stage: str = kw.get("stage") if kw.get("stage") in STAGES else "prepare"
        self.pack: Optional[dict] = kw.get("pack")      # one volume of a multi-volume download
        self.cancel = False
        self.runner: Optional[encoder.Runner] = None

    def say(self, msg: str, progress: Optional[float] = None, level: str = "info") -> None:
        self.message = msg
        if progress is None or progress <= 0 or progress >= 1 or level != "info":   # not every percent of encoding
            if not self.log or not self.log[-1].endswith(msg):
                applog.log.add(msg, level, self.name)
        if progress is not None:
            self.progress = progress
        self.updated = _now()
        stamp = time.strftime("%H:%M:%S")
        if not self.log or not self.log[-1].endswith(msg):
            self.log.append(f"{stamp} {msg}")
            del self.log[:-60]

    def to_dict(self, full: bool = True) -> dict:
        d = {k: getattr(self, k) for k in self.KEYS}
        d["progress"] = round(self.progress, 4)
        if not full:
            d.pop("log")
            d["candidates"] = [{**{k: c.get(k) for k in CANDIDATE_KEYS}, "id": candidate_id(c)}
                               for c in self.candidates]
        return d


class Pipeline:
    def __init__(self, get_settings: Callable[[], dict], db: dbm.Database, inbox: str, library_root: str,
                 work_dir: str, client: Optional[audible.AudibleClient] = None,
                 jobs: int = 2, nice: str = "10", start: bool = False,
                 hint_provider: Optional[Callable[[str], Optional[dict]]] = None, index=None,
                 engine=None, notify: Optional[Callable[[], None]] = None,
                 providers: Optional[prov.Providers] = None,
                 on_filed: Optional[Callable[[str], None]] = None,
                 on_event: Optional[Callable[["Job"], None]] = None) -> None:
        self.on_filed = on_filed               # e.g. ask Audiobookshelf to scan
        self.on_event = on_event               # a job ended up done / review / failed (notifications)
        # open book requests [{id, user, key, book}] and "this job belongs to a request now" (set by the app)
        self.open_requests: Callable[[], list] = lambda: []
        self.on_request: Callable[["Job"], None] = lambda job: None
        self.engine = engine                   # Engine: runs the stages (None: tests call process())
        self.notify = notify or (lambda: None)  # live updates for the browser
        self.hint_provider = hint_provider     # search info for folders that came from a download
        self.index = index                     # LibraryIndex: what the library already holds
        self.get_settings = get_settings
        self.db = db
        self._stored: Dict[str, str] = {}      # job id -> JSON last written (only changed jobs are saved)
        self.inbox = os.path.realpath(inbox) if inbox else ""
        self.library = os.path.realpath(library_root) if library_root else ""
        self.work_dir = work_dir
        self.client = client or audible.AudibleClient()
        # Audible first; the other shops only when given (the app passes them, tests stay offline)
        self.providers = providers or prov.Providers(self.client, fallbacks=())
        self.cpu_jobs = max(1, jobs)
        self.nice = nice
        self.jobs: List[Job] = []
        self.lock = threading.RLock()
        self._save_lock = threading.Lock()
        self._file_lock = threading.Lock()     # one book at a time is checked against and filed into the library
        self.wake = threading.Condition(self.lock)
        self.seen: Dict[str, tuple] = {}     # inbox path -> (signature, first seen)
        self.ready_at: Dict[str, float] = {}  # inbox path -> when it counts as completely copied
        self.complete: set = set()            # inbox names known to be complete (uploads): no waiting
        self.last_scan: Optional[float] = None
        self.scan_note = ""
        self._force_scan = False
        self._load()
        if start:
            self.start()

    def start(self) -> None:
        """Watch the inbox and hand the waiting jobs to the engine."""
        with self.lock:
            waiting = sorted((j for j in self.jobs if j.status == "queued"), key=lambda j: j.created)
        for j in waiting:
            self._schedule(j)
        threading.Thread(target=self._scanner, daemon=True, name="inbox-scanner").start()

    # -- persistence (table "jobs") ------------------------------------------
    def _load(self) -> None:
        for row in self.db.all("SELECT data FROM jobs WHERE kind='audiobook' ORDER BY created"):
            d = dbm.loads(row["data"], {})
            if not isinstance(d, dict) or not d.get("source"):
                continue
            job = Job(**{k: d[k] for k in Job.KEYS if k in d})
            if "stage" not in d:               # written by version 3: derive the stage
                job.stage = "finish" if job.m4b and os.path.exists(job.m4b) else "prepare"
            if job.status in ACTIVE:           # interrupted by a restart: start that step again
                job.status = "queued"
                job.say(t("Nach Neustart erneut eingereiht"))
            self.jobs.append(job)
        self.save()

    def save(self) -> None:
        """Write the jobs that changed since the last save (one transaction)."""
        with self._save_lock:
            with self.lock:
                current = {j.id: (j, dbm.dumps(j.to_dict())) for j in self.jobs}
            with self.db.tx() as c:
                for jid in [i for i in self._stored if i not in current]:
                    c.execute("DELETE FROM jobs WHERE id=?", (jid,))
                    self._stored.pop(jid, None)
                for jid, (j, text) in current.items():
                    if self._stored.get(jid) == text:
                        continue
                    c.execute("INSERT OR REPLACE INTO jobs(id, kind, status, source, created, updated, data) "
                              "VALUES(?,?,?,?,?,?,?)",
                              (jid, "audiobook", j.status, j.source, j.created, j.updated, text))
                    self._stored[jid] = text
        self.notify()

    # -- public API --------------------------------------------------------
    def status(self) -> dict:
        s = self.get_settings()
        with self.lock:
            jobs = [j.to_dict(full=False) for j in sorted(self.jobs, key=lambda j: -j.created)]
        return {
            "enabled": bool(s.get("ab_watch")) and bool(self.inbox),
            "inbox": self.inbox, "library": self.library,
            "inbox_ok": bool(self.inbox) and os.path.isdir(self.inbox),
            "library_ok": bool(self.library) and os.path.isdir(self.library),
            "last_scan": self.last_scan, "scan_note": self.scan_note,
            "waiting": [{"name": os.path.relpath(p, self.inbox), "since": v[1],
                         "ready_in": max(0, round(self.ready_at.get(p, 0) - _now()))}
                        for p, v in list(self.seen.items()) if not self._known(p)],
            "jobs": jobs,
            "review": sum(1 for j in jobs if j["status"] == "review"),
        }

    def get(self, job_id: str) -> Job:
        with self.lock:
            for j in self.jobs:
                if j.id == job_id:
                    return j
        raise KeyError(job_id)

    def add(self, source: str, origin: str = "manual", chapter_names: Optional[List[str]] = None,
            rename_files: bool = False, pack: Optional[dict] = None) -> Job:
        """origin: inbox (watched folder), manual (from /data, originals untouched) or
        library (an older book in the library that gets converted and replaces itself)."""
        source = os.path.realpath(source)
        with self.lock:
            for j in self.jobs:
                if j.source == source and j.status not in FINAL:
                    return j
            if origin == "inbox":
                rel = os.path.relpath(source, self.inbox)
            elif origin == "library":
                rel = os.path.relpath(source, self.library)
            else:
                rel = os.path.basename(source)
            job = Job(source=source, rel=rel, origin=origin, chapter_names=chapter_names,
                      rename_files=rename_files, pack=pack)
            self.jobs.append(job)
            self._trim()
        self.save()
        self._schedule(job)
        return job

    def arrived(self, names: List[str]) -> None:
        """Entries that were moved into the inbox complete (an upload): take them at once."""
        with self.lock:
            self.complete.update(names)
        self.scan_now()

    def scan_now(self) -> None:
        with self.lock:
            self._force_scan = True
            self.wake.notify_all()

    def search(self, job_id: str, query: str, region: str) -> Job:
        if links.is_link(query):
            return self.add_link(job_id, query)[0]
        job = self.get(job_id)
        hints = {**job.hints, "query": audible.clean_query(query) or query, "title": "", "author": "",
                 "asin": audible.find_asin(query)}
        job.candidates = self.providers.search(hints, region)[:12]
        job.region = region
        job.say(t("{n} Treffer für „{query}“ ({region})", n=len(job.candidates), query=query, region=region))
        self.save()
        return job

    def add_link(self, job_id: str, url: str) -> tuple:
        """Book data from an Audible/BookBeat/Thalia link becomes the top candidate."""
        job = self.get(job_id)
        book = links.fetch(url, self.client)
        book = audible.rank([book], job.hints)[0]
        cid = candidate_id(book)
        job.candidates = [book] + [c for c in job.candidates if candidate_id(c) != cid]
        job.say(t("Daten aus Link: {title} ({source})", title=book["title"], source=book.get("source")))
        self.save()
        return job, book

    def candidate(self, job: Job, cand_id: str) -> Optional[dict]:
        return next((c for c in job.candidates if candidate_id(c) == cand_id), None)

    def choose(self, job_id: str, cand_id: str = "", asin: str = "", region: str = "") -> Job:
        job = self.get(job_id)
        book = self.candidate(job, cand_id) if cand_id else None
        if book is None and asin:
            book = next((c for c in job.candidates if c.get("asin") == asin and c.get("region") == region), None) \
                or self.client.product(asin, region)
        if book is None:
            raise KeyError(cand_id or asin)
        job.match = {k: v for k, v in book.items() if k not in ("match", "request_id", "request_user")}
        job.duplicate = job.dup_decision = None     # another book: check the library again
        job.say(t("Ausgewählt: {title} ({source})", title=book["title"], source=book.get("source") or "audible"))
        self._link_request(job, book)
        self._requeue_if_waiting(job)
        return job

    def manual(self, job_id: str, data: dict, base_id: str = "") -> Job:
        """The user's own data – on top of a candidate they edited, or of the file tags."""
        job = self.get(job_id)
        data = {k: data.get(k) for k in EDITABLE if k in data}
        for k in ("authors", "narrators", "genres"):
            if isinstance(data.get(k), str):
                data[k] = [x.strip() for x in data[k].split(",") if x.strip()]
        base = self.candidate(job, base_id) if base_id else None
        if base:
            # The form was prefilled with the candidate: what the user left there is what they want,
            # including fields they emptied (e.g. a wrong series)
            book = {**{k: v for k, v in base.items() if k != "match"},
                    **{k: (v or None) if k not in ("authors", "narrators", "genres") else v for k, v in data.items()},
                    "source": base.get("source") or "audible", "edited": True}
        else:
            current = tagger.read_book(job.m4b) if job.m4b and os.path.exists(job.m4b) else {}
            book = {**current, **{k: v for k, v in data.items() if v not in (None, "", [])}, "source": "manual"}
        if not book.get("title"):
            book["title"] = job.hints.get("title") or job.name
        if not book.get("authors") and job.hints.get("author"):
            book["authors"] = [job.hints["author"]]
        job.match = book
        job.duplicate = job.dup_decision = None
        job.say(t("Eigene Angaben (auf Basis von {source})", source=base.get("source")) if base else t("Eigene Angaben"))
        self._requeue_if_waiting(job)
        return job

    def reopen(self, job_id: str) -> Job:
        """Take a filed book back out of the library to match it again."""
        job = self.get(job_id)
        with self.lock:
            if job.status != "done" or not job.dest or not os.path.isfile(job.dest):
                raise ValueError(t("Nur abgelegte Bücher, die noch in der Bibliothek liegen, lassen sich korrigieren"))
            work = self._job_work(job)
            os.makedirs(work, exist_ok=True)
            folder = os.path.dirname(job.dest)
            m4b = os.path.join(work, "book.m4b")
            shutil.move(job.dest, m4b)
            for name in SIDECARS:
                p = os.path.join(folder, name)
                if os.path.isfile(p):
                    os.remove(p)
            _remove_empty_parents(folder, self.library)
            if self.index:
                self.index.remove(job.dest)
            job.m4b, job.dest, job.match = m4b, None, None
            job.duplicate = job.dup_decision = None
            job.status, job.stage = "review", "finish"
            job.say(t("Zur Korrektur aus der Bibliothek geholt"))
        self.save()
        return job

    def resolve_duplicate(self, job_id: str, action: str) -> Job:
        """The book is already in the library: keep both, replace the old one, or drop the new one."""
        job = self.get(job_id)
        if action not in ("keep", "replace", "discard"):
            raise ValueError(t("Unbekannte Aktion"))
        if not job.duplicate:
            raise ValueError(t("Für dieses Buch gibt es keinen Treffer in der Bibliothek"))
        if action == "discard":
            with self.lock:
                self._dispose_source(job, self.get_settings())
                self._cleanup_work(job)
                job.m4b = None
                job.status = "cancelled"
                job.say(t("Verworfen – schon in der Bibliothek: {path}", path=job.duplicate["path"]))
            self.save()
            return job
        job.dup_decision = action
        job.say(t("Trotzdem ablegen (zusätzlich)") if action == "keep" else t("Ersetzt {path}", path=job.duplicate["path"]))
        self._requeue_if_waiting(job)
        return job

    def retry(self, job_id: str) -> Job:
        job = self.get(job_id)
        with self.lock:
            if job.status not in ("failed", "cancelled"):
                return job
            failed_path = job.dest if job.dest and job.origin == "inbox" and os.path.exists(job.dest or "") else None
            if failed_path:
                back = os.path.join(self.inbox, job.rel)
                if not os.path.exists(back):
                    os.makedirs(os.path.dirname(back), exist_ok=True)
                    shutil.move(failed_path, back)
                    _remove_empty_parents(os.path.dirname(failed_path), self.inbox)
                    note = os.path.join(back, "FEHLER.txt")
                    if os.path.isfile(note):
                        os.remove(note)
                job.source = back
                job.dest = None
            job.status, job.error, job.progress, job.stage = "queued", None, 0.0, "prepare"
            job.m4b = None
            job.say(t("Erneut eingereiht"))
        self.save()
        self._schedule(job)
        return job

    def cancel(self, job_id: str) -> Job:
        job = self.get(job_id)
        with self.lock:
            job.cancel = True
            if job.runner:
                job.runner.kill()
            if job.status in ("queued", "review"):
                job.status = "cancelled"
                job.say(t("Abgebrochen"))
                self._cleanup_work(job)
        if self.engine:
            self.engine.discard(job.id)
        self.save()
        return job

    def remove(self, job_id: str) -> None:
        with self.lock:
            job = self.get(job_id)
            if job.status in ACTIVE:
                raise ValueError(t("Läuft noch – erst abbrechen"))
            self._cleanup_work(job)
            self.jobs.remove(job)
        if self.engine:
            self.engine.discard(job.id)
        self.save()

    def clear_finished(self) -> None:
        with self.lock:
            self.jobs = [j for j in self.jobs if j.status != "done"]
        self.save()

    # -- internals ---------------------------------------------------------
    def _known(self, path: str) -> bool:
        """Inbox entries that already have a job (cancelled ones stay known until removed)."""
        with self.lock:
            return any(j.source == path or j.source.startswith(path + os.sep) for j in self.jobs
                       if j.status != "done" and j.origin == "inbox")

    def _trim(self) -> None:
        done = [j for j in self.jobs if j.status == "done"]
        for j in done[:-200]:
            self.jobs.remove(j)

    def _requeue_if_waiting(self, job: Job) -> None:
        with self.lock:
            requeue = job.status in ("review", "failed")
            if requeue:
                job.status = "queued"
                job.error = None
                job.stage = "finish" if job.m4b and os.path.exists(job.m4b) else "prepare"
        self.save()
        if requeue:
            self._schedule(job)

    def _schedule(self, job: Job) -> None:
        if self.engine and job.status == "queued":
            self.engine.submit(STAGE_QUEUE[job.stage], job.id, lambda: self.step(job))

    def _job_work(self, job: Job) -> str:
        return os.path.join(self.work_dir, job.id)

    def _cleanup_work(self, job: Job) -> None:
        shutil.rmtree(self._job_work(job), ignore_errors=True)

    def _scanner(self) -> None:
        while True:
            s = self.get_settings()
            interval = max(10, int(s.get("ab_scan_interval") or 60))
            if s.get("ab_watch") and self.inbox:
                try:
                    settle = s.get("ab_settle")
                    self.scan_once(settle=120 if settle in (None, "") else int(settle))
                except Exception as e:  # noqa: BLE001 – keep watching no matter what
                    self.scan_note = t("Fehler beim Durchsuchen: {error}", error=e)
            # wake up when the next waiting download counts as complete, not a whole interval later
            due = [at - _now() for at in list(self.ready_at.values())]
            timeout = min([interval] + [max(1.0, d + 1) for d in due])
            with self.lock:
                if not self._force_scan:
                    self.wake.wait(timeout=timeout)
                self._force_scan = False

    def scan_once(self, settle: int = 120) -> List[Job]:
        """One pass over the inbox; queues every entry that has been quiet for ``settle`` seconds."""
        self.last_scan = _now()
        if not os.path.isdir(self.inbox):
            self.scan_note = t("Eingangsordner nicht gefunden")
            return []
        added = []
        present = set()
        for name in sorted(os.listdir(self.inbox), key=natural_key):
            if not _visible(name):
                continue
            path = os.path.join(self.inbox, name)
            if os.path.islink(path):     # never follow links out of the inbox
                continue
            present.add(path)
            if self._known(path):
                continue
            sig = signature(path)
            if sig is None:                      # still downloading (.part, .!qB …)
                self.seen.pop(path, None)
                continue
            if sig[0] == 0:
                continue
            prev = self.seen.get(path)
            # finished SABnzbd downloads from the search arrive in one move: a short pause is enough
            wait = min(settle, GRAB_SETTLE) if settle and self.hint_provider and self.hint_provider(name) else settle
            if name in self.complete:
                wait = 0
            if not prev and wait:
                applog.info(t("Neu im Eingang – wird verarbeitet, sobald {n} s lang nichts mehr geschrieben wurde",
                              n=wait), name)
            elif not prev:
                applog.info(t("Neu im Eingang – wird jetzt verarbeitet"), name)
            if not prev or prev[0] != sig:
                self.seen[path] = (sig, _now())
                self.ready_at[path] = _now() + wait
                if wait > 0:
                    continue
            quiet_since = max(sig[2], self.seen[path][1])
            self.ready_at[path] = quiet_since + wait
            if wait > 0 and _now() - quiet_since < wait:
                continue
            units = book_units(path)
            if not units:
                continue
            added += self._add_units(path, units)
            self.complete.discard(name)
            self.seen.pop(path, None)
            self.ready_at.pop(path, None)
        for p in list(self.seen):
            if p not in present:
                self.seen.pop(p, None)
                self.ready_at.pop(p, None)
        self.scan_note = t("{n} neue(s) Hörbuch/Hörbücher", n=len(added)) if added else t("Nichts Neues")
        self.notify()
        return added

    def step(self, job: Job) -> None:
        """Run the job's current stage (called by an engine worker)."""
        with self.lock:
            if job.status != "queued" or job not in self.jobs:
                return                       # cancelled or removed while waiting
            job.cancel = False
            stage = job.stage
            job.status = STAGE_STATUS[stage]
        try:
            if stage == "prepare":
                self._prepare(job)
                if job.match is None:
                    self._match(job)
                if not self._skip_owned_volume(job):
                    job.stage, job.status = "convert", "queued"
            elif stage == "convert":
                self._convert(job)
                job.stage = "finish"
                if job.match is None:
                    job.status = "review"
                    job.say(t("Bitte Treffer auswählen") if job.candidates
                            else t("Kein Treffer – bitte suchen oder Angaben eintragen"))
                else:
                    job.status = "queued"
            else:
                with self._file_lock:
                    self._finish(job)
        except encoder.Cancelled:
            job.status = "cancelled"
            job.say(t("Abgebrochen"))
            self._cleanup_work(job)
        except Exception as e:  # noqa: BLE001 – every error ends up in the UI
            if job.cancel:
                job.status = "cancelled"
                job.say(t("Abgebrochen"))
                self._cleanup_work(job)
            else:
                self._fail(job, str(e) or e.__class__.__name__)
        finally:
            job.runner = None
            self.save()
        if self.on_event and job.status in ("done", "review", "failed"):
            try:
                self.on_event(job)
            except Exception as e:  # noqa: BLE001 – a notification must never break the pipeline
                applog.warn(f"Benachrichtigung fehlgeschlagen: {e}", "ntfy")
        self._schedule(job)

    def process(self, job: Job) -> None:
        """Run all stages one after another, without the engine (tests, command line)."""
        with self.lock:
            if job.status in ACTIVE:
                job.status = "queued"
        while job.status == "queued":
            self.step(job)

    # -- steps ---------------------------------------------------------------
    def _prepare(self, job: Job) -> None:
        if not os.path.exists(job.source):
            raise RuntimeError(t("Quelle nicht mehr vorhanden: {path}", path=job.source))
        job.status = "matching"
        job.say(t("Lese Dateien …"), 0.0)
        if os.path.isfile(job.source):
            folder, files = os.path.dirname(job.source), [os.path.basename(job.source)]
        else:
            folder, files = job.source, encoder.collect_audio(job.source)
        if not files:
            raise RuntimeError(t("Keine Audiodateien gefunden"))
        if job.rename_files and job.chapter_names and len(job.chapter_names) == len(files):
            files = rename_tracks(folder, files, job.chapter_names)
            job.rename_files = False          # done – a retry must not rename again
            job.say(t("{n} Tracks umbenannt", n=len(files)))
        job.files = files
        suspicious = suspicious_files(job.source)
        if suspicious:
            job.say(t("⚠ Verdächtige Dateien im Download (werden weder ausgeführt noch übernommen): {files}",
                      files=", ".join(suspicious[:5]) + (" …" if len(suspicious) > 5 else "")))
            job.hints["suspicious"] = suspicious[:20]
        infos = [meta_cache.probe(os.path.join(folder, f)) for f in files]
        job.duration = sum(i.get("duration") or 0 for i in infos)
        tags = infos[0].get("tags", {}) if infos else {}
        name = job.name
        if DISC_RX.fullmatch(name.strip()) or len(re.sub(r"[\W\d_]", "", name)) < 3:
            name = os.path.basename(os.path.dirname(job.source)) or name
        album = (tags.get("album") or "").strip()
        author = (tags.get("album_artist") or tags.get("artist") or "").strip()
        if album and author:
            query = audible.clean_query(f"{album} {author}")
        elif job.origin == "library":
            # Author/Series/NN - Title: the whole path says more than the last folder
            query = audible.clean_query(" ".join(re.sub(r"^\s*\d+(?:[.,]\d+)?\s*[-–.]\s*", "", p)
                                                 for p in job.rel.split(os.sep)))
        else:
            query = audible.clean_query(name)
        job.hints = {
            "query": query,
            "title": audible.clean_query(album) if album else "",
            "author": author if author.lower() not in ("various artists", "various", "unknown", "unbekannt") else "",
            "narrator": (tags.get("composer") or tags.get("narrator") or "").strip(),
            "asin": audible.find_asin(tags.get("asin", ""), tags.get("audible_asin", ""), job.name),
            "duration": job.duration,
            "folder": name,
        }
        if job.pack:
            # One volume of a pack: author and series from the download, the title from this folder
            pack = job.pack
            title = packs.title_of(album or job.name, pack) or packs.title_of(job.name, pack)
            author = pack.get("author") or job.hints["author"]
            job.hints.update(title=title, author=author, series=pack.get("series") or "",
                             series_position=pack.get("volume"),
                             query=audible.clean_query(f"{title} {author}") or job.hints["query"])
            job.say(t("Band {n} aus „{pack}“", n=pack.get("volume") or "?", pack=pack.get("name") or ""))
        grab = self.hint_provider(os.path.basename(job.source)) if self.hint_provider and not job.pack else None
        if grab and (grab.get("asin") or grab.get("title")):
            # Downloaded for a book picked in the shop: its ASIN / title and author are the surest hint
            job.hints["title"] = grab.get("title") or job.hints["title"]
            job.hints["author"] = grab.get("author") or job.hints["author"]
            job.hints["asin"] = grab.get("asin") or job.hints["asin"]
            job.say(t("Download für „{title}“", title=grab.get("title") or grab.get("query") or grab.get("asin")))
        if grab and grab.get("request_id"):
            job.hints["request_id"] = grab["request_id"]          # the request reports back when this is done
        elif grab and grab.get("query"):
            # Only a free search term: the release name is usually more precise – the term is the fallback
            job.hints["alt_query"] = audible.clean_query(grab["query"]) or grab["query"]
            job.say(t("Download aus der Suche „{query}“", query=grab["query"]))
        manual_dir = job.source if os.path.isdir(job.source) else os.path.dirname(job.source)
        manual = tagger.load_manual(manual_dir) if os.path.isdir(job.source) else None
        if manual:
            job.match = manual
            job.say(t("Angaben aus {source}", source=manual["source"]))

    def _match(self, job: Job) -> None:
        s = self.get_settings()
        mode = s.get("ab_match", "auto")
        if mode == "off":
            job.match = {"source": "tags"}
            return
        region = job.region or s.get("ab_region", "de")
        job.say(t("Suche bei Audible ({region}) …", region=region))
        ranked, notes = self.providers.lookup(job.hints, region)
        for note in notes:
            job.say(note)
        threshold = float(s.get("ab_threshold") or 0.8)
        ranked = self._with_requests(job, ranked, region)
        job.candidates = ranked[:12]
        job.region = region
        best = prov.auto_match(ranked, threshold) if mode == "auto" else None
        if mode == "auto" and not best and ranked and ranked[0].get("request_id"):
            # a requested book from another shop (BookBeat): clearly the best, certain enough → take it
            best = audible.best_match(ranked, threshold)
        if best:
            job.match = {k: v for k, v in best.items() if k not in ("match", "request_id", "request_user")}
            job.say(t("Erkannt: {title} – {authors} (Sicherheit {score} %)", title=best["title"],
                      authors=", ".join(best["authors"]), score=round(best["match"]["score"] * 100)))
            self._link_request(job, best)
        elif ranked and ranked[0].get("source") not in prov.AUTO_SOURCES:
            job.say(t("Nur Treffer außerhalb von Audible – bitte prüfen (ohne Laufzeit und Sprecher)"))
        elif ranked:
            job.say(t("{n} mögliche Treffer, keiner eindeutig", n=len(ranked)))
        else:
            job.say(t("Kein Treffer"))
        self.save()

    def _with_requests(self, job: Job, ranked: List[dict], region: str) -> List[dict]:
        """Mark candidates that are an open book request; add requested books the shop search missed."""
        if job.hints.get("request_id"):
            return ranked                      # downloaded for a request already: nothing to guess
        try:
            reqs = self.open_requests() or []
        except Exception:  # noqa: BLE001 – matching works without requests
            return ranked
        if not reqs:
            return ranked
        import bookrequests
        by_key = {r["key"]: r for r in reqs}
        out = []
        for c in ranked:
            r = by_key.pop(bookrequests.book_key(c), None)
            out.append({**c, "request_id": r["id"], "request_user": r["user"]} if r else c)
        for r in by_key.values():
            book = r["book"]
            if book.get("asin"):
                try:                           # the full Audible data (runtime, narrators, chapters)
                    book = {**self.client.product(book["asin"], region), "source": "audible"}
                except Exception:  # noqa: BLE001 – then the request's own data
                    pass
            score = audible.score(book, job.hints)
            if score["score"] >= 0.5:
                out.append({**book, "source": book.get("source") or "audible", "match": score,
                            "request_id": r["id"], "request_user": r["user"]})
        out.sort(key=lambda b: (-b["match"]["score"], b.get("format_type") != "unabridged", not b.get("request_id")))
        return out

    def _link_request(self, job: Job, book: dict) -> None:
        if book.get("request_id") and not job.hints.get("request_id"):
            job.hints["request_id"] = book["request_id"]
            job.say(t("Gehört zur Anfrage von {name}", name=book.get("request_user") or "–"))
            try:
                self.on_request(job)
            except Exception as e:  # noqa: BLE001
                applog.warn(t("Anfragen: {error}", error=e), "Anfragen")

    def _convert(self, job: Job) -> None:
        s = self.get_settings()
        work = self._job_work(job)
        os.makedirs(work, exist_ok=True)
        folder = job.source if os.path.isdir(job.source) else os.path.dirname(job.source)
        out = os.path.join(work, "book.m4b")
        job.status = "converting"
        job.runner = encoder.Runner(lambda: job.cancel)
        chapters_txt = None
        for f in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
            p = os.path.join(folder, f)
            if (f.lower().endswith("chapters.txt") and os.path.isdir(job.source) and encoder.is_regular(p)
                    and os.path.getsize(p) <= encoder.MAX_TEXT_FILE):
                with open(p, encoding="utf-8", errors="replace") as fh:
                    chapters_txt = encoder.parse_chapters_txt(fh.read())
                break
        single = len(job.files) == 1 and job.files[0].lower().endswith((".m4b", ".m4a", ".mp4"))
        src_first = os.path.join(folder, job.files[0])
        if single and meta_cache.probe(src_first).get("audio_codec") == "aac":
            job.say(t("M4B übernehmen (ohne Neukodierung) …"), 0.2)
            encoder.remux(src_first, out, chapters_txt or None, runner=job.runner, nice=self.nice)
            tagger.copy_tags(src_first, out)
            job.converted_info = {"copied": True}
        else:
            def progress(p: float, msg: str) -> None:
                job.progress = p
                job.message = msg
                job.updated = _now()
                self.notify()

            job.say(t("Konvertiere {n} Datei(en) …", n=len(job.files)), 0.0)
            info = encoder.merge_to_m4b(
                folder, job.files, out, bitrate=s.get("bitrate", "auto"),
                mono=bool(s.get("force_mono")), jobs=self.cpu_jobs,
                chapter_mode=s.get("chapter_titles", "auto"), titles=job.chapter_names, progress=progress,
                cancelled=lambda: job.cancel, runner=job.runner, nice=self.nice)
            if chapters_txt:
                encoder.remux(out, out + ".tmp.m4b", chapters_txt, runner=job.runner, nice=self.nice)
                os.replace(out + ".tmp.m4b", out)
            job.converted_info = {"bitrate": info["bitrate"], "chapters": len(info["chapters"]),
                                  "copied": info["copied"]}
            job.say(t("M4B fertig: {n} Kapitel, {bitrate}", n=len(info["chapters"]), bitrate=info["bitrate"]), 1.0)
        job.duration = meta_cache.probe(out).get("duration") or job.duration
        cover = encoder.find_cover(folder) if os.path.isdir(job.source) else None
        if not cover:
            cover = encoder.extract_cover(src_first, work)
        if cover:
            ext = os.path.splitext(cover)[1].lower()
            dst = os.path.join(work, "local-cover" + ext)
            shutil.copyfile(cover, dst)
            job.cover = dst
        job.m4b = out

    def _finish(self, job: Job) -> None:
        s = self.get_settings()
        book = job.match or {}
        if book.get("source") == "tags":
            # No lookup: keep what the files say, fill gaps from folder name
            found = {k: v for k, v in tagger.read_book(job.m4b).items() if v and not k.startswith("_")}
            book = {**book_from_hints(job), **found, "source": "tags"}
        # Already in the library? Ask (or follow the setting) instead of silently adding a "(2)"
        dup_mode = s.get("ab_duplicates", "ask")
        if self.index and dup_mode != "keep" and not job.dup_decision:
            own = os.path.relpath(job.source, self.library) if job.origin == "library" else None
            found = [d for d in self.index.find(book) if d["path"] != own]   # not the book being converted
            if found:
                d = found[0]
                job.duplicate = {k: d.get(k) for k in ("path", "title", "authors", "asin", "format", "why")}
                why = {"asin": t("gleiche ASIN"), "title": t("gleicher Autor und Titel")}.get(d["why"], t("gleiche Serie und Folge"))
                where = f"{d['path']} ({why})"
                if dup_mode == "skip":
                    self._dispose_source(job, s)
                    self._cleanup_work(job)
                    job.m4b = None
                    job.status = "cancelled"
                    job.say(t("Übersprungen – schon in der Bibliothek: {where}", where=where))
                    return
                job.status = "review"
                job.say(t("Schon in der Bibliothek: {where} – bitte entscheiden", where=where))
                return
        parked: List[Optional[Tuple[str, str, str]]] = []     # old library copies, put aside until the new one is in place
        if job.dup_decision == "replace" and job.duplicate:
            parked.append(self._park(job, job.duplicate["path"], "archive"))
        job.status = "tagging"
        job.say(t("Schreibe Tags …"), 1.0)

        # Cover: Audible (full resolution) > image in folder > embedded
        cover: Optional[bytes] = None
        if book.get("cover_url") and s.get("ab_cover_source", "audible") == "audible":
            try:
                cover = audible.fetch_image(book["cover_url"])
                if cover is None:
                    job.say(t("Cover aus dem Shop übersprungen (keine öffentliche Adresse oder kein JPEG/PNG)"))
            except audible.AudibleError as e:
                job.say(t("Cover-Download fehlgeschlagen: {error}", error=e))
        if not cover and job.cover and os.path.exists(job.cover):
            with open(job.cover, "rb") as fh:
                cover = fh.read()
        if not cover:
            cover = tagger.read_cover(job.m4b)

        # Chapters from Audible when they fit the file
        if book.get("asin") and s.get("ab_chapters", "auto") != "files":
            self._audible_chapters(job, book, s.get("ab_chapters", "auto"))

        tagger.write_tags(job.m4b, book, s, cover)
        root = self.library
        if not root:
            raise RuntimeError(t("Kein Bibliotheksordner (LIBRARY_DIR) eingerichtet"))
        folder, filename = library.destination(root, book, s.get("ab_folder_template"),
                                               s.get("ab_file_template"),
                                               title=tagger.clean_title(book, s))
        if job.origin == "library":
            # The new M4B replaces the old files of this very book: they step aside first ...
            parked.append(self._park(job, os.path.relpath(job.source, root), s.get("ab_library_originals", "archive")))
        created = None
        try:
            # Existing author/series folders win, even when spelled a little differently
            folder = library.unique_folder(library.reuse_existing(root, folder))
            if not os.path.exists(folder):
                created = folder
            os.makedirs(folder, exist_ok=True)
            tagger.write_sidecars(folder, book, s, cover)
            dest = library.place(job.m4b, folder, filename)
        except Exception:
            if created and os.path.isdir(created):   # half-made new folder (cover, desc.txt) goes again ...
                shutil.rmtree(created, ignore_errors=True)
            for p in reversed(parked):          # ... and the old copy comes back
                self._unpark(p)
            raise
        job.dest = dest
        for p in parked:                        # only now: delete or archive the old copy
            self._settle(job, p)
        if s.get("write_abs", True):
            absm.write_metadata(os.path.dirname(dest), book, encoder.file_chapters(dest),
                                meta_cache.probe(dest).get("duration") or job.duration)
        if self.index:
            self.index.add(dest, book)
        job.m4b = None
        self._dispose_source(job, s)
        self._cleanup_work(job)
        job.status = "done"
        job.say(t("Abgelegt: {path}", path=os.path.relpath(dest, root)), 1.0)
        if self.on_filed:
            self.on_filed(dest)

    def _park(self, job: Job, rel: str, mode: str) -> Optional[Tuple[str, str, str]]:
        """Put an old book folder aside (hidden, same parent folder – a rename, no copy).

        Deleting or archiving it waits until the new M4B is filed: if that fails, the old
        copy comes back untouched (``_unpark``)."""
        old = os.path.realpath(os.path.join(self.library, rel))
        if os.path.commonpath([old, self.library]) != self.library or old == self.library or not os.path.isdir(old):
            return None
        hidden = _free_path(os.path.join(os.path.dirname(old), f".br-alt-{job.id}-{os.path.basename(old)}"))
        os.rename(old, hidden)
        if self.index:
            self.index.remove(os.path.join(old, "x"))
        return hidden, old, mode

    def _unpark(self, parked: Optional[Tuple[str, str, str]]) -> None:
        if parked and os.path.isdir(parked[0]) and not os.path.exists(parked[1]):
            os.rename(parked[0], parked[1])

    def _settle(self, job: Job, parked: Optional[Tuple[str, str, str]]) -> None:
        """The new copy is filed: the parked old one is deleted or moved to ``_ersetzt``."""
        if not parked:
            return
        hidden, old, mode = parked
        rel = os.path.relpath(old, self.library)
        if mode == "delete":
            shutil.rmtree(hidden)
            job.say(t("Alte Dateien gelöscht: {path}", path=rel))
        else:
            bin_root = os.path.join(self.inbox or os.path.dirname(self.work_dir), "_ersetzt")
            target = _free_path(os.path.join(bin_root, rel))
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.move(hidden, target)
            job.say(t("Alte Fassung verschoben nach {path}", path=os.path.relpath(target, os.path.dirname(bin_root))))
        _remove_empty_parents(os.path.dirname(old), self.library)

    def _audible_chapters(self, job: Job, book: dict, mode: str) -> None:
        data = self.client.chapters(book["asin"], book.get("region") or "de")
        if not data:
            return
        aud = data["runtime_ms"] / 1000 or (data["chapters"][-1]["start"] if data["chapters"] else 0)
        dur = meta_cache.probe(job.m4b).get("duration") or job.duration
        if not aud or not dur:
            return
        diff = abs(dur - aud) / aud
        current = encoder.file_chapters(job.m4b)
        if abs(dur - aud) > max(0.02 * aud, 2.0):
            job.say(t("Audible-Kapitel passen nicht (Laufzeit weicht {diff} % ab) – behalte eigene", diff=f"{diff * 100:.1f}"))
            return
        if mode == "auto" and not encoder.looks_generic(current):
            return
        scale = dur / aud
        chapters = [{"start": c["start"] * scale, "title": c["title"] or t("Kapitel {n}", n=i + 1)}
                    for i, c in enumerate(data["chapters"])]
        tmp = job.m4b + ".chapters.m4b"
        encoder.remux(job.m4b, tmp, chapters, nice=self.nice)
        tagger.copy_tags(job.m4b, tmp)
        os.replace(tmp, job.m4b)
        job.say(t("{n} Kapitel von Audible übernommen", n=len(chapters)))

    def _dispose_source(self, job: Job, s: dict) -> None:
        if job.origin == "inbox":
            self._dispose_path(job.source, s)

    def _dispose_path(self, path: str, s: dict) -> None:
        """An inbox entry is done with: to ``_fertig`` (default) or deleted."""
        if not os.path.exists(path):
            return
        if s.get("ab_originals", "archive") == "delete":
            if os.path.isdir(path):
                shutil.rmtree(path, ignore_errors=True)
            else:
                os.remove(path)
        else:
            target = _free_path(os.path.join(self.inbox, "_fertig", os.path.relpath(path, self.inbox)))
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.move(path, target)
        _remove_empty_parents(os.path.dirname(path), self.inbox)

    def _add_units(self, path: str, units: List[str]) -> List[Job]:
        """Jobs for one inbox entry. A download with several volumes becomes one job per volume –
        only the volumes chosen in the search, and (without a choice) none that are already owned."""
        if units == [path]:
            return [self.add(path, origin="inbox")]
        name = os.path.basename(path)
        info = packs.pack_info(name) or {}
        grab = (self.hint_provider(name) if self.hint_provider else None) or {}
        chosen = grab.get("volumes") if isinstance(grab.get("volumes"), list) else None
        author = info.get("author") or grab.get("author") or ""
        series = info.get("series") or ""
        owned = self.index.series_owned(series, author) if self.index and series else {}
        s = self.get_settings()
        jobs: List[Job] = []
        for u in units:
            vol = packs.volume_of(os.path.basename(u), info)
            if chosen is not None and vol is not None and vol not in chosen:
                applog.info(t("Band {n} nicht ausgewählt – übersprungen", n=vol), name)
                self._dispose_path(u, s)
                continue
            if chosen is None and vol is not None and vol in owned:
                applog.info(t("Band {n} ist schon in der Bibliothek ({where}) – übersprungen", n=vol, where=owned[vol]), name)
                self._dispose_path(u, s)
                continue
            jobs.append(self.add(u, origin="inbox", pack={
                "name": name, "volume": vol, "series": series, "author": author, "chosen": chosen is not None}))
        return jobs

    def _skip_owned_volume(self, job: Job) -> bool:
        """A pack volume Audible recognised as a book we already have: skip it before converting.
        Volumes chosen in the search are processed anyway (the duplicate question comes at the end)."""
        if not (job.pack and not job.pack.get("chosen") and job.match and job.match.get("title") and self.index):
            return False
        found = self.index.find(job.match)
        if not found:
            return False
        self._dispose_source(job, self.get_settings())
        self._cleanup_work(job)
        job.status = "cancelled"
        job.say(t("Übersprungen – schon in der Bibliothek: {where}", where=found[0]["path"]))
        return True

    def _fail(self, job: Job, msg: str) -> None:
        job.status = "failed"
        job.error = msg
        job.say(t("Fehler: {error}", error=msg), level="error")
        self._cleanup_work(job)
        job.m4b = None
        if job.origin == "inbox" and os.path.exists(job.source):
            target = _free_path(os.path.join(self.inbox, "_failed", job.rel))
            try:
                os.makedirs(os.path.dirname(target), exist_ok=True)
                shutil.move(job.source, target)
                job.dest = target
                _remove_empty_parents(os.path.dirname(job.source), self.inbox)
                if os.path.isdir(target):
                    with open(os.path.join(target, "FEHLER.txt"), "w", encoding="utf-8") as fh:
                        fh.write(msg + "\n\n" + "\n".join(job.log) + "\n")
            except OSError as e:
                job.say(t("Konnte nicht nach _failed verschieben: {error}", error=e))


def safe_track_name(name: str) -> str:
    name = re.sub(r'[\\/:*?"<>|\x00-\x1f]', "-", name).strip(" .")
    return re.sub(r"\s+", " ", name)[:150] or "Track"


def rename_tracks(folder: str, files: List[str], names: List[str]) -> List[str]:
    """``03 - Name.mp3`` for every track, in its own subfolder; returns the new relative paths."""
    width = max(2, len(str(len(files))))
    plan = []
    for n, (rel, name) in enumerate(zip(files, names), 1):
        ext = os.path.splitext(rel)[1].lower()
        new = os.path.join(os.path.dirname(rel), f"{n:0{width}d} - {safe_track_name(name)}{ext}")
        plan.append((rel, new))
    # two steps, so swapping names between tracks can't overwrite anything
    temp = []
    for i, (rel, new) in enumerate(plan):
        if rel == new:
            temp.append(rel)
            continue
        tmp = os.path.join(os.path.dirname(rel), f".rn-{i}-{os.getpid()}{os.path.splitext(rel)[1]}")
        os.rename(os.path.join(folder, rel), os.path.join(folder, tmp))
        temp.append(tmp)
    out = []
    for tmp, (rel, new) in zip(temp, plan):
        if tmp != new:
            target = os.path.join(folder, new)
            if os.path.exists(target):
                new = _free_path(target)[len(folder) + 1:]
            os.rename(os.path.join(folder, tmp), os.path.join(folder, new))
        out.append(new)
    return out


def suspicious_files(source: str) -> List[str]:
    """Executables, scripts and links in a download – a sign of a fake or malicious release."""
    if os.path.isfile(source):
        return []
    out = []
    for root, dirs, files in os.walk(source):
        for name in dirs + files:
            p = os.path.join(root, name)
            if os.path.islink(p) or name.lower().endswith(SUSPICIOUS_EXTS):
                out.append(os.path.relpath(p, source) + (" (" + t("Verknüpfung") + ")" if os.path.islink(p) else ""))
        dirs[:] = [d for d in dirs if not os.path.islink(os.path.join(root, d))]
    return sorted(out)


def book_from_hints(job: Job) -> dict:
    h = job.hints
    return {"title": h.get("title") or h.get("folder") or job.name,
            "authors": [h["author"]] if h.get("author") else [],
            "narrators": [h["narrator"]] if h.get("narrator") else []}


def _free_path(path: str) -> str:
    if not os.path.exists(path):
        return path
    base, ext = os.path.splitext(path) if os.path.isfile(path) else (path, "")
    n = 2
    while os.path.exists(f"{base} ({n}){ext}"):
        n += 1
    return f"{base} ({n}){ext}"


def _remove_empty_parents(path: str, stop: str) -> None:
    stop = os.path.realpath(stop)
    path = os.path.realpath(path)
    while path.startswith(stop + os.sep) and path != stop:
        try:
            leftovers = [f for f in os.listdir(path) if not f.startswith(".")]
            if any(encoder.is_audio(f) or os.path.isdir(os.path.join(path, f)) for f in leftovers):
                return
            if leftovers:
                return
            os.rmdir(path)
        except OSError:
            return
        path = os.path.dirname(path)
