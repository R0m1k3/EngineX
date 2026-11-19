-- Effort de réflexion par session (`none`, `low`, `medium`, `high`) ; NULL =
-- `medium` (budget du preset). Envoyé à Codex à chaque `turn/start`.
ALTER TABLE chat_sessions ADD COLUMN effort TEXT;
