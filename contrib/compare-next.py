#!/usr/bin/env python3
"""Vergleicht zwei Hörwerk-Instanzen über die API: die laufende (Python) und hoerwerk:next (Rust) auf einer Kopie
von /config – siehe docs/VERGLEICH.md. Liest nur, ändert nichts.

    python3 compare-next.py http://tower:8080 http://tower:8081
    python3 compare-next.py http://tower:8080 http://tower:8081 --user admin --zeigen 20

Fragt nach dem Passwort (und dem Code der Authenticator-App, falls eingeschaltet). Beide Instanzen haben dieselben
Konten, weil die zweite eine Kopie der Datenbank nutzt. Ergebnis: je Bereich „gleich“ oder die abweichenden Stellen.
Exit-Code 0 = alles gleich, 1 = Unterschiede, 2 = Anmeldung oder Verbindung gescheitert.
"""
from __future__ import annotations

import argparse
import getpass
import json
import sys
import urllib.error
import urllib.request
from typing import Any, List, Optional, Tuple

OWN = {"X-Requested-With": "Hoerwerk", "Content-Type": "application/json"}
# (Bereich, Adresse, Schlüssel, die sich von selbst unterscheiden – Zeitpunkte, Sitzungen, Laufzeit)
AREAS = [
    ("Einstellungen", "/api/settings", set()),
    ("Konten", "/api/users", {"last_login", "last_seen", "sessions"}),
    ("Eigenes Konto", "/api/auth/me", {"session", "sessions", "last_seen"}),
    ("Einladungen", "/api/invites", set()),
    ("Bibliothek (Zahlen)", "/api/library", {"scanned", "last_scan", "scanning", "duration"}),
    ("Bibliothek (Bücher)", "/api/library/books?limit=100000", set()),
    ("Noch nicht umgewandelt", "/api/library/unconverted", set()),
    ("Anfragen", "/api/requests", set()),
    ("Anfragen (Schlüssel)", "/api/requests/keys", set()),
    ("Statistik", "/api/stats", {"since"}),
    ("Quellen (Indexer, SABnzbd, Audiobookshelf, Plex)", "/api/sources", set()),
    ("Benachrichtigungen", "/api/notify", {"sent", "last_error"}),
    ("Hörverlauf-Verknüpfungen", "/api/listening/accounts", set()),
    ("Entdecken: Genres", "/api/discover/genres", set()),
]


class Instance:
    def __init__(self, base: str) -> None:
        self.base = base.rstrip("/")
        self.cookie = ""

    def call(self, method: str, path: str, body: Optional[dict] = None) -> Tuple[int, Any]:
        data = json.dumps(body).encode() if body is not None else None
        headers = dict(OWN) if method != "GET" else {}
        if self.cookie:
            headers["Cookie"] = self.cookie
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=120) as r:  # noqa: S310 – the admin's own servers
                cookie = r.headers.get("Set-Cookie")
                if cookie:
                    self.cookie = cookie.split(";")[0]
                raw = r.read()
                return r.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                return e.code, json.loads(raw)
            except ValueError:
                return e.code, raw.decode("utf-8", "replace")[:200]

    def login(self, user: str, password: str) -> None:
        status, res = self.call("POST", "/api/auth/login", {"name": user, "password": password})
        if status != 200:
            raise SystemExit(f"{self.base}: Anmeldung gescheitert ({status}): {res}")
        if isinstance(res, dict) and res.get("totp"):
            code = input(f"{self.base}: Code aus der Authenticator-App: ").strip()
            status, res = self.call("POST", "/api/auth/login/totp", {"ticket": res.get("ticket"), "code": code})
            if status != 200:
                raise SystemExit(f"{self.base}: Code nicht angenommen ({status}): {res}")


def differences(a: Any, b: Any, ignore: set, path: str = "") -> List[Tuple[str, Any, Any]]:
    """Where two JSON answers differ: [(Stelle, links, rechts)]."""
    if isinstance(a, dict) and isinstance(b, dict):
        out: List[Tuple[str, Any, Any]] = []
        for k in sorted(set(a) | set(b)):
            if k in ignore:
                continue
            if k not in a or k not in b:
                out.append((f"{path}.{k}", a.get(k, "– fehlt –"), b.get(k, "– fehlt –")))
            else:
                out += differences(a[k], b[k], ignore, f"{path}.{k}")
        return out
    if isinstance(a, list) and isinstance(b, list):
        out = []
        if len(a) != len(b):
            out.append((f"{path} (Anzahl)", len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            out += differences(x, y, ignore, f"{path}[{i}]")
        return out
    if isinstance(a, float) or isinstance(b, float):
        try:
            if abs(float(a) - float(b)) < 1e-6:
                return []
        except (TypeError, ValueError):
            pass
    return [] if a == b else [(path or "(Antwort)", a, b)]


def short(v: Any) -> str:
    text = json.dumps(v, ensure_ascii=False) if not isinstance(v, str) else repr(v)
    return text if len(text) <= 90 else text[:87] + "…"


def main() -> int:
    ap = argparse.ArgumentParser(description="Vergleicht Hörwerk (Python) mit hoerwerk:next (Rust) über die API.")
    ap.add_argument("alt", help="Adresse der laufenden Instanz, z. B. http://tower:8080")
    ap.add_argument("neu", help="Adresse von hoerwerk:next, z. B. http://tower:8081")
    ap.add_argument("--user", default="admin", help="Admin-Konto (in beiden gleich), Standard: admin")
    ap.add_argument("--zeigen", type=int, default=10, help="höchstens so viele Unterschiede je Bereich zeigen")
    args = ap.parse_args()
    old, new = Instance(args.alt), Instance(args.neu)
    password = getpass.getpass(f"Passwort für {args.user}: ")
    try:
        for inst in (old, new):
            inst.login(args.user, password)
    except OSError as e:
        print(f"Keine Verbindung: {e}")
        return 2
    versions = [inst.call("GET", "/api/health")[1] for inst in (old, new)]
    print(f"Alt: {args.alt}  Version {(versions[0] or {}).get('version', '?')}")
    print(f"Neu: {args.neu}  Version {(versions[1] or {}).get('version', '?')}\n")
    differ = 0
    for name, path, ignore in AREAS:
        (sa, a), (sb, b) = old.call("GET", path), new.call("GET", path)
        if sa != sb:
            print(f"✗ {name}: Antwort {sa} gegenüber {sb}")
            differ += 1
            continue
        diffs = differences(a, b, ignore)
        if not diffs:
            print(f"✓ {name}: gleich")
            continue
        differ += 1
        print(f"✗ {name}: {len(diffs)} abweichende Stelle{'n' if len(diffs) != 1 else ''}")
        for where, x, y in diffs[:args.zeigen]:
            print(f"    {where}\n      alt: {short(x)}\n      neu: {short(y)}")
        if len(diffs) > args.zeigen:
            print(f"    … und {len(diffs) - args.zeigen} weitere (--zeigen {len(diffs)} zeigt alle)")
    print("\nAlles gleich." if not differ else f"\n{differ} Bereich{'e' if differ != 1 else ''} mit Unterschieden.")
    print("Hinweis: Was seit dem Kopieren von /config passiert ist (neue Anfragen, gefilterte Bücher), "
          "unterscheidet sich natürlich.")
    return 0 if not differ else 1


if __name__ == "__main__":
    sys.exit(main())
