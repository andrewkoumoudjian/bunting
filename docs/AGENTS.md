# Documentation instructions

Documentation records decisions, invariants, contracts, migration requirements, rejected alternatives and source evidence. Do not describe planned behavior as implemented; label Target versus Current explicitly. Pin referenced GitHub commits. Link official Cloudflare documentation when describing the publication wrapper.

Before changing implementation guidance, read [`README.md`](README.md) (status map), the relevant ADRs including their status lines, `architecture.md`, `reference-functionality-audit.md`, and the applicable note under `ports/`. A port note must distinguish copied, translated and behavior-derived work and must record licensing before code is imported.

When a document is superseded, update `README.md` and add the historical banner at the top of the superseded document in the same commit. Never rewrite a historical document's body to look current.
