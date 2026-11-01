"""Everything that comes from the environment (Docker/Unraid variables), read once at start."""
from __future__ import annotations

import os
from dataclasses import dataclass, field
import ipaddress
from typing import FrozenSet, Mapping, Tuple

import library

VERSION = "7.8.0"


def _bool(env: Mapping[str, str], key: str, default: bool) -> bool:
    return env.get(key, "true" if default else "false").strip().lower() in ("1", "true", "yes", "on")


def parse_networks(text: str) -> Tuple:
    """ "172.18.0.0/16, 10.0.0.5" -> networks; invalid entries are an error at start (better than silently ignored)."""
    out = []
    for part in (p.strip() for p in (text or "").split(",")):
        if part:
            try:
                out.append(ipaddress.ip_network(part, strict=False))
            except ValueError as e:
                raise SystemExit(f"TRUSTED_PROXIES: {part!r} ist keine IP/CIDR ({e})") from e
    return tuple(out)


def cpu_jobs(env: Mapping[str, str] = os.environ) -> int:
    return int(env.get("CPU_CORES") or max(1, (os.cpu_count() or 2) // 2))


@dataclass(frozen=True)
class Config:
    config_dir: str = "/config"
    inbox: str = "/inbox"
    library: str = "/audiobooks"
    work_dir: str = "/inbox/.work"
    port: int = 8080
    auth_user: str = ""
    auth_pass: str = ""
    # Optional: only answer to these Host names (protects against DNS rebinding), e.g. "tower,192.168.1.10"
    allowed_hosts: FrozenSet[str] = field(default_factory=frozenset)
    # Reverse proxies (Pangolin/Newt, Traefik …) whose X-Forwarded-For is believed – IPs or CIDR ranges
    trusted_proxies: Tuple = ()
    cookie_secure: str = "auto"         # auto (when the proxy says https) | true | false
    allow_public: bool = False          # no longer used: accounts protect every access (kept for old templates)
    cpu_jobs: int = 1
    nice: str = "10"
    encode_workers: int = 1     # conversions at the same time (each uses cpu_jobs cores)
    io_workers: int = 2         # lookups, tagging, filing at the same time
    env: Mapping[str, str] = field(default_factory=dict, compare=False, repr=False)

    @classmethod
    def from_env(cls, env: Mapping[str, str] = os.environ) -> "Config":
        inbox = env.get("INBOX_DIR", "/inbox")
        return cls(
            config_dir=env.get("CONFIG_DIR", "/config"),
            inbox=inbox,
            library=env.get("LIBRARY_DIR", "/audiobooks"),
            work_dir=env.get("WORK_DIR") or os.path.join(inbox, ".work"),
            port=int(env.get("PORT", "8080")),
            auth_user=env.get("AUTH_USER", ""),
            auth_pass=env.get("AUTH_PASS", ""),
            allowed_hosts=frozenset(h.strip().lower() for h in env.get("ALLOWED_HOSTS", "").split(",") if h.strip()),
            allow_public=env.get("ALLOW_PUBLIC", "false").lower() == "true",
            trusted_proxies=parse_networks(env.get("TRUSTED_PROXIES", "")),
            cookie_secure=(env.get("COOKIE_SECURE", "auto").strip().lower() or "auto"),
            cpu_jobs=cpu_jobs(env),
            nice=env.get("NICE", "10"),
            encode_workers=max(1, int(env.get("ENCODE_WORKERS", "1"))),
            io_workers=max(1, int(env.get("IO_WORKERS", "2"))),
            env=dict(env),
        )

    @property
    def auth(self) -> bool:
        return bool(self.auth_user and self.auth_pass)


def public_url(text: str) -> str:
    """``https://host[:port]`` (no path, no trailing slash) or "" – anything else is no usable address."""
    from urllib.parse import urlsplit
    text = str(text or "").strip()
    if not text:
        return ""
    try:
        u = urlsplit(text if "://" in text else "https://" + text)
        port = u.port
    except ValueError:
        return ""
    if u.scheme not in ("http", "https") or not u.hostname or u.username or u.password or u.path.strip("/") or u.query:
        return ""
    host = f"[{u.hostname}]" if ":" in u.hostname else u.hostname
    return f"{u.scheme}://{host}" + (f":{port}" if port else "")


def default_settings(env: Mapping[str, str] = os.environ) -> dict:
    """Settings the UI can change; the environment only provides the starting values."""
    b = lambda key, default: _bool(env, key, default)  # noqa: E731
    return {
        "language": env.get("APP_LANGUAGE", "de").lower(),  # de|en
        # the address from the internet (behind Pangolin & co.) – invitation links always point there
        "public_url": "",          # set under Einstellungen → Allgemein, or taken over from the reverse proxy
        # M4B encoding
        "bitrate": env.get("AAC_BITRATE", "auto"),          # auto|64k|96k|…
        "force_mono": b("FORCE_MONO", False),
        "chapter_titles": env.get("CHAPTER_TITLES", "auto"),  # auto|filename|tag
        # Audiobook pipeline
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
        "write_desc": b("WRITE_DESC", False),       # only a fallback for metadata.json
        "write_reader": b("WRITE_READER", False),
        "write_cover": b("WRITE_COVER", True),
        "write_abs": b("WRITE_ABS_METADATA", True),             # metadata.json for Audiobookshelf
        # Entdecken (suggestions)
        "discover_min_rating": float(env.get("DISCOVER_MIN_RATING", "4.3")),
        "discover_min_count": int(env.get("DISCOVER_MIN_COUNT", "50")),
        "discover_unabridged": b("DISCOVER_UNABRIDGED", True),
        "discover_german": b("DISCOVER_GERMAN", True),
        "discover_nonfiction_store": b("DISCOVER_NONFICTION_STORE", False),
        "discover_genre": env.get("DISCOVER_GENRE", ""),
        # Accounts: book requests per user and calendar month (users can have their own number)
        "request_quota": int(env.get("REQUEST_QUOTA", "5")),
        "admin_totp_required": b("ADMIN_TOTP_REQUIRED", False),
        # Gesucht (wanted list)
        "wanted_interval_hours": int(env.get("WANTED_INTERVAL_HOURS", "12")),
    }


# Allowed values of the settings that are a choice
CHOICES = {
    "ab_match": ("auto", "review", "off"),
    "ab_duplicates": ("ask", "skip", "keep"), "ab_library_originals": ("archive", "delete"),
    "ab_chapters": ("auto", "audible", "files"), "ab_originals": ("archive", "delete"),
    "ab_cover_source": ("audible", "local"), "chapter_titles": ("auto", "filename", "tag"),
    "language": ("de", "en"),
}
