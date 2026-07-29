# Binding instructions

Bindings expose only the concrete `bunting-rs` façade. Keep ownership explicit,
copy values across language boundaries, bound every foreign input, and keep
unsafe Rust confined to the reviewed C ABI crate. Binding crates use explicit
lint tables under ADR 0026 and are excluded from default workspace members.
