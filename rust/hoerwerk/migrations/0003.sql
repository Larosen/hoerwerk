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
    CREATE INDEX requests_user ON requests(user_id, created);
    CREATE INDEX requests_status ON requests(status);
    CREATE INDEX requests_key ON requests(book_key);
    ALTER TABLE users ADD COLUMN totp TEXT;
    ALTER TABLE users ADD COLUMN prefs TEXT NOT NULL DEFAULT '{}';
    CREATE TABLE audit (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        time    REAL NOT NULL,
        user    TEXT NOT NULL,
        ip      TEXT NOT NULL,
        action  TEXT NOT NULL,
        detail  TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX audit_time ON audit(time);
