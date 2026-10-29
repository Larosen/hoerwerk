"""MP3 -> M4B conversion: one audiobook per folder, one chapter per MP3."""
from __future__ import annotations

import os
import re
import shutil
import tempfile
import threading
import time
import uuid
from collections import deque
from typing import Callable, List, Optional

import db as dbm
import encoder
import tagger
from meta import cache as meta_cache
from rules import natural_key

COVER_NAMES = ("cover", "folder", "front", "albumart")
COVER_EXTS = (".jpg", ".jpeg", ".png")


# --------------------------------------------------------------------------- #
# Helpers
# --------------------------------------------------------------------------- #

def _safe_filename(s: str) -> str:
    s = re.sub(r'[/\\:*?"<>|\x00-\x1f]', "-", s).strip(" .")
    return s or "Hörbuch"


def list_mp3s(folder: str) -> List[str]:
    try:
        names = [n for n in os.listdir(folder)
                 if n.lower().endswith(".mp3") and not n.startswith(".")
                 and os.path.isfile(os.path.join(folder, n))]
    except OSError:
        return []
    return sorted(names, key=natural_key)


def find_cover_file(folder: str) -> Optional[str]:
    try:
        entries = os.listdir(folder)
    except OSError:
        return None
    lower = {e.lower(): e for e in entries}
    for base in COVER_NAMES:
        for ext in COVER_EXTS:
            if base + ext in lower:
                return os.path.join(folder, lower[base + ext])
    return None


# --------------------------------------------------------------------------- #
# Jobs (kind "folder" in the job engine)
# --------------------------------------------------------------------------- #

FINAL = ("done", "error", "cancelled")


class Job:
    def __init__(self, folder: str, rel: str, reason: str, **kw) -> None:
        self.id = kw.get("id") or uuid.uuid4().hex[:10]
        self.folder = folder          # absolute path
        self.rel = rel                # path relative to the data root (for UI)
        self.reason = reason
        self.status = kw.get("status") or "queued"        # queued|running|done|error|cancelled
        self.progress = float(kw.get("progress") or 0.0)
        self.message = kw.get("message") or "Wartet …"
        self.output: Optional[str] = kw.get("output")
        self.created = kw.get("created") or time.time()
        self.finished: Optional[float] = kw.get("finished")
        self.log: deque = deque(kw.get("log") or [], maxlen=40)
        self._runner: Optional[encoder.Runner] = None
        self._cancel = False

    def to_dict(self) -> dict:
        return {
            "id": self.id, "folder": self.rel, "reason": self.reason,
            "status": self.status, "progress": round(self.progress, 4),
            "message": self.message, "output": self.output,
            "created": self.created, "finished": self.finished,
            "log": list(self.log)[-12:],
        }

    def record(self) -> dict:
        return {**self.to_dict(), "path": self.folder, "log": list(self.log)}


