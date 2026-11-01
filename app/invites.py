"""Invitations by link (like Wizarr): an admin creates a link, whoever opens it picks their own name and
password and gets a family account.

* The link carries a random token (192 bit). It expires after a number of days (or never), can be used a set
  number of times and can be revoked at any time.
* Accepting is one transaction: the account is created and the use counted together – two people with the
  same single-use link cannot both get in.
* Wrong or used-up tokens count as failed sign-ins (IP brake, ``auth.log`` → CrowdSec), so links can't be guessed.
"""
from __future__ import annotations

import secrets
import time
from typing import Callable, List, Optional

import db as dbm
from accounts import PERMS
from i18n import t

MAX_USES = 100
MAX_DAYS = 365
NOTE_MAX = 60
TOKEN_CHARS = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_")


def valid_token_format(token: str) -> bool:
    return 20 <= len(token or "") <= 64 and set(token) <= TOKEN_CHARS


class InviteError(ValueError):
    """The link does not (or no longer) work – the same answer for every reason, so nothing leaks."""


class Invites:
    def __init__(self, db: dbm.Database, accounts, clock: Callable[[], float] = time.time) -> None:
        self.db = db
        self.accounts = accounts
        self.clock = clock

    # -- admin ---------------------------------------------------------------
    def create(self, by: str, note: str = "", days: Optional[int] = 7, uses: int = 1,
               quota: Optional[int] = None, perms: Optional[dict] = None) -> dict:
        note = " ".join(str(note or "").split())[:NOTE_MAX]
        try:
            days = int(days or 0)                   # 0 = never expires
            uses = int(uses if uses not in (None, "") else 1)
        except (TypeError, ValueError):
            raise ValueError(t("Ungültige Daten"))
        if not 0 <= days <= MAX_DAYS or not 1 <= uses <= MAX_USES:
            raise ValueError(t("Ungültige Daten"))
        if quota not in (None, ""):
            quota = int(quota)
            if not 0 <= quota <= 1000:
                raise ValueError(t("Ungültige Daten"))
        else:
            quota = None
        # the rights the new account starts with (never admin – only the known family rights)
        perms = {p: bool((perms or {}).get(p)) for p in PERMS} if isinstance(perms, dict) or perms is None else None
        if perms is None:
            raise ValueError(t("Ungültige Daten"))
        now = self.clock()
        token = secrets.token_urlsafe(24)
        with self.db.tx() as c:
            cur = c.execute("INSERT INTO invites(token, note, created, created_by, expires, max_uses, data) "
                            "VALUES(?,?,?,?,?,?,?)",
                            (token, note, now, by, now + days * 86400 if days else None, uses,
                             dbm.dumps({"quota": quota, "perms": perms, "used_by": []})))
        return self.get(cur.lastrowid)

    def _public(self, row) -> dict:
        data = dbm.loads(row["data"], {}) or {}
        now = self.clock()
        status = ("revoked" if row["revoked"] else "used" if row["uses"] >= row["max_uses"]
                  else "expired" if row["expires"] and row["expires"] < now else "open")
        return {"id": row["id"], "token": row["token"], "note": row["note"], "created": row["created"],
                "created_by": row["created_by"], "expires": row["expires"], "max_uses": row["max_uses"],
                "uses": row["uses"], "quota": data.get("quota"), "perms": data.get("perms") or {},
                "used_by": data.get("used_by", []), "status": status}

    def get(self, iid: int) -> Optional[dict]:
        row = self.db.one("SELECT * FROM invites WHERE id=?", (iid,))
        return self._public(row) if row else None

    def list(self) -> List[dict]:
        return [self._public(r) for r in self.db.all("SELECT * FROM invites ORDER BY id DESC LIMIT 200")]

    def revoke(self, iid: int) -> dict:
        if not self.db.run("UPDATE invites SET revoked=1 WHERE id=?", (iid,)):
            raise KeyError(iid)
        return self.get(iid)

    def delete(self, iid: int) -> None:
        if not self.db.run("DELETE FROM invites WHERE id=?", (iid,)):
            raise KeyError(iid)

    # -- the invited person ----------------------------------------------------
    def check(self, token: str) -> dict:
        """The open invitation for this link (only what the page may show), else InviteError."""
        if not valid_token_format(token):
            raise InviteError(t("Diese Einladung gilt nicht (mehr). Bitte frag nach einem neuen Link."))
        row = self.db.one("SELECT * FROM invites WHERE token=?", (token,))
        if not row or self._public(row)["status"] != "open":
            raise InviteError(t("Diese Einladung gilt nicht (mehr). Bitte frag nach einem neuen Link."))
        return self._public(row)

    def accept(self, token: str, name: str, password: str) -> dict:
        """Create the account and count the use – together, or not at all."""
        with self.db.tx() as c:
            invite = self.check(token)
            user = self.accounts.create(name, password, role="user", perms=invite["perms"], quota=invite["quota"])
            used_by = invite["used_by"] + [{"name": user["name"], "time": self.clock()}]
            done = c.execute("UPDATE invites SET uses=uses+1, data=? WHERE id=? AND uses<max_uses AND revoked=0",
                             (dbm.dumps({"quota": invite["quota"], "perms": invite["perms"], "used_by": used_by[-MAX_USES:]}),
                              invite["id"]))
            if done.rowcount != 1:                  # used up in the meantime: the account goes with the rollback
                raise InviteError(t("Diese Einladung gilt nicht (mehr). Bitte frag nach einem neuen Link."))
        return user
