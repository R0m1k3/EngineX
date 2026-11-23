-- Tester v2 : contrôles à points, verdict, note visuelle 1-5 (tests visuels seulement).
-- L'ancien barème (auto + 4 axes) ne se compare pas au nouveau : anciens runs effacés.
DROP TABLE IF EXISTS bench_cases;
DELETE FROM bench_runs;
CREATE TABLE bench_cases (
  id INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL REFERENCES bench_runs(id) ON DELETE CASCADE,
  case_key TEXT NOT NULL,
  session_id TEXT,
  status TEXT NOT NULL CHECK (status IN ('pending','running','done','failed')),
  reason TEXT,
  audit_json TEXT,
  points REAL,
  points_max INTEGER,
  verdict TEXT CHECK (verdict IN ('passed','partial','failed')),
  visual INTEGER CHECK (visual BETWEEN 1 AND 5),
  comment TEXT,
  audited_at INTEGER,
  started_at INTEGER,
  finished_at INTEGER
);
CREATE INDEX bench_cases_session ON bench_cases(session_id);
CREATE INDEX bench_cases_run ON bench_cases(run_id);
