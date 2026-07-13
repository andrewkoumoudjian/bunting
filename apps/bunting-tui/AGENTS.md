# Native Bunting TUI instructions

- This app is a native test harness, never a production market authority or Worker dependency.
- The TUI is a FIX initiator. Its optional loopback server is a native-only FIX acceptor that invokes `bunting-engine` directly in-process.
- Preserve the Longbridge-derived application, navigation, popup, view, UI-helper and widget separation. Longbridge brokerage and quote systems are not Bunting authorities.
- Keep FIX framing/session behavior in `simfix-*`, keep all buffers bounded, and show raw redacted FIX logs.
- Do not claim the loopback acceptor changes the Cloudflare Worker rule: the Worker still initiates outbound TCP and never accepts raw TCP.
