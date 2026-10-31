"""Accounts, passwords, login sessions and the brake against password guessing.

* Passwords: scrypt (stdlib), random 16-byte salt per user, stored as ``scrypt$N$r$p$salt$hash``.
  Unknown user names are checked against a dummy hash, so the answer time doesn't tell which names exist.
* Sessions: 256-bit random token in an HttpOnly cookie; the database only keeps its SHA-256.
  "Stay signed in": 30 days, extended while used; otherwise 12 hours.
* Brake: per client IP at most ``IP_FAILS`` failed logins in ``WINDOW`` seconds, then blocked for ``WINDOW``.
  Per account only a growing delay – a stranger can't lock a family member out.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import secrets
import threading
import time
from collections import defaultdict, deque
from typing import Deque, Dict, List, Optional

import db as dbm
from i18n import t

ROLES = ("admin", "user")
PERMS = ("see_all_requests", "auto_approve", "interactive_search")
MIN_PASSWORD = 10
MAX_PASSWORD = 256
NAME_MAX = 40
SESSION_SHORT = 12 * 3600
SESSION_LONG = 30 * 24 * 3600
TOUCH_EVERY = 300                # write last_seen at most every 5 minutes
IP_FAILS = 5
WINDOW = 15 * 60
SCRYPT = {"n": 2 ** 15, "r": 8, "p": 1}
_MAXMEM = 64 * 1024 * 1024


class AuthError(Exception):
    """Login refused; ``status`` 401 (wrong data) or 429 (too many attempts)."""

    def __init__(self, message: str, status: int = 401, reason: str = "") -> None:
        super().__init__(message)
        self.status = status
        self.reason = reason


def hash_password(password: str) -> str:
    salt = secrets.token_bytes(16)
    digest = hashlib.scrypt(password.encode(), salt=salt, maxmem=_MAXMEM, dklen=32, **SCRYPT)
    b64 = lambda b: base64.b64encode(b).decode()  # noqa: E731
    return f"scrypt${SCRYPT['n']}${SCRYPT['r']}${SCRYPT['p']}${b64(salt)}${b64(digest)}"


def verify_password(password: str, stored: str) -> bool:
    try:
        kind, n, r, p, salt, digest = stored.split("$")
        if kind != "scrypt":
            return False
        want = base64.b64decode(digest)
        got = hashlib.scrypt(password.encode(), salt=base64.b64decode(salt), n=int(n), r=int(r), p=int(p),
                             maxmem=_MAXMEM, dklen=len(want))
    except (ValueError, TypeError):
        return False
    return hmac.compare_digest(got, want)


_DUMMY = hash_password(secrets.token_urlsafe(16))


def check_password_rules(password: str, name: str = "") -> None:
    if not isinstance(password, str) or len(password) < MIN_PASSWORD:
        raise ValueError(t("Das Passwort braucht mindestens {n} Zeichen", n=MIN_PASSWORD))
    if len(password) > MAX_PASSWORD:
        raise ValueError(t("Das Passwort ist zu lang"))
    if name and password.strip().lower() == name.strip().lower():
        raise ValueError(t("Das Passwort darf nicht der Name sein"))


def check_name(name: str) -> str:
    name = (name or "").strip()
    if not name or len(name) > NAME_MAX or any(ord(c) < 32 for c in name):
        raise ValueError(t("Ungültiger Name"))
    return name


def _token_hash(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()


class Accounts:
    def __init__(self, db: dbm.Database, clock=time.time) -> None:
        self.db = db
        self.clock = clock
        self.lock = threading.Lock()
        self.ip_fails: Dict[str, Deque[float]] = defaultdict(deque)
        self.user_fails: Dict[str, int] = defaultdict(int)

    # -- users -------------------------------------------------------------
    @staticmethod
    def public(row) -> Optional[dict]:
        if row is None:
            return None
        perms = dbm.loads(row["perms"], {}) or {}
        return {"id": row["id"], "name": row["name"], "role": row["role"],
                "perms": {p: bool(perms.get(p)) for p in PERMS}, "quota": row["quota"],
                "disabled": bool(row["disabled"]), "created": row["created"], "last_login": row["last_login"]}

    def count(self) -> int:
        return self.db.one("SELECT COUNT(*) AS n FROM users")["n"]

    def get(self, uid: int) -> Optional[dict]:
        return self.public(self.db.one("SELECT * FROM users WHERE id=?", (uid,)))

    def by_name(self, name: str) -> Optional[dict]:
        return self.public(self.db.one("SELECT * FROM users WHERE name=?", (name.strip(),)))

    def list(self) -> List[dict]:
        return [self.public(r) for r in self.db.all("SELECT * FROM users ORDER BY name COLLATE NOCASE")]

    def create(self, name: str, password: str, role: str = "user", perms: Optional[dict] = None,
               quota: Optional[int] = None) -> dict:
        name = check_name(name)
        check_password_rules(password, name)
        if role not in ROLES:
            raise ValueError(t("Unbekannte Rolle"))
        if self.by_name(name):
            raise ValueError(t("Den Namen gibt es schon"))
        now = self.clock()
        clean = {p: bool((perms or {}).get(p)) for p in PERMS}
        with self.db.tx() as c:
            cur = c.execute("INSERT INTO users(name, pw, role, perms, quota, created, pw_changed) VALUES(?,?,?,?,?,?,?)",
                            (name, hash_password(password), role, dbm.dumps(clean), _quota(quota), now, now))
        return self.get(cur.lastrowid)

    def update(self, uid: int, changes: dict) -> dict:
        user = self.get(uid)
        if not user:
            raise KeyError(uid)
        sets, args = [], []
        if "name" in changes:
            name = check_name(changes["name"])
            other = self.by_name(name)
            if other and other["id"] != uid:
                raise ValueError(t("Den Namen gibt es schon"))
            sets.append("name=?"); args.append(name)  # noqa: E702
        if "role" in changes:
            if changes["role"] not in ROLES:
                raise ValueError(t("Unbekannte Rolle"))
            if user["role"] == "admin" and changes["role"] != "admin" and self._admins(exclude=uid) == 0:
                raise ValueError(t("Der letzte Admin kann nicht herabgestuft werden"))
            sets.append("role=?"); args.append(changes["role"])  # noqa: E702
        if "perms" in changes and isinstance(changes["perms"], dict):
            merged = {**user["perms"], **{p: bool(v) for p, v in changes["perms"].items() if p in PERMS}}
            sets.append("perms=?"); args.append(dbm.dumps(merged))  # noqa: E702
        if "quota" in changes:
            sets.append("quota=?"); args.append(_quota(changes["quota"]))  # noqa: E702
        if "disabled" in changes:
            off = bool(changes["disabled"])
            if off and user["role"] == "admin" and self._admins(exclude=uid) == 0:
                raise ValueError(t("Der letzte Admin kann nicht gesperrt werden"))
            sets.append("disabled=?"); args.append(int(off))  # noqa: E702
        if sets:
            with self.db.tx() as c:
                c.execute(f"UPDATE users SET {', '.join(sets)} WHERE id=?", (*args, uid))
                if changes.get("disabled"):
                    c.execute("DELETE FROM sessions WHERE user_id=?", (uid,))
        return self.get(uid)

    def set_password(self, uid: int, password: str) -> None:
        user = self.get(uid)
        if not user:
            raise KeyError(uid)
        check_password_rules(password, user["name"])
        with self.db.tx() as c:
            c.execute("UPDATE users SET pw=?, pw_changed=? WHERE id=?", (hash_password(password), self.clock(), uid))
            c.execute("DELETE FROM sessions WHERE user_id=?", (uid,))     # everywhere signed out

    def delete(self, uid: int) -> None:
        user = self.get(uid)
        if not user:
            raise KeyError(uid)
        if user["role"] == "admin" and self._admins(exclude=uid) == 0:
            raise ValueError(t("Der letzte Admin kann nicht gelöscht werden"))
        self.db.run("DELETE FROM users WHERE id=?", (uid,))

    def _admins(self, exclude: int = 0) -> int:
        return self.db.one("SELECT COUNT(*) AS n FROM users WHERE role='admin' AND disabled=0 AND id<>?",
                           (exclude,))["n"]

    # -- login -------------------------------------------------------------
    def _blocked(self, ip: str) -> bool:
        now = self.clock()
        with self.lock:
            q = self.ip_fails[ip]
            while q and q[0] < now - WINDOW:
                q.popleft()
            return len(q) >= IP_FAILS

    def _fail(self, ip: str, name: str) -> None:
        with self.lock:
            self.ip_fails[ip].append(self.clock())
            self.user_fails[name.lower()] += 1
            if len(self.ip_fails) > 10000:          # memory bound against address floods
                self.ip_fails.clear()

    def delay_for(self, name: str) -> float:
        """Growing pause after failed logins of one account (0, 0, 0, 1, 2, 4 … max 8 s)."""
        n = self.user_fails.get((name or "").lower(), 0)
        return 0.0 if n < 3 else float(min(8, 2 ** (n - 3)))

    def authenticate(self, name: str, password: str, ip: str) -> dict:
        """The user for name + password, or AuthError (reason for the security log in ``reason``)."""
        if self._blocked(ip):
            raise AuthError(t("Zu viele Fehlversuche – bitte in 15 Minuten erneut versuchen"), 429, "ip_blocked")
        name = (name or "").strip()[:NAME_MAX]
        row = self.db.one("SELECT * FROM users WHERE name=?", (name,)) if name else None
        ok = verify_password(password or "", row["pw"] if row else _DUMMY)
        if not row or not ok:
            self._fail(ip, name)
            raise AuthError(t("Name oder Passwort falsch"), 401, "unknown_user" if not row else "bad_password")
        if row["disabled"]:
            self._fail(ip, name)
            raise AuthError(t("Name oder Passwort falsch"), 401, "disabled")
        with self.lock:
            self.user_fails.pop(name.lower(), None)
        self.db.run("UPDATE users SET last_login=? WHERE id=?", (self.clock(), row["id"]))
        return self.public(row)

    # -- sessions ----------------------------------------------------------
    def start_session(self, uid: int, persistent: bool, ip: str = "", agent: str = "") -> str:
        token = secrets.token_urlsafe(32)
        now = self.clock()
        life = SESSION_LONG if persistent else SESSION_SHORT
        with self.db.tx() as c:
            c.execute("DELETE FROM sessions WHERE expires < ?", (now,))
            c.execute("INSERT INTO sessions(token_hash, user_id, created, last_seen, expires, persistent, ip, agent)"
                      " VALUES(?,?,?,?,?,?,?,?)", (_token_hash(token), uid, now, now, now + life, int(persistent),
                                                   ip[:64], agent[:200]))
        return token

    def session_user(self, token: str, ip: str = "") -> Optional[dict]:
        """The signed-in user for a cookie token (None if unknown, expired or the user is locked)."""
        if not token or len(token) > 100:
            return None
        th = _token_hash(token)
        row = self.db.one("SELECT s.*, u.disabled FROM sessions s JOIN users u ON u.id = s.user_id "
                          "WHERE s.token_hash=?", (th,))
        now = self.clock()
        if not row or row["expires"] < now or row["disabled"]:
            return None
        if now - row["last_seen"] > TOUCH_EVERY:
            life = SESSION_LONG if row["persistent"] else SESSION_SHORT
            expires = max(row["expires"], now + life) if row["persistent"] else row["expires"]
            self.db.run("UPDATE sessions SET last_seen=?, expires=?, ip=? WHERE token_hash=?",
                        (now, expires, ip[:64] or row["ip"], th))
        user = self.get(row["user_id"])
        if user:
            user["session"] = th[:16]
        return user

    def end_session(self, token: str) -> None:
        self.db.run("DELETE FROM sessions WHERE token_hash=?", (_token_hash(token or ""),))

    def sessions(self, uid: int) -> List[dict]:
        rows = self.db.all("SELECT * FROM sessions WHERE user_id=? AND expires>=? ORDER BY last_seen DESC",
                           (uid, self.clock()))
        return [{"id": r["token_hash"][:16], "created": r["created"], "last_seen": r["last_seen"],
                 "expires": r["expires"], "ip": r["ip"], "agent": r["agent"]} for r in rows]

    def end_sessions(self, uid: int, keep: str = "") -> int:
        """Sign out everywhere (but keep the session ``keep``, a 16-char id prefix)."""
        with self.db.tx() as c:
            return c.execute("DELETE FROM sessions WHERE user_id=? AND substr(token_hash, 1, 16)<>?",
                             (uid, keep)).rowcount

    def end_session_id(self, uid: int, sid: str) -> bool:
        return self.db.run("DELETE FROM sessions WHERE user_id=? AND substr(token_hash, 1, 16)=?", (uid, sid)) > 0


def _quota(value) -> Optional[int]:
    if value in (None, ""):
        return None
    n = int(value)
    if not 0 <= n <= 1000:
        raise ValueError(t("Ungültiges Kontingent"))
    return n
