CREATE TABLE kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE rename_history (
        id     TEXT PRIMARY KEY,
        time   REAL NOT NULL,
        folder TEXT NOT NULL,
        data   TEXT NOT NULL
    );
    CREATE INDEX rename_history_time ON rename_history(time);
    CREATE TABLE presets (
        name  TEXT PRIMARY KEY,
        rules TEXT NOT NULL
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
    CREATE INDEX jobs_status ON jobs(kind, status);
    CREATE INDEX jobs_source ON jobs(source);
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
    CREATE INDEX library_asin ON library(asin);
    CREATE TABLE grabs (
        name TEXT PRIMARY KEY,
        key  TEXT NOT NULL,
        time REAL NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX grabs_key ON grabs(key);
    CREATE INDEX grabs_time ON grabs(time);
