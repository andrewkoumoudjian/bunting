# FIX bridge instructions

Expose standard local FIX/TCP and map supported application messages through the versioned typed tRPC client defined by ADR 0015. Own bounded durable FIX session state locally; keep FIX session sequences distinct from Bunting event sequences; never forward raw FIX frames to the Worker or rewrite sequence numbers or business fields silently. Support partial TCP reads, reconnect, resend and bounded backpressure.
