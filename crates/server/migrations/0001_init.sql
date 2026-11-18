-- Réglages clé/valeur (password_hash) et sessions. Dates en secondes Unix.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY,
    created_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
