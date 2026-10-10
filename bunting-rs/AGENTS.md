# Bunting composition instructions

Keep this crate a thin, portable composition boundary over reusable packages. Re-export only deliberately stable first-party types and keep product metadata free of runtime state.

Do not duplicate matching, command-transaction, persistence, ledger, or risk logic here. Do not depend on `apps/`, create a nested workspace, or claim that the QUARCC port is complete.

`archive.rs` (version 2) is a run's genesis plus its journaled command records, replayed through `bunting_origin_store::RunRecovery`. Do not add a second replay path here: archive verification must stay the same code a restart uses.
