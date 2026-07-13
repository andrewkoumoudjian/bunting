PRAGMA foreign_keys = OFF;

CREATE TABLE snapshots_v2 (
    run_id TEXT NOT NULL,
    venue_id TEXT NOT NULL,
    instrument_id TEXT NOT NULL,
    represented_sequence TEXT NOT NULL,
    checksum TEXT NOT NULL,
    package_json TEXT NOT NULL,
    PRIMARY KEY (run_id, venue_id, instrument_id, represented_sequence, checksum),
    FOREIGN KEY (run_id) REFERENCES runs(run_id),
    CHECK (length(venue_id) BETWEEN 1 AND 39)
);

INSERT INTO snapshots_v2 (
    run_id,
    venue_id,
    instrument_id,
    represented_sequence,
    checksum,
    package_json
)
SELECT
    run_id,
    '1',
    instrument_id,
    represented_sequence,
    checksum,
    package_json
FROM snapshots;

DROP TABLE snapshots;
ALTER TABLE snapshots_v2 RENAME TO snapshots;

PRAGMA foreign_keys = ON;
