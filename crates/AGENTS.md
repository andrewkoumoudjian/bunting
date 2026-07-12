# Future scaffold instructions

The Cargo-less directories under `crates/` are future scaffolds, not active workspace packages. Do not move them into `packages/` or add manifests until their roadmap phase introduces real implementation, tests and a reviewed reusable boundary.

Do not build a parallel matching engine. Reusable first-party implementation belongs under `packages/`; deployment-specific implementation belongs under `apps/`.
