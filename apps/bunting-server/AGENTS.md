# Bunting WASI server instructions

Keep this app a thin WASI host adapter over `bunting-application`. Wasmer-hosted
sockets, filesystem persistence, and TLS termination belong here; market
authority, matching, canonical events, identity authorization, and commit
preparation do not. Bound every connection, request, journal, and recovery file.
