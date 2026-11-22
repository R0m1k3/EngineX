-- Page « Tester » : exécutions (runs) de tests de codage et résultat de chaque cas.
CREATE TABLE bench_runs (
  id INTEGER PRIMARY KEY,
  preset_id INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running','done','stopped')),
  started_at INTEGER NOT NULL,
  finished_at INTEGER
);
CREATE TABLE bench_cases (
  id INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL REFERENCES bench_runs(id) ON DELETE CASCADE,
  case_key TEXT NOT NULL,
  session_id TEXT,
  status TEXT NOT NULL CHECK (status IN ('pending','running','done','failed')),
  reason TEXT,
  audit_json TEXT,
  auto_score REAL,
  manual_json TEXT,
  manual_score REAL,
  comment TEXT,
  audited_at INTEGER,
  started_at INTEGER,
  finished_at INTEGER
);
CREATE INDEX bench_cases_session ON bench_cases(session_id);
CREATE INDEX bench_cases_run ON bench_cases(run_id);
