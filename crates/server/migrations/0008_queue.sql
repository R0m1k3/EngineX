-- Étape 7a : modèle par session et par cron (NULL = preset par défaut), preset
-- par défaut distinct du preset chargé (la file peut en charger un autre).
ALTER TABLE chat_sessions ADD COLUMN preset_id INTEGER REFERENCES presets(id) ON DELETE SET NULL;
ALTER TABLE crons ADD COLUMN preset_id INTEGER REFERENCES presets(id) ON DELETE SET NULL;
INSERT OR IGNORE INTO settings (key, value) SELECT 'preset.default', value FROM settings WHERE key = 'preset.active';
