"""Everything that comes from the environment (Docker/Unraid variables), read once at start."""
from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import FrozenSet, Mapping

import library
import planner

VERSION = "4.2.1"


def _bool(env: Mapping[str, str], key: str, default: bool) -> bool:
    return env.get(key, "true" if default else "false").strip().lower() in ("1", "true", "yes", "on")


def cpu_jobs(env: Mapping[str, str] = os.environ) -> int:
    return int(env.get("CPU_CORES") or max(1, (os.cpu_count() or 2) // 2))


@dataclass(frozen=True)
class Config:
    data_root: str = "/data"
    config_dir: str = "/config"
    inbox: str = "/inbox"
    library: str = "/audiobooks"
    work_dir: str = "/inbox/.work"
    port: int = 8080
    auth_user: str = ""
    auth_pass: str = ""
    show_hidden: bool = False
    max_items: int = 20000
    # Optional: only answer to these Host names (protects against DNS rebinding), e.g. "tower,192.168.1.10"
    allowed_hosts: FrozenSet[str] = field(default_factory=frozenset)
    # Without a password the UI only answers clients from private networks (LAN, VPN, Docker)
    allow_public: bool = False
    cpu_jobs: int = 1
    nice: str = "10"
    encode_workers: int = 1     # conversions at the same time (each uses cpu_jobs cores)
    io_workers: int = 2         # lookups, tagging, filing at the same time
    env: Mapping[str, str] = field(default_factory=dict, compare=False, repr=False)

    @classmethod
    def from_env(cls, env: Mapping[str, str] = os.environ) -> "Config":
        inbox = env.get("INBOX_DIR", "/inbox")
        return cls(
            data_root=os.path.realpath(env.get("DATA_DIR", "/data")),
            config_dir=env.get("CONFIG_DIR", "/config"),
            inbox=inbox,
            library=env.get("LIBRARY_DIR", "/audiobooks"),
            work_dir=env.get("WORK_DIR") or os.path.join(inbox, ".work"),
            port=int(env.get("PORT", "8080")),
            auth_user=env.get("AUTH_USER", ""),
            auth_pass=env.get("AUTH_PASS", ""),
            show_hidden=env.get("SHOW_HIDDEN", "false").lower() == "true",
            max_items=int(env.get("MAX_ITEMS", "20000")),
            allowed_hosts=frozenset(h.strip().lower() for h in env.get("ALLOWED_HOSTS", "").split(",") if h.strip()),
            allow_public=env.get("ALLOW_PUBLIC", "false").lower() == "true",
            cpu_jobs=cpu_jobs(env),
            nice=env.get("NICE", "10"),
            encode_workers=max(1, int(env.get("ENCODE_WORKERS", "1"))),
            io_workers=max(1, int(env.get("IO_WORKERS", "2"))),
            env=dict(env),
        )

    @property
    def auth(self) -> bool:
        return bool(self.auth_user and self.auth_pass)


def default_settings(env: Mapping[str, str] = os.environ) -> dict:
    """Settings the UI can change; the environment only provides the starting values."""
    b = lambda key, default: _bool(env, key, default)  # noqa: E731
    return {
        "language": env.get("APP_LANGUAGE", "de").lower(),  # de|en
        "auto_convert": b("AUTO_CONVERT", True),
        "bitrate": env.get("AAC_BITRATE", "auto"),          # auto|64k|96k|…
        "force_mono": b("FORCE_MONO", False),
        "originals": env.get("ORIGINALS", "keep"),          # keep|move|delete
        "originals_folder": env.get("ORIGINALS_FOLDER", "_mp3"),
        "chapter_titles": env.get("CHAPTER_TITLES", "auto"),  # auto|filename|tag
        "output_name": env.get("OUTPUT_NAME", "{folder}"),
        "title_source": env.get("TITLE_SOURCE", "folder"),  # folder|album
        # Rename defaults (the UI can override them per run)
        "conflict": env.get("CONFLICT", "error"),            # error|number
        "sidecars": env.get("SIDECARS", "true").lower() == "true",
        "sidecar_exts": env.get("SIDECAR_EXTS", planner.DEFAULT_SIDECARS),
        "allow_subdirs": b("ALLOW_SUBDIRS", False),
        # Audiobook pipeline (auto-m4b + beets-audible)
        "ab_watch": b("AUDIOBOOK_WATCH", True),
        "ab_region": env.get("AUDIBLE_REGION", "de"),
        "ab_match": env.get("MATCH_MODE", "auto"),            # auto|review|off
        "ab_threshold": float(env.get("MATCH_THRESHOLD", "0.8")),
        "ab_folder_template": env.get("FOLDER_TEMPLATE", library.DEFAULT_FOLDER),
        "ab_file_template": env.get("FILE_TEMPLATE", library.DEFAULT_FILE),
        "ab_chapters": env.get("CHAPTERS", "auto"),           # auto|audible|files
        "ab_cover_source": env.get("COVER_SOURCE", "audible"),  # audible|local
        "ab_originals": env.get("INBOX_ORIGINALS", "archive"),  # archive|delete
        "ab_scan_interval": int(env.get("SCAN_INTERVAL", "60")),
        "ab_fallback": b("METADATA_FALLBACK", True),          # Audnexus / Apple Books / Open Library
        "ab_duplicates": env.get("DUPLICATES", "ask"),       # ask|skip|keep
        "ab_library_originals": env.get("LIBRARY_ORIGINALS", "archive"),  # archive|delete
        "library_rescan_hours": int(env.get("LIBRARY_RESCAN_HOURS", "6")),
        "ab_settle": int(env.get("SETTLE_SECONDS", "120")),
        "narrator_in_artist": b("NARRATOR_IN_ARTIST", False),
        "keep_series_in_title": b("KEEP_SERIES_IN_TITLE", True),
        "keep_series_in_subtitle": b("KEEP_SERIES_IN_SUBTITLE", True),
        "write_desc": b("WRITE_DESC", True),
        "write_reader": b("WRITE_READER", True),
        "write_cover": b("WRITE_COVER", True),
        "write_abs": b("WRITE_ABS_METADATA", True),             # metadata.json for Audiobookshelf
    }


# Allowed values of the settings that are a choice
CHOICES = {
    "originals": ("keep", "move", "delete"), "ab_match": ("auto", "review", "off"),
    "ab_duplicates": ("ask", "skip", "keep"), "ab_library_originals": ("archive", "delete"),
    "ab_chapters": ("auto", "audible", "files"), "ab_originals": ("archive", "delete"),
    "ab_cover_source": ("audible", "local"), "chapter_titles": ("auto", "filename", "tag"),
    "language": ("de", "en"),
}
RENAME_OPTION_KEYS = ("conflict", "sidecars", "sidecar_exts", "allow_subdirs")
