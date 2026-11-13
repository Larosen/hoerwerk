CREATE TABLE follows (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        kind        TEXT NOT NULL,
        name        TEXT NOT NULL,
        language    TEXT NOT NULL DEFAULT '',
        region      TEXT NOT NULL DEFAULT '',
        auto        INTEGER NOT NULL DEFAULT 1,
        since       REAL NOT NULL,
        checked     REAL,
        data        TEXT NOT NULL DEFAULT '{}'
    );
    CREATE UNIQUE INDEX follows_person ON follows(user_id, kind, name)
