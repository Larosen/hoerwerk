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


TEXT_EXTS = {"cue", "txt", "yml", "yaml", "json", "nfo"}
IMAGE_EXTS = {"jpg", "jpeg", "png", "webp"}
MAX_TEXT = 1 << 20                     # chapter lists, cue sheets, metadata: small
# Content that never belongs in an audiobook – recognised by its first bytes, whatever the name says
DANGEROUS = (
    (b"MZ", "exe"), (b"\x7fELF", "elf"), (b"%PDF", "pdf"), (b"PK\x03\x04", "zip"), (b"Rar!", "archive"),
    (b"7z\xbc\xaf\x27\x1c", "archive"), (b"\x1f\x8b", "archive"), (b"BZh", "archive"), (b"\xfd7zXZ", "archive"),
    (b"#!", "script"), (b"\xca\xfe\xba\xbe", "mac"), (b"\xcf\xfa\xed\xfe", "mac"), (b"\xfe\xed\xfa", "mac"),
    (b"\xd0\xcf\x11\xe0", "office"), (b"{\\rtf", "office"),
)


def _what(kind: str) -> str:
    return {"exe": t("ein Windows-Programm (.exe/.dll)"), "elf": t("ein Linux-Programm"), "pdf": t("ein PDF-Dokument"),
            "zip": t("ein ZIP-Archiv (auch .docx/.jar/.apk)"), "archive": t("ein Archiv"), "script": t("ein Skript"),
            "mac": t("ein macOS-Programm"), "office": t("ein Office-Dokument")}[kind]


class UploadError(Exception):
    pass


def _is_audio(head: bytes) -> bool:
    if head[:3] == b"ID3" or head[:4] in (b"fLaC", b"OggS") or head[4:8] == b"ftyp":
        return True
    if head[:4] == b"RIFF" and head[8:12] == b"WAVE":
        return True
    if head[:16] == bytes.fromhex("3026b2758e66cf11a6d900aa0062ce6c"):     # WMA (ASF)
        return True
    # MPEG audio / AAC (ADTS) frame sync near the start (some rips begin with a little padding)
    for i in range(min(len(head) - 1, 4096)):
        if head[i] == 0xFF and head[i + 1] & 0xE0 == 0xE0 and head[i + 1] & 0x06 != 0x06 or \
                head[i] == 0xFF and head[i + 1] in (0xF1, 0xF9):
            return True
    return False


def _is_image(head: bytes) -> bool:
    return head[:3] == b"\xff\xd8\xff" or head[:8] == b"\x89PNG\r\n\x1a\n" or (head[:4] == b"RIFF" and head[8:12] == b"WEBP")


def check_content(name: str, head: bytes, size: int) -> None:
    """The file must really be what its name says: audio, a cover image or a small text file.

    A program, document or archive renamed to ``.mp3`` is refused here – by its first bytes."""
    for magic, kind in DANGEROUS:
        if head.startswith(magic):
            raise UploadError(t("{name} ist {what} – nicht erlaubt", name=name, what=_what(kind)))
    ext = os.path.splitext(name)[1][1:].lower()
    if ext in TEXT_EXTS:
        if size > MAX_TEXT or b"\x00" in head:
            raise UploadError(t("{name} ist keine Textdatei", name=name))
        try:
            head.decode("utf-8")
        except UnicodeDecodeError as e:
            if e.start < len(head) - 4:                  # a split character at the very end is fine
                try:
                    head.decode("cp1252")                # old cue sheets
                except UnicodeDecodeError:
                    raise UploadError(t("{name} ist keine Textdatei", name=name))
        if head.lstrip()[:9].lower().startswith((b"<html", b"<script", b"<!doctype")):
            raise UploadError(t("{name} ist keine Textdatei", name=name))
    elif ext in IMAGE_EXTS:
        if not _is_image(head):
            raise UploadError(t("{name} ist kein Bild (JPEG, PNG oder WebP)", name=name))
    elif not _is_audio(head):
        raise UploadError(t("{name} ist keine Audiodatei", name=name))


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
        first = True
        try:
            with open(tmp, "wb") as fh:
                while left > 0:
                    chunk = stream.read(min(CHUNK, left))
                    if not chunk:
                        raise UploadError(t("Upload abgebrochen"))
                    if first:
                        # the start of the file decides – before anything lands on the disk
                        head = chunk
                        while len(head) < min(65536, length) and left - len(head) > 0:
                            more = stream.read(min(65536 - len(head), left - len(head)))
                            if not more:
                                raise UploadError(t("Upload abgebrochen"))
                            head += more
                        check_content(parts[-1], head, length)
                        chunk, first = head, False
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
