# Third-party source notices

This application contains modified source from:

- longbridge/longbridge-terminal at 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3: the complete `src/tui` tree was copied as the adaptation input. Copyright 2026 Longbridge, Apache License 2.0.
- cli-candlestick-chart version 0.24.0 from the same pinned Longbridge tree: the required chart library modules are adapted under `src/chart/`, and its ANSI-to-TUI renderer pattern is used for the FIX order-book graph. Its CLI, examples, optional integrations, and git dependency are excluded. Copyright (c) 2021 Julien-R44, MIT License.
- makeev/alphai-tui at f814697c6159d76b2dfb503ba5201b8c3fb702ad: retained license/provenance for the earlier terminal implementation; no AlphaAI source remains active in the Longbridge-first TUI. Copyright (c) 2026 Mikhail Makeev, MIT License.

Bunting removed Longbridge brokerage, quote, account and watchlist systems and changed the retained application, key, navigation, popup, rendering, view, UI-helper and widget components for Bunting branding, bounded in-memory FIX logs, engine order-book rendering, execution reports, and FIX command workflows. The candlestick chart consumes only bounded projections decoded from Bunting FIX market-data snapshots. Changed source files carry prominent modification notices. The original licenses follow in LICENSES/.

Longbridge Attribution Notice
Copyright 2026 Longbridge.

This product uses the Longbridge open-source version provided by Longbridge.
