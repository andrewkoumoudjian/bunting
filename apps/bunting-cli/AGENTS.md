# Unified native CLI instructions

Keep this application a thin command router over the native server, relay and
terminal libraries. It owns command-line parsing, local configuration
initialization and release compatibility aliases, but no market, FIX session,
storage or terminal behavior.

The released executable is named `bunting`. The legacy `bunting-server` and
`bunting-tui` names are temporary one-release aliases and must execute the same
compiled binary without changing protocol behavior.
