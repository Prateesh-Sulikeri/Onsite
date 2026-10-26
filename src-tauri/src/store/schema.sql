CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dossiers (
    id         TEXT PRIMARY KEY,
    company    TEXT NOT NULL,
    role       TEXT NOT NULL,
    level      TEXT,
    data       TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS research_jobs (
    id         INTEGER PRIMARY KEY,
    request    TEXT NOT NULL,
    status     TEXT NOT NULL,
    progress   INTEGER NOT NULL DEFAULT 0,
    log        TEXT NOT NULL DEFAULT '',
    raw_output TEXT,
    dossier_id TEXT REFERENCES dossiers(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS loops (
    id         INTEGER PRIMARY KEY,
    config     TEXT NOT NULL,
    verdict    TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rounds (
    id                INTEGER PRIMARY KEY,
    loop_id           INTEGER NOT NULL REFERENCES loops(id),
    idx               INTEGER NOT NULL,
    round_key         TEXT NOT NULL,
    name              TEXT NOT NULL,
    persona           TEXT NOT NULL,
    minutes           INTEGER NOT NULL,
    model             TEXT NOT NULL,
    claude_session_id TEXT,
    transcript        TEXT NOT NULL DEFAULT '[]',
    workspace         TEXT NOT NULL DEFAULT '{}',
    started_at        TEXT NOT NULL,
    ended_at          TEXT
);

CREATE TABLE IF NOT EXISTS reports (
    id         INTEGER PRIMARY KEY,
    round_id   INTEGER NOT NULL UNIQUE REFERENCES rounds(id),
    data       TEXT,
    raw        TEXT NOT NULL,
    created_at TEXT NOT NULL
);
