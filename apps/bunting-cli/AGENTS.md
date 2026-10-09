# Native CLI instructions

Keep this application a thin command router over terminal and offline
competition utilities. It owns command-line parsing and local configuration
initialization, but no market, FIX session, storage or terminal behavior.

The released native executable is named `bunting`, with `bunting-tui` as its
compatibility alias. The `bunting-server` command is a separate launcher for
the server artifact (currently a WASIX module, ADR 0027; host direction in
ADR 0033) and must not route through this native binary.
