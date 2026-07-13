# FIX bridge instructions

This legacy client bridge is not the production FIX boundary after ADR 0020. Production FIX sessions live in `apps/bunting-worker`, initiate outbound TCP, map to in-process application calls, and keep FIX sequences distinct from Bunting event sequences. Retain this path only for external conformance tooling; never route production FIX through browser RPC.
