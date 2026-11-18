-- Étape 3 : clés d'API de /v1 (seul le SHA-256 de la clé est gardé).
CREATE TABLE api_keys (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    hash         BLOB NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);
