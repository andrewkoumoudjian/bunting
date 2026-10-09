# Bunting composition instructions

Keep this crate a thin, portable composition boundary over reusable packages. Re-export only deliberately stable first-party types and keep product metadata free of runtime state.

Do not duplicate matching, command-transaction, persistence, ledger, or risk logic here. Do not depend on `apps/`, create a nested workspace, or claim that the QUARCC port is complete.

`archive.rs` currently replays simulation commands only; do not describe it as a full trading replay until the archive covers every input (ADR 0025 as expanded by ADR 0028 item 5).
