-- Étape 2 : modèles locaux, téléchargements, presets, moteurs llama.cpp.
CREATE TABLE models (
    id         INTEGER PRIMARY KEY,
    repo       TEXT NOT NULL,
    file       TEXT NOT NULL,            -- fichier principal (1re tranche)
    shards     TEXT NOT NULL,            -- JSON : ["a-00001-of-00002.gguf", ...]
    mmproj     TEXT,                     -- nom de fichier, NULL si absent
    size_bytes INTEGER NOT NULL,         -- poids, toutes tranches
    sha256     TEXT NOT NULL,            -- JSON : {"fichier": "sha"}
    meta       TEXT NOT NULL,            -- JSON GgufMeta
    created_at INTEGER NOT NULL,
    UNIQUE (repo, file)
);
CREATE TABLE downloads (
    id             INTEGER PRIMARY KEY,
    repo           TEXT NOT NULL,
    files          TEXT NOT NULL,        -- JSON : [FileSpec] (tranches puis mmproj)
    mmproj         TEXT,                 -- nom du mmproj demandé, NULL sinon
    received_bytes INTEGER NOT NULL DEFAULT 0,
    total_bytes    INTEGER NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('queued','running','paused','done','failed','cancelled')),
    error          TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE TABLE presets (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    model_id   INTEGER NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    settings   TEXT NOT NULL,            -- JSON PresetSettings
    sources    TEXT NOT NULL,            -- JSON Merged
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE engine_versions (
    tag          TEXT PRIMARY KEY,       -- 'image' ou 'server-cuda-bNNNNN'
    build        TEXT NOT NULL,
    path         TEXT NOT NULL,          -- chemin du binaire llama-server
    bin_mtime    INTEGER NOT NULL,
    installed_at INTEGER NOT NULL,
    last_test    TEXT                    -- JSON : {ok, code, message, gpus, has_fit, at}
);
