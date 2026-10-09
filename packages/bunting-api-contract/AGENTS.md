# API contract package instructions

This package owns shared identity and role types (`ActorRole`, `ActorIdentity`, `Audience`, validated decimal-string IDs) used by the server, plus the deterministic contract descriptor. The browser procedure set is **retired under ADR 0031**: do not add browser procedures. The Bunting Native Protocol message schema is the target owner of app-facing types here or in a renamed successor crate.

Keep FIX parsing in `simfix-wire`, market behavior outside this package, and externally wide integers as validated decimal strings.
