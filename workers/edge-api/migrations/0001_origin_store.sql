PRAGMA foreign_keys = ON;

CREATE TABLE runs (
    run_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    state_json TEXT NOT NULL,
    CHECK (length(run_id) BETWEEN 1 AND 39),
    CHECK (length(version) BETWEEN 1 AND 20)
);

CREATE TABLE command_guards (
    run_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    expected_version TEXT NOT NULL,
    PRIMARY KEY (run_id, command_id),
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);

CREATE TABLE commands (
    run_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    result_json TEXT NOT NULL,
    committed_version TEXT NOT NULL,
    PRIMARY KEY (run_id, command_id),
    FOREIGN KEY (run_id, command_id)
        REFERENCES command_guards(run_id, command_id)
);

CREATE TABLE events (
    run_id TEXT NOT NULL,
    sequence TEXT NOT NULL,
    command_id TEXT NOT NULL,
    event_json TEXT NOT NULL,
    PRIMARY KEY (run_id, sequence),
    FOREIGN KEY (run_id, command_id)
        REFERENCES command_guards(run_id, command_id)
);

CREATE TABLE snapshots (
    run_id TEXT NOT NULL,
    instrument_id TEXT NOT NULL,
    represented_sequence TEXT NOT NULL,
    checksum TEXT NOT NULL,
    package_json TEXT NOT NULL,
    PRIMARY KEY (run_id, instrument_id, represented_sequence, checksum),
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);

CREATE INDEX events_by_command ON events(run_id, command_id);
