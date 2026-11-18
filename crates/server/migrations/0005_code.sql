-- Étape 4b : instantanés, retour arrière, copie séparée, vérification.
ALTER TABLE chat_turns ADD COLUMN origin TEXT NOT NULL DEFAULT 'user';  -- 'user' | 'verify'
ALTER TABLE chat_turns ADD COLUMN snap_pre TEXT;
ALTER TABLE chat_turns ADD COLUMN snap_post TEXT;
ALTER TABLE chat_turns ADD COLUMN changes TEXT;            -- JSON {files, truncated}
ALTER TABLE chat_sessions ADD COLUMN copy_branch TEXT;     -- NULL = dossier du topic
ALTER TABLE chat_sessions ADD COLUMN pending_note TEXT;
ALTER TABLE chat_sessions ADD COLUMN pending_revert TEXT;  -- id du tour à retirer chez Codex
ALTER TABLE chat_sessions ADD COLUMN undo_snap TEXT;
CREATE TABLE chat_reverted (                               -- tours retirés par « fichiers et conversation »
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL, PRIMARY KEY (session_id, turn_id)
);
CREATE TABLE verifications (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL, attempt INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running','passed','failed','error','cancelled')),
    steps TEXT NOT NULL, started_at INTEGER NOT NULL, completed_at INTEGER
);
CREATE INDEX verifications_turn ON verifications(session_id, turn_id);
