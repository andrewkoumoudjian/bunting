# Native CLI instructions

Keep this application a thin command router over terminal and offline
competition utilities. It owns command-line parsing and local configuration
initialization, but no market, FIX session, storage or terminal behavior.

The released native executable is named `bunting`, with `bunting-server` and
`bunting-tui` as aliases routed by executable name. `bunting server <config>`
runs the venue in-process by calling `bunting_server::runtime::run`
(ADR 0044); keep server behavior in `apps/bunting-server`, not here.
