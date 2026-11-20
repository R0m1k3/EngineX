-- Dossier propre de la session sous `<topic>/sessions/<work_dir>` (seul
-- dossier monté en /workspace, supprimé avec la session). NULL = anciennes
-- sessions (dossier du topic) et copies séparées.
ALTER TABLE chat_sessions ADD COLUMN work_dir TEXT;
