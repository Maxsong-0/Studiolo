CREATE TABLE IF NOT EXISTS agent_type (
    name TEXT PRIMARY KEY CHECK (length(name) BETWEEN 1 AND 32),
    token_sha256 TEXT NOT NULL UNIQUE CHECK (length(token_sha256) = 64),
    issued_at TEXT NOT NULL,
    revoked_at TEXT);
CREATE TABLE IF NOT EXISTS agent_instance (
    id TEXT PRIMARY KEY,
    type_name TEXT NOT NULL REFERENCES agent_type(name),
    session_digest TEXT NOT NULL,
    registered_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS reads (
    seq INTEGER PRIMARY KEY,
    recorded_at_ms INTEGER NOT NULL,
    recorded_at TEXT NOT NULL,
    type_name TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    path TEXT NOT NULL,
    revision TEXT NOT NULL,
    tool TEXT NOT NULL,
    ledger_offset INTEGER NOT NULL UNIQUE);
CREATE INDEX IF NOT EXISTS reads_by_type ON reads(type_name, recorded_at_ms);
CREATE INDEX IF NOT EXISTS reads_by_instance ON reads(instance_id, recorded_at_ms);
CREATE INDEX IF NOT EXISTS reads_by_path ON reads(path, recorded_at_ms);
CREATE INDEX IF NOT EXISTS reads_by_time ON reads(recorded_at_ms);
CREATE TABLE IF NOT EXISTS outage (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    denied_count INTEGER NOT NULL DEFAULT 0,
    reason TEXT NOT NULL);
