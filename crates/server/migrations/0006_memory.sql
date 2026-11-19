-- Étape 5 : mémoire à 3 niveaux, versions, index du recall.
ALTER TABLE chat_sessions ADD COLUMN memory_seen TEXT;   -- empreinte du bloc mémoire que l'agent a reçu

CREATE TABLE memory_revisions (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,            -- relatif à <data>/workspace/memory (memory.md, <p>/memory.md, …)
    content TEXT NOT NULL,         -- version remplacée
    author TEXT NOT NULL,          -- 'michael' | 'session:<id>'
    created_at INTEGER NOT NULL
);
CREATE INDEX memory_revisions_path ON memory_revisions(path, id);

-- Index du recall : une ligne par message (de Michael ou de l'agent) ; clé entière
-- déclarée (stable au VACUUM, contrairement au rowid implicite de chat_items).
CREATE TABLE recall_docs (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    UNIQUE (session_id, item_id)
);
CREATE VIRTUAL TABLE recall_fts USING fts5(text, tokenize = 'unicode61 remove_diacritics 2');

CREATE TRIGGER recall_items_ai AFTER INSERT ON chat_items
WHEN new.kind IN ('userMessage', 'agentMessage') AND COALESCE(json_extract(new.data, '$.text'), '') <> ''
BEGIN
    INSERT INTO recall_docs (session_id, item_id) VALUES (new.session_id, new.item_id);
    INSERT INTO recall_fts (rowid, text) VALUES (last_insert_rowid(), json_extract(new.data, '$.text'));
END;

CREATE TRIGGER recall_items_ad AFTER DELETE ON chat_items
BEGIN
    DELETE FROM recall_fts WHERE rowid IN
        (SELECT id FROM recall_docs WHERE session_id = old.session_id AND item_id = old.item_id);
    DELETE FROM recall_docs WHERE session_id = old.session_id AND item_id = old.item_id;
END;

INSERT INTO recall_docs (session_id, item_id)
    SELECT session_id, item_id FROM chat_items
    WHERE kind IN ('userMessage', 'agentMessage') AND COALESCE(json_extract(data, '$.text'), '') <> ''
    ORDER BY rowid;
INSERT INTO recall_fts (rowid, text)
    SELECT d.id, json_extract(i.data, '$.text') FROM recall_docs d
    JOIN chat_items i ON i.session_id = d.session_id AND i.item_id = d.item_id;
