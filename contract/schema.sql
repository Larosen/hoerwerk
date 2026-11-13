-- Database schema of Hörwerk (PRAGMA user_version = 6). Every backend creates and accepts exactly this.
-- Generated from app/db.py MIGRATIONS; contract/test_contract.py compares a fresh database against it.
CREATE INDEX audit_time ON audit(time);
CREATE UNIQUE INDEX follows_person ON follows(user_id, kind, name);
CREATE INDEX grabs_key ON grabs(key);
CREATE INDEX grabs_time ON grabs(time);
CREATE INDEX jobs_source ON jobs(source);
CREATE INDEX jobs_status ON jobs(kind, status);
CREATE INDEX library_asin ON library(asin);
CREATE INDEX rename_history_time ON rename_history(time);
CREATE INDEX requests_key ON requests(book_key);
CREATE INDEX requests_status ON requests(status);
CREATE INDEX requests_user ON requests(user_id, created);
CREATE INDEX sessions_expires ON sessions(expires);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE TABLE audit (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        time    REAL NOT NULL,
        user    TEXT NOT NULL,
        ip      TEXT NOT NULL,
        action  TEXT NOT NULL,
        detail  TEXT NOT NULL DEFAULT ''
    );
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
CREATE TABLE grabs (
        name TEXT PRIMARY KEY,
        key  TEXT NOT NULL,
        time REAL NOT NULL,
        data TEXT NOT NULL
    );
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
CREATE TABLE jobs (
        id      TEXT PRIMARY KEY,
        kind    TEXT NOT NULL,
        status  TEXT NOT NULL,
        source  TEXT NOT NULL,
        created REAL NOT NULL,
        updated REAL NOT NULL,
        data    TEXT NOT NULL
    );
CREATE TABLE kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
CREATE TABLE library (
        path            TEXT PRIMARY KEY,
        title           TEXT NOT NULL DEFAULT '',
        authors         TEXT NOT NULL DEFAULT '[]',
        asin            TEXT,
        series          TEXT,
        series_position TEXT,
        format          TEXT,
        files           INTEGER NOT NULL DEFAULT 0,
        mtime           REAL NOT NULL DEFAULT 0
    );
CREATE TABLE presets (
        name  TEXT PRIMARY KEY,
        rules TEXT NOT NULL
    );
CREATE TABLE rename_history (
        id     TEXT PRIMARY KEY,
        time   REAL NOT NULL,
        folder TEXT NOT NULL,
        data   TEXT NOT NULL
    );
CREATE TABLE requests (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
        book_key    TEXT NOT NULL,
        book        TEXT NOT NULL,
        status      TEXT NOT NULL,
        created     REAL NOT NULL,
        updated     REAL NOT NULL,
        data        TEXT NOT NULL DEFAULT '{}'
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
    , totp TEXT, prefs TEXT NOT NULL DEFAULT '{}', origin TEXT NOT NULL DEFAULT '', last_seen REAL);
