"""Upload from the browser into the inbox – e.g. audiobooks ripped from CD.

The browser sends a folder file by file (streamed to disk, never held in memory) into a hidden
staging folder ``<inbox>/.upload-<id>/``. Only when the upload is finished, each top-level folder is
moved into the inbox in one step – so the inbox never sees a half-copied book and starts right away.
From there it is the normal way: tags and folder names → Audible → M4B with chapters → library.

Safety: relative paths only (no ``..``, no absolute paths, no hidden names), only audio files,
covers, cue sheets and the chapter/metadata files Hörwerk understands; size limit per file, free
space checked before each file; stale staging folders are removed after a day.
"""
from __future__ import annotations

import os
import re
import secrets
import shutil
import threading
import time
from typing import Callable, Dict, List, Optional

import applog
from encoder import AUDIO_EXTS
from i18n import t

MAX_FILE = 4 << 30                     # 4 GB per file (one big M4B / FLAC image)
RESERVE = 512 << 20                    # keep this much free on the disk
STALE = 24 * 3600
ALLOWED = set(AUDIO_EXTS) | {"jpg", "jpeg", "png", "webp", "cue", "txt", "yml", "yaml", "json", "nfo"}
TEXT_NAMES = {"chapters.txt", "metadata.yml", "metadata.yaml", "metadata.json", "desc.txt", "reader.txt"}
CHUNK = 1 << 20
_PART = re.compile(r"^[^\x00-\x1f/\\\\:*?\"<>|]{1,200}$")


class UploadError(Exception):
    pass


def clean_path(rel: str) -> List[str]:
    """Path parts of an uploaded file, or UploadError for anything that could escape or hide."""
    parts = [p for p in (rel or "").replace("\\", "/").split("/") if p not in ("", ".")]
    if not parts or len(parts) > 6:
        raise UploadError(t("Ungültiger Dateipfad"))
    for p in parts:
        if p == ".." or p.startswith((".", "_")) or not _PART.match(p) or p != p.strip():
            raise UploadError(t("Ungültiger Dateipfad: {path}", path=rel[:120]))
    name = parts[-1]
    ext = os.path.splitext(name)[1][1:].lower()
    if ext not in ALLOWED or (ext in ("txt", "yml", "yaml", "json") and name.lower() not in TEXT_NAMES):
        raise UploadError(t("Dateityp nicht erlaubt: {name}", name=name[:120]))
    return parts


class Uploads:
    def __init__(self, inbox: str, on_done: Callable[[List[str]], None] = lambda names: None,
                 clock=time.time) -> None:
        self.inbox = inbox
        self.on_done = on_done
        self.clock = clock
        self.lock = threading.Lock()
        self.sessions: Dict[str, dict] = {}

    def _dir(self, uid: str) -> str:
        return os.path.join(self.inbox, f".upload-{uid}")

    def start(self, user: str) -> str:
        if not self.inbox or not os.path.isdir(self.inbox):
            raise UploadError(t("Eingangsordner nicht gefunden"))
        self.cleanup()
        uid = secrets.token_hex(8)
        os.makedirs(self._dir(uid), mode=0o775)
        with self.lock:
            self.sessions[uid] = {"user": user, "started": self.clock(), "files": 0, "bytes": 0}
        return uid

    def _session(self, uid: str) -> dict:
        with self.lock:
            s = self.sessions.get(uid or "")
        if not s or not re.fullmatch(r"[0-9a-f]{16}", uid) or not os.path.isdir(self._dir(uid)):
            raise UploadError(t("Upload nicht gefunden – bitte neu starten"))
        return s

    def put(self, uid: str, rel: str, stream, length: int) -> dict:
        s = self._session(uid)
        parts = clean_path(rel)
        if length <= 0 or length > MAX_FILE:
            raise UploadError(t("Datei zu groß oder leer: {name}", name=parts[-1]))
        if shutil.disk_usage(self.inbox).free < length + RESERVE:
            raise UploadError(t("Zu wenig Platz im Eingang"))
        root = os.path.realpath(self._dir(uid))
        target = os.path.realpath(os.path.join(root, *parts))
        if os.path.commonpath([root, target]) != root:
            raise UploadError(t("Ungültiger Dateipfad"))
        os.makedirs(os.path.dirname(target), exist_ok=True)
        tmp = target + ".part"
        left = length
        try:
            with open(tmp, "wb") as fh:
                while left > 0:
                    chunk = stream.read(min(CHUNK, left))
                    if not chunk:
                        raise UploadError(t("Upload abgebrochen"))
                    fh.write(chunk)
                    left -= len(chunk)
            os.replace(tmp, target)
        except BaseException:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise
        with self.lock:
            s["files"] += 1
            s["bytes"] += length
        return {"ok": True, "files": s["files"], "bytes": s["bytes"]}

    def finish(self, uid: str) -> List[str]:
        """Move every top-level entry of the upload into the inbox (in one step each); returns the names."""
        s = self._session(uid)
        src = self._dir(uid)
        moved = []
        for name in sorted(os.listdir(src)):
            if name.endswith(".part"):
                continue
            dest = os.path.join(self.inbox, name)
            n = 2
            while os.path.exists(dest):                      # never overwrite something in the inbox
                base, ext = os.path.splitext(name) if os.path.isfile(os.path.join(src, name)) else (name, "")
                dest = os.path.join(self.inbox, f"{base} ({n}){ext}")
                n += 1
            os.replace(os.path.join(src, name), dest)
            moved.append(os.path.basename(dest))
        shutil.rmtree(src, ignore_errors=True)
        with self.lock:
            self.sessions.pop(uid, None)
        applog.info(t("Hochgeladen von {user}: {names} ({n} Dateien)", user=s["user"], names=", ".join(moved),
                      n=s["files"]), "Eingang")
        if moved:
            self.on_done(moved)
        return moved

    def cancel(self, uid: str) -> None:
        self._session(uid)
        shutil.rmtree(self._dir(uid), ignore_errors=True)
        with self.lock:
            self.sessions.pop(uid, None)

    def cleanup(self) -> None:
        """Remove staging folders of uploads that never finished (older than a day)."""
        try:
            names = os.listdir(self.inbox)
        except OSError:
            return
        for name in names:
            path = os.path.join(self.inbox, name)
            if name.startswith(".upload-") and os.path.isdir(path) and not os.path.islink(path):
                try:
                    if self.clock() - os.path.getmtime(path) > STALE:
                        shutil.rmtree(path, ignore_errors=True)
                except OSError:
                    pass


def session_id(value: Optional[str]) -> str:
    return value if value and re.fullmatch(r"[0-9a-f]{16}", value) else ""
