-- Étape 7c : agents. Un rôle durable dans un topic, lancé par un horaire, un
-- webhook, une notification du fil, un autre agent ou à la main ; une session
-- neuve par départ ; budget quotidien en jetons et en minutes de GPU.
CREATE TABLE agents (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    topic_id INTEGER NOT NULL REFERENCES topics(id) ON DELETE RESTRICT,
    preset_id INTEGER REFERENCES presets(id) ON DELETE SET NULL,
    role TEXT NOT NULL,
    schedule TEXT,                             -- cron 5 champs ou every:<min> ; NULL = pas d'horaire
    tz TEXT NOT NULL DEFAULT 'Europe/Paris',
    feed_filter TEXT,                          -- JSON {sources, min_level, contains} ; NULL = pas de fil
    approval TEXT NOT NULL DEFAULT 'auto' CHECK (approval IN ('auto','prudent')),
    timeout_s INTEGER NOT NULL DEFAULT 1800,
    budget_tokens INTEGER NOT NULL DEFAULT 0,  -- par jour ; 0 = sans limite
    budget_gpu_min INTEGER NOT NULL DEFAULT 0, -- par jour ; 0 = sans limite
    notify INTEGER NOT NULL DEFAULT 0,         -- résultat poussé (push) même réussi
    enabled INTEGER NOT NULL DEFAULT 1,
    next_run INTEGER,
    budget_note_day TEXT,                      -- jour de la dernière alerte de budget
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE agent_runs (
    id INTEGER PRIMARY KEY,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    trigger TEXT NOT NULL CHECK (trigger IN ('schedule','manual','webhook','feed','agent','session')),
    context TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK (status IN ('running','ok','failed','skipped')),
    started_at INTEGER NOT NULL, ended_at INTEGER,
    output TEXT NOT NULL DEFAULT '',
    session_id TEXT,
    tokens INTEGER NOT NULL DEFAULT 0,
    gpu_s INTEGER NOT NULL DEFAULT 0,
    depth INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX agent_runs_agent ON agent_runs(agent_id, id);
CREATE INDEX agent_runs_session ON agent_runs(session_id);
ALTER TABLE webhooks ADD COLUMN agent_id INTEGER REFERENCES agents(id) ON DELETE SET NULL;
