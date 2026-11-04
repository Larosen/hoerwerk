"""Which Host names Hörwerk answers under – protection against DNS rebinding.

A DNS-rebinding page arrives in the browser under the attacker's own domain (evil.example resolving to the NAS).
So a request is only answered when its Host is a name the owner uses:

* always: IP addresses and localhost – a rebinding page can never arrive under these;
* ``ALLOWED_HOSTS`` set: exactly those names (``*.example.de`` for all subdomains; ``*`` = any name, the explicit
  way back to the old behaviour);
* ``ALLOWED_HOSTS`` empty (the default): names of the home network – single names like ``tower``, and
  ``.local``, ``.lan``, ``.home.arpa``, ``.internal``, ``.localdomain``, ``fritz.box`` – which nobody can register
  on the internet, plus the host of the public address the admin entered under Einstellungen → Allgemein;
* a request from a reverse proxy listed in ``TRUSTED_PROXIES``: any name – the proxy only forwards the names it is
  configured for (Traefik/Pangolin route by host), and a rebinding page talks to Hörwerk directly, never through it.

Never learned from a request's Host or X-Forwarded-Host header.
"""
from __future__ import annotations

import ipaddress
from typing import FrozenSet

LOOPBACK_NAMES = frozenset({"localhost", "127.0.0.1", "::1"})
HOME_SUFFIXES = (".local", ".lan", ".home.arpa", ".internal", ".localdomain", ".localhost", ".fritz.box", ".home")


def host_name(header: str) -> str:
    """"Tower:8080" -> "tower", "[::1]:8080" -> "::1"."""
    host = (header or "").strip().lower()
    if host.startswith("["):
        return host[1:].split("]", 1)[0]
    return host.rsplit(":", 1)[0] if host.count(":") == 1 else host


def is_ip(name: str) -> bool:
    try:
        ipaddress.ip_address(name.split("%")[0])
        return True
    except ValueError:
        return False


def home_name(name: str) -> bool:
    return bool(name) and ("." not in name.rstrip(".") or name.rstrip(".").endswith(HOME_SUFFIXES))


def allowed(header: str, configured: FrozenSet[str], public_host: str = "", via_proxy: bool = False) -> bool:
    name = host_name(header).rstrip(".")
    if not name:
        return False
    if name in LOOPBACK_NAMES or is_ip(name) or "*" in configured or (via_proxy and not configured):
        return True
    if configured:
        return name in configured or any(p.startswith("*.") and name.endswith(p[1:]) for p in configured)
    return home_name(name) or (bool(public_host) and name == public_host.lower())
