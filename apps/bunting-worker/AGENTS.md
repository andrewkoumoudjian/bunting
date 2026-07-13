# Bunting Worker instructions

- Browser and Rust/WASM clients use bounded fetch/streaming handlers.
- Worker components invoke application and engine functions directly in-process.
- FIX Durable Objects may initiate outbound TCP, but never accept inbound raw TCP or own market authority.
- Authenticate private operations and keep every buffer and journal bounded.
