# Native Bunting TUI instructions

- This app is a participant/operator client and test harness, never a market authority.
- Today it is a FIX initiator. Under ADR 0031 it becomes the first Bunting Native Protocol client through the shared `bunting-client` crate; do not build a TUI-private protocol.
- Its optional loopback acceptor invokes `bunting-engine` in-process for local testing only; it is not a second venue and must not diverge from server semantics.
- Preserve the Longbridge-derived application, navigation, popup, view, UI-helper and widget separation. Longbridge brokerage and quote systems are not Bunting authorities.
- Keep FIX framing/session behavior in `simfix-*`, keep all buffers bounded, and show raw redacted FIX logs.