class Converter:
    """MP3 folder -> M4B in the same folder. Runs in the engine's ``encode`` queue."""

    def __init__(self, get_settings: Callable[[], dict], db: Optional[dbm.Database] = None, engine=None,
                 jobs: int = 1, nice: str = "10", notify: Optional[Callable[[], None]] = None) -> None:
        self.get_settings = get_settings
        self.notify = notify or (lambda: None)   # live updates for the browser
        self.db = db
        self.engine = engine
        self.cpu_jobs = max(1, jobs)
        self.nice = nice
        self.jobs: List[Job] = []
        self._lock = threading.Lock()
        self._load()

    # -- persistence -------------------------------------------------------
    def _load(self) -> None:
        if not self.db:
            return
        for row in self.db.all("SELECT data FROM jobs WHERE kind='folder' ORDER BY created"):
            d = dbm.loads(row["data"], {})
            if not d.get("path"):
                continue
            job = Job(d["path"], d.get("folder") or "", d.get("reason") or "",
                      **{k: v for k, v in d.items() if k not in ("path", "folder", "reason")})
            if job.status == "running":       # interrupted by a restart
                job.status, job.progress, job.message = "queued", 0.0, "Nach Neustart erneut eingereiht"
            self.jobs.append(job)

    def _save(self, job: Job) -> None:
        if self.db:
            self.db.run("INSERT OR REPLACE INTO jobs(id, kind, status, source, created, updated, data) "
                        "VALUES(?,?,?,?,?,?,?)", (job.id, "folder", job.status, job.folder, job.created,
                                                  time.time(), dbm.dumps(job.record())))
        self.notify()

    def _forget(self, ids: List[str]) -> None:
        if self.db and ids:
            with self.db.tx() as c:
                for i in ids:
                    c.execute("DELETE FROM jobs WHERE id=?", (i,))
        self.notify()

    def start(self) -> None:
        with self._lock:
            waiting = [j for j in self.jobs if j.status == "queued"]
        for j in waiting:
            self._schedule(j)

    def _schedule(self, job: Job) -> None:
        if self.engine:
            self.engine.submit("encode", job.id, lambda: self.run(job))

    # -- public API --------------------------------------------------------
    def enqueue(self, folder: str, rel: str, reason: str) -> Job:
        with self._lock:
            # Don't queue the same folder twice; a queued job picks up the
            # latest file names anyway because it lists the folder when it starts.
            for j in self.jobs:
                if j.folder == folder and j.status == "queued":
                    return j
            job = Job(folder, rel, reason)
            self.jobs.append(job)
            self._trim()
        self._save(job)
        self._schedule(job)
        return job

    def cancel(self, job_id: str) -> bool:
        with self._lock:
            for j in self.jobs:
                if j.id == job_id and j.status in ("queued", "running"):
                    j._cancel = True
                    if j.status == "queued":
                        j.status, j.message = "cancelled", "Abgebrochen"
                        j.finished = time.time()
                        if self.engine:
                            self.engine.discard(j.id)
                    elif j._runner:
                        j._runner.kill()
                    break
            else:
                return False
        self._save(j)
        return True

    def clear_finished(self) -> None:
        with self._lock:
            gone = [j.id for j in self.jobs if j.status in FINAL]
            self.jobs = [j for j in self.jobs if j.status not in FINAL]
        self._forget(gone)

    def list(self) -> List[dict]:
        with self._lock:
            return [j.to_dict() for j in reversed(self.jobs)]

    def busy_folders(self) -> set:
        with self._lock:
            return {j.folder for j in self.jobs if j.status == "running"}

    # -- internals ---------------------------------------------------------
    def _trim(self) -> None:
        done = [j for j in self.jobs if j.status in FINAL]
        gone = []
        while len(done) > 50:
            j = done.pop(0)
            self.jobs.remove(j)
            gone.append(j.id)
        self._forget(gone)

    def run(self, job: Job) -> None:
        """One conversion (an engine step; tests call it directly)."""
        with self._lock:
            if job.status != "queued" or job not in self.jobs:
                return
            job.status, job.message = "running", "Analysiere Dateien …"
        self._save(job)
        try:
            self._run(job)
            if job._cancel:
                job.status, job.message = "cancelled", "Abgebrochen"
            else:
                job.status, job.progress = "done", 1.0
        except Exception as e:  # noqa: BLE001 – report everything to the UI
            if job._cancel:
                job.status, job.message = "cancelled", "Abgebrochen"
            else:
                job.status, job.message = "error", str(e)
        job.finished = time.time()
        job._runner = None
        self._save(job)

    def _run(self, job: Job) -> None:
        s = self.get_settings()
        folder = job.folder
        files = list_mp3s(folder)
        if not files:
            raise RuntimeError("Keine MP3-Dateien im Ordner gefunden")

        first_tags = meta_cache.probe(os.path.join(folder, files[0])).get("tags", {})
        folder_name = os.path.basename(folder.rstrip("/")) or "Hörbuch"
        title = (first_tags.get("album") if s.get("title_source") == "album" else None) or folder_name
        artist = first_tags.get("album_artist") or first_tags.get("artist") or ""
        out_name = _safe_filename(
            s.get("output_name", "{folder}")
            .replace("{folder}", folder_name)
            .replace("{album}", first_tags.get("album") or folder_name)
            .replace("{artist}", artist)
        ) + ".m4b"
        out_path = os.path.join(folder, out_name)
        job.output = out_name

        def progress(p: float, msg: str) -> None:
            job.progress = min(p, 0.999)
            job.message = msg
            self.notify()
            if not job.log or job.log[-1] != msg:
                job.log.append(msg)

        runner = encoder.Runner(lambda: job._cancel)
        job._runner = runner
        tags = {"title": title, "album": title, "artist": artist, "album_artist": artist}
        for key in ("date", "comment", "composer"):
            if first_tags.get(key):
                tags[key] = first_tags[key]
        mode = "tag" if s.get("chapter_titles") == "tag" else "filename"
        info = encoder.merge_to_m4b(
            folder, files, out_path, bitrate=s.get("bitrate", "auto"),
            mono=bool(s.get("force_mono")), jobs=self.cpu_jobs, chapter_mode=mode, tags=tags,
            progress=progress, cancelled=lambda: job._cancel, runner=runner,
            nice=self.nice)

        # Cover + audiobook media type (ffmpeg can't write "stik", mutagen can)
        cover_path = find_cover_file(folder)
        cover = None
        if cover_path:
            with open(cover_path, "rb") as fh:
                cover = fh.read()
        else:
            with tempfile.TemporaryDirectory(prefix="cover-") as tmp:
                emb = encoder.extract_cover(os.path.join(folder, files[0]), tmp)
                if emb:
                    with open(emb, "rb") as fh:
                        cover = fh.read()
        tagger.mark_audiobook(out_path, cover)

        # Originals
        mode = s.get("originals", "keep")
        if mode == "move":
            dest = os.path.join(folder, s.get("originals_folder") or "_mp3")
            os.makedirs(dest, exist_ok=True)
            for f in files:
                shutil.move(os.path.join(folder, f), os.path.join(dest, f))
            job.log.append(f"MP3s verschoben nach {os.path.basename(dest)}/")
        elif mode == "delete":
            for f in files:
                os.remove(os.path.join(folder, f))
            job.log.append("MP3s gelöscht")
        job.message = f"Fertig: {out_name} ({len(info['chapters'])} Kapitel, {info['bitrate']})"
