//! Cloudflare D1 adapter for atomic expected-version origin commits.

use bunting_engine::{EngineSnapshotEnvelope, RunState};
use bunting_market_events::EventEnvelope;
use bunting_origin_store::{CommandResult, CommitOutcome, CommitRequest, OriginError};
use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, D1Type};

const INSERT_GUARD: &str = "INSERT INTO command_guards(run_id, command_id, fingerprint, expected_version, actor_id, session_id, local_command_id, local_order_id) SELECT ?, ?, ?, ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM runs WHERE run_id = ? AND version = ?)";
const INSERT_EVENT: &str = "INSERT INTO events(run_id, sequence, command_id, event_json) SELECT ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM command_guards WHERE run_id = ? AND command_id = ? AND fingerprint = ?)";
const INSERT_COMMAND: &str = "INSERT INTO commands(run_id, command_id, fingerprint, payload_json, result_json, committed_version) SELECT ?, ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM command_guards WHERE run_id = ? AND command_id = ? AND fingerprint = ?)";
const INSERT_SNAPSHOT: &str = "INSERT INTO snapshots(run_id, venue_id, instrument_id, represented_sequence, checksum, package_json) SELECT ?, ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM command_guards WHERE run_id = ? AND command_id = ? AND fingerprint = ?)";
const UPDATE_RUN: &str = "UPDATE runs SET version = ?, state_json = ? WHERE run_id = ? AND version = ? AND EXISTS (SELECT 1 FROM command_guards WHERE run_id = ? AND command_id = ? AND fingerprint = ?)";

#[derive(Debug, Deserialize)]
struct RunRow {
    state_json: String,
}

#[derive(Debug, Deserialize)]
struct CommandRow {
    fingerprint: String,
    result_json: String,
}

#[derive(Debug, Deserialize)]
struct VersionRow {
    version: String,
}

#[derive(Debug, Deserialize)]
struct EventRow {
    event_json: String,
}

/// Loads a bounded committed event tail in numeric sequence order.
pub async fn load_event_tail(
    database: &D1Database,
    run_id: &str,
    after_sequence: u64,
    limit: usize,
) -> Result<Vec<EventEnvelope>, OriginError> {
    let limit = u32::try_from(limit).map_err(|_| OriginError::Unavailable)?;
    let statement = bind(
        database,
        "SELECT event_json FROM events WHERE run_id = ? AND CAST(sequence AS INTEGER) > CAST(? AS INTEGER) ORDER BY CAST(sequence AS INTEGER) ASC LIMIT ?",
        &[run_id.to_owned(), after_sequence.to_string(), limit.to_string()],
    )
    .map_err(|_| OriginError::Unavailable)?;
    statement
        .all()
        .await
        .map_err(|_| OriginError::Unavailable)?
        .results::<EventRow>()
        .map_err(|_| OriginError::Unavailable)?
        .into_iter()
        .map(|row| serde_json::from_str(&row.event_json).map_err(|_| OriginError::Unavailable))
        .collect()
}

fn bind(
    database: &D1Database,
    sql: &str,
    values: &[String],
) -> worker::Result<D1PreparedStatement> {
    let arguments: Vec<D1Type<'_>> = values
        .iter()
        .map(|value| D1Type::Text(value.as_str()))
        .collect();
    database.prepare(sql).bind_refs(&arguments)
}

/// Loads the complete authoritative recovery projection.
pub async fn load_run(database: &D1Database, run_id: &str) -> Result<RunState, OriginError> {
    let statement = bind(
        database,
        "SELECT state_json FROM runs WHERE run_id = ?",
        &[run_id.to_string()],
    )
    .map_err(|_| OriginError::Unavailable)?;
    let row = statement
        .first::<RunRow>(None)
        .await
        .map_err(|_| OriginError::Unavailable)?
        .ok_or(OriginError::UnknownRun)?;
    EngineSnapshotEnvelope::from_persisted_json(&row.state_json)
        .map(|envelope| envelope.state)
        .map_err(|_| OriginError::Unavailable)
}

/// Looks up a durable idempotency response.
pub async fn find_command(
    database: &D1Database,
    run_id: &str,
    command_id: &str,
) -> Result<Option<(String, CommandResult)>, OriginError> {
    let statement = bind(
        database,
        "SELECT fingerprint, result_json FROM commands WHERE run_id = ? AND command_id = ?",
        &[run_id.to_string(), command_id.to_string()],
    )
    .map_err(|_| OriginError::Unavailable)?;
    let row = statement
        .first::<CommandRow>(None)
        .await
        .map_err(|_| OriginError::Unavailable)?;
    row.map(|record| {
        serde_json::from_str(&record.result_json)
            .map(|result| (record.fingerprint, result))
            .map_err(|_| OriginError::Unavailable)
    })
    .transpose()
}

