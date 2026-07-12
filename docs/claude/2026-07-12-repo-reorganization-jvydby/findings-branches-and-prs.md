# Findings — branches & pull requests

Captured 2026-07-12 via the GitHub API (`list_branches`, `list_pull_requests`) and local git.

## Pull requests

| PR | Title | Head branch | State |
|----|-------|-------------|-------|
| #1 | Bootstrap Bunting architecture, Rust kernel, streaming, strategy isolation | `feat/bootstrap-architecture` | **merged** (`merged_at` set) |
| #2 | Adopt OrderBook-rs as the Worker kernel and add Workers Cache recovery | `feat/orderbook-rs-worker-kernel` | **merged** |
| #3 | Implement origin-backed command transactions | `feat/command-transaction-origin-store` | **merged** |

Note: the `list_pull_requests` endpoint reports `"merged": false` even for merged PRs — a known API
quirk. The authoritative signal is `merged_at` being non-null, which is set for all three.

## Remote branches at session time

`main`, `feat/bootstrap-architecture`, `feat/orderbook-rs-worker-kernel`,
`feat/deterministic-kernel-vertical-slice` (+ the working branch
`claude/bunting-repo-reorganization-jvydby`). The PR #3 branch
`feat/command-transaction-origin-store` was already deleted on the remote.

## Delete / archive recommendations

| Branch | State | Recommendation |
|---|---|---|
| `main` | default | **Keep.** |
| `claude/bunting-repo-reorganization-jvydby` | working branch | **Keep.** |
| `feat/bootstrap-architecture` | PR #1 merged | **Delete** — fully merged. |
| `feat/orderbook-rs-worker-kernel` | PR #2 merged | **Delete** — fully merged. |
| `feat/command-transaction-origin-store` | PR #3 merged; remote ref already absent | No action. |
| `feat/deterministic-kernel-vertical-slice` | **no PR, never merged** | **Archive then delete.** Its custom `BTreeMap`/arena order book was declared obsolete by ADR-0013; useful parts (checked IDs/units, canonical events, ledger, participant risk) were already salvaged into PR #2. Tag `archive/deterministic-kernel-vertical-slice` for provenance, then delete the branch. |

This matches the "Branch disposition" table in the authoritative `docs/repository-reorganization.md`.

## How to action (requires the user's explicit go-ahead)

Branch deletion/tagging was **not** performed automatically — it is outside the working branch's
push scope and needs an explicit go-ahead. Suggested commands once approved:

```bash
# archive the never-merged branch for provenance, then delete it
git tag archive/deterministic-kernel-vertical-slice origin/feat/deterministic-kernel-vertical-slice
git push origin archive/deterministic-kernel-vertical-slice
git push origin --delete feat/deterministic-kernel-vertical-slice

# delete the fully-merged branches
git push origin --delete feat/bootstrap-architecture
git push origin --delete feat/orderbook-rs-worker-kernel
```

Do not infer safety for unlisted branches — compare each with `main` first; delete only when it is
zero commits ahead or its unique work has been intentionally preserved.
</content>
