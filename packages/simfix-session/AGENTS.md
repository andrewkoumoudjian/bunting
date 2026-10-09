# simfix-session instructions

Implement the FIX session state machine behind clock, transport, and message-store traits. No Tokio, sockets, files, native TLS, threads, or Worker APIs.

Periodic TestRequest/Heartbeat round-trip measurement for ADR 0030 (target) belongs here behind the clock trait, with server-side monotonic send stamps; never trust client-supplied timestamps for ordering.
