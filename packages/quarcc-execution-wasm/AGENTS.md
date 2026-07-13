# QUARCC Wasm-binding instructions

Expose only serialized portable-core operations. Do not add browser networking,
ambient clocks, storage, or venue authority. Native and Wasm behavior must use
the same `quarcc-execution-engine` state machine and snapshot format.
