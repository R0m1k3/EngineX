-- Étape 4a : coffre, projets et topics, sessions Codex.
CREATE TABLE secrets (
    id INTEGER PRIMARY KEY, scope TEXT NOT NULL, name TEXT NOT NULL,
    nonce BLOB NOT NULL, value BLOB NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    UNIQUE (scope, name)
);
CREATE TABLE projects (
    id INTEGER PRIMARY KEY, slug TEXT NOT NULL UNIQUE, name TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE topics (
    id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    slug TEXT NOT NULL, name TEXT NOT NULL, created_at INTEGER NOT NULL, UNIQUE (project_id, slug)
);
CREATE TABLE chat_sessions (
    id TEXT PRIMARY KEY,                       -- uuid v4
    topic_id INTEGER NOT NULL REFERENCES topics(id) ON DELETE RESTRICT,
    title TEXT NOT NULL, thread_id TEXT,
    approval TEXT NOT NULL DEFAULT 'auto' CHECK (approval IN ('auto','prudent')),
    archived INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX chat_sessions_topic ON chat_sessions(topic_id, updated_at);
CREATE TABLE chat_turns (
    id TEXT NOT NULL,                          -- id de tour Codex
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('inProgress','completed','interrupted','failed')),
    error TEXT, usage TEXT, started_at INTEGER NOT NULL, completed_at INTEGER,
    PRIMARY KEY (session_id, id)
);
CREATE TABLE chat_items (
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    turn_id TEXT,                              -- NULL le temps de la réponse à turn/start
    item_id TEXT NOT NULL,                     -- id Codex, ou "ex-<uuid>" pour les messages de Michael
    seq INTEGER NOT NULL,                      -- ordre d'arrivée dans la session (1, 2, 3…)
    kind TEXT NOT NULL,                        -- ThreadItem.type
    data TEXT NOT NULL,                        -- JSON de l'élément, masqué
    PRIMARY KEY (session_id, item_id)
);
CREATE INDEX chat_items_seq ON chat_items(session_id, seq);
ALTER TABLE api_keys ADD COLUMN internal INTEGER NOT NULL DEFAULT 0;
