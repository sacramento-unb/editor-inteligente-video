CREATE TABLE analysis_results (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    confidence REAL NOT NULL,
    suggestion TEXT NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '{}',
    rule_version TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX analysis_project_media ON analysis_results(project_id, media_file_id, start_ms);

CREATE TABLE render_plans (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'DRAFT',
    plan_json TEXT NOT NULL,
    output_path TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    approved_at TEXT,
    rendered_at TEXT
);

CREATE INDEX render_plans_project_created ON render_plans(project_id, created_at);