/// Atomically appends events, idempotency, projections, and snapshot metadata.
#[expect(
    clippy::too_many_lines,
    reason = "the statement order is the reviewed D1 transaction contract"
)]
pub async fn commit(
    database: &D1Database,
    request: &CommitRequest,
    command_json: &str,
) -> Result<CommitOutcome, OriginError> {
    if let Some((stored_fingerprint, result)) = find_command(
        database,
        &request.run_id.to_string(),
        &request.command_id.to_string(),
    )
    .await?
    {
        return if stored_fingerprint == request.fingerprint {
            Ok(CommitOutcome::Duplicate(result))
        } else {
            Err(OriginError::IdempotencyConflict)
        };
    }

    let run_id = request.run_id.to_string();
    let command_id = request.command_id.to_string();
    let expected = request.expected_version.to_string();
    let fingerprint = request.fingerprint.clone();
    let (actor_id, session_id, local_command_id, local_order_id) = request.client_key.map_or_else(
        || (String::new(), String::new(), String::new(), String::new()),
        |key| {
            (
                key.actor.to_string(),
                key.session_id.to_string(),
                key.local_command_id.to_string(),
                key.local_order_id
                    .map_or_else(String::new, |id| id.to_string()),
            )
        },
    );
    let mut statements =
        Vec::with_capacity(request.events.len() + request.candidate.listings().len() + 3);
    statements.push(
        bind(
            database,
            INSERT_GUARD,
            &[
                run_id.clone(),
                command_id.clone(),
                fingerprint.clone(),
                expected.clone(),
                actor_id,
                session_id,
                local_command_id,
                local_order_id,
                run_id.clone(),
                expected.clone(),
            ],
        )
        .map_err(|_| OriginError::Unavailable)?,
    );
    for event in &request.events {
        let event_json = serde_json::to_string(event).map_err(|_| OriginError::InvalidCommit)?;
        statements.push(
            bind(
                database,
                INSERT_EVENT,
                &[
                    run_id.clone(),
                    event.sequence.to_string(),
                    command_id.clone(),
                    event_json,
                    run_id.clone(),
                    command_id.clone(),
                    fingerprint.clone(),
                ],
            )
            .map_err(|_| OriginError::Unavailable)?,
        );
    }
    let result_json =
        serde_json::to_string(&request.result).map_err(|_| OriginError::InvalidCommit)?;
    statements.push(
        bind(
            database,
            INSERT_COMMAND,
            &[
                run_id.clone(),
                command_id.clone(),
                fingerprint.clone(),
                command_json.to_string(),
                result_json,
                request.result.committed_sequence.to_string(),
                run_id.clone(),
                command_id.clone(),
                fingerprint.clone(),
            ],
        )
        .map_err(|_| OriginError::Unavailable)?,
    );
    for (listing_key, listing) in request.candidate.listings() {
        let snapshot = listing.snapshot();
        if snapshot.represented_sequence != request.result.committed_sequence {
            continue;
        }
        statements.push(
            bind(
                database,
                INSERT_SNAPSHOT,
                &[
                    run_id.clone(),
                    listing_key.venue_id.to_string(),
                    listing_key.instrument_id.to_string(),
                    snapshot.represented_sequence.to_string(),
                    snapshot.checksum.clone(),
                    snapshot.package_json.clone(),
                    run_id.clone(),
                    command_id.clone(),
                    fingerprint.clone(),
                ],
            )
            .map_err(|_| OriginError::Unavailable)?,
        );
    }
    let state_json = request
        .candidate
        .snapshot_envelope()
        .and_then(|envelope| envelope.to_json())
        .map_err(|_| OriginError::InvalidCommit)?;
    statements.push(
        bind(
            database,
            UPDATE_RUN,
            &[
                request.candidate.sequence().to_string(),
                state_json,
                run_id.clone(),
                expected,
                run_id.clone(),
                command_id,
                fingerprint,
            ],
        )
        .map_err(|_| OriginError::Unavailable)?,
    );
    let Ok(results) = database.batch(statements).await else {
        if let Some((stored_fingerprint, result)) =
            find_command(database, &run_id, &request.command_id.to_string()).await?
        {
            return if stored_fingerprint == request.fingerprint {
                Ok(CommitOutcome::Duplicate(result))
            } else {
                Err(OriginError::IdempotencyConflict)
            };
        }
        return Err(OriginError::Unavailable);
    };
    let updated = results
        .last()
        .ok_or(OriginError::InvalidCommit)?
        .meta()
        .map_err(|_| OriginError::Unavailable)?
        .and_then(|meta| meta.changes)
        .unwrap_or(0);
    if updated == 1 {
        return Ok(CommitOutcome::Committed(request.result.clone()));
    }
    let statement = bind(
        database,
        "SELECT version FROM runs WHERE run_id = ?",
        std::slice::from_ref(&run_id),
    )
    .map_err(|_| OriginError::Unavailable)?;
    let current_value = statement
        .first::<VersionRow>(None)
        .await
        .map_err(|_| OriginError::Unavailable)?
        .ok_or(OriginError::UnknownRun)?
        .version
        .parse::<u64>()
        .map_err(|_| OriginError::Unavailable)?;
    Err(OriginError::VersionConflict {
        current: bunting_market_types::EventSequence::new(current_value),
    })
}
