# Native Bunting server instructions

Keep this app a thin native adapter over `bunting-application`. Sockets,
filesystem persistence, and TLS termination belong here; market
authority, matching, canonical events, identity authorization, and commit
preparation do not. Bound every connection, request, journal, and recovery file.
