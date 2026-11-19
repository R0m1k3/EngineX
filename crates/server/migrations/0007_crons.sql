-- Étape 6a : tâches planifiées. Une session « Cron : <nom> » par cron, gardée
-- d'une exécution à l'autre (recréée si Michael la supprime).
CREATE TABLE crons (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    topic_id INTEGER NOT NULL REFERENCES topics(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('prompt','shell')),
    body TEXT NOT NULL,
    schedule TEXT NOT NULL,                    -- cron 5 champs, ou "every:<minutes>"
    tz TEXT NOT NULL DEFAULT 'Europe/Paris',
    timeout_s INTEGER NOT NULL DEFAULT 1800,
    enabled INTEGER NOT NULL DEFAULT 1,
    session_id TEXT,
    next_run INTEGER,                          -- secondes Unix ; NULL si désactivé
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE cron_runs (
    id INTEGER PRIMARY KEY,
    cron_id INTEGER NOT NULL REFERENCES crons(id) ON DELETE CASCADE,
    trigger TEXT NOT NULL CHECK (trigger IN ('schedule','manual')),
    status TEXT NOT NULL CHECK (status IN ('running','ok','failed','skipped')),
    started_at INTEGER NOT NULL, ended_at INTEGER,
    output TEXT NOT NULL DEFAULT '',
    session_id TEXT
);
CREATE INDEX cron_runs_cron ON cron_runs(cron_id, id);
