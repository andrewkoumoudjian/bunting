# Market-making crate instructions

This crate contains pure strategy analytics and desired-quote generation. It is platform-neutral and protocol-neutral.

Allowed modules include volatility and imbalance estimators, Avellaneda-Stoikov/GLFT-style calculations, inventory skew, quote clamps, warmup state and exact conversion of proposed values into tick/lot intents.

Do not perform HTTP, WebSocket, persistence, sleeps, wall-clock reads, order submission, fill inference, account reconciliation or authoritative risk/accounting here. Floating or decimal analytics must reject non-finite values and cross an explicit side-aware rounding boundary before producing `PriceTicks` or `QuantityLots`.

Every model has a version, explicit configuration, bounded state, independent golden vectors and snapshot/restore tests. Optional FFT or GARCH modules are deferred until simpler models demonstrate measured value.