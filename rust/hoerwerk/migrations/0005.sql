ALTER TABLE users ADD COLUMN origin TEXT NOT NULL DEFAULT '';
    ALTER TABLE users ADD COLUMN last_seen REAL;
    UPDATE users SET last_seen = (SELECT MAX(last_seen) FROM sessions WHERE sessions.user_id = users.id);
