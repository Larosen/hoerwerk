CREATE TABLE users (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
        pw          TEXT NOT NULL,
        role        TEXT NOT NULL DEFAULT 'user',
        perms       TEXT NOT NULL DEFAULT '{}',
        quota       INTEGER,
        disabled    INTEGER NOT NULL DEFAULT 0,
        created     REAL NOT NULL,
        last_login  REAL,
        pw_changed  REAL NOT NULL
    );
    CREATE TABLE sessions (
        token_hash  TEXT PRIMARY KEY,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        created     REAL NOT NULL,
        last_seen   REAL NOT NULL,
        expires     REAL NOT NULL,
        persistent  INTEGER NOT NULL DEFAULT 0,
        ip          TEXT NOT NULL DEFAULT '',
        agent       TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX sessions_user ON sessions(user_id);
    CREATE INDEX sessions_expires ON sessions(expires);
