CREATE TABLE invites (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        token       TEXT NOT NULL UNIQUE,
        note        TEXT NOT NULL DEFAULT '',
        created     REAL NOT NULL,
        created_by  TEXT NOT NULL DEFAULT '',
        expires     REAL,
        max_uses    INTEGER NOT NULL DEFAULT 1,
        uses        INTEGER NOT NULL DEFAULT 0,
        revoked     INTEGER NOT NULL DEFAULT 0,
        data        TEXT NOT NULL DEFAULT '{}'
    );
