//! Replay one competition archive and print its verified result as JSON.
//!
//! Host-neutral on purpose: it uses only `std` file reads and the
//! `bunting-rs` archive API, so the same source builds natively and for
//! WASIX. `tools/host_parity.sh` runs both builds on one archive and
//! requires byte-identical output (ADR 0044).
use bunting_rs::CompetitionArchive;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: replay_archive <archive.json>")?;
    let archive = CompetitionArchive::from_json(&std::fs::read_to_string(path)?)
        .map_err(|error| format!("cannot read archive: {error}"))?;
    let result = archive
        .replay()
        .map_err(|error| format!("archive does not replay: {error}"))?;
    println!(
        "{}",
        serde_json::json!({
            "commands": result.command_count,
            "events": result.event_count,
            "final_chain": result.final_chain,
            "final_state_hash": result.final_state_hash,
            "scores": result.scores,
        })
    );
    Ok(())
}
