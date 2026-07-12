# Bunting composition instructions

Keep this crate a thin, portable composition boundary over reusable packages. Re-export only deliberately stable first-party types and keep product metadata free of runtime state.

Do not duplicate matching, command-transaction, persistence, ledger, or risk logic here. Do not expose Worker-only adapters by default, depend on `apps/`, create a nested workspace, or claim that the NBC or QUARCC ports are complete.
