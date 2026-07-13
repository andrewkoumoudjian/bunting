# tRPC 11.18.0 conformance oracle

This development-only harness records the HTTP behavior selected by
`schemas/trpc/bunting.v1.json`. It uses the official MIT-licensed tRPC packages
at version `11.18.0`, whose source tag resolves to
`6aec1578a899df50a17e4e78d5512a099b574c18`.

Refresh and validate from the repository root:

```bash
npm --prefix tests/oracles/trpc ci
npm --prefix tests/oracles/trpc run fixtures:refresh
npm --prefix tests/oracles/trpc run validate
```

The generator normalizes headers, JSON bodies, and SSE frames before writing
fixtures. Production manifests do not reference this directory, and CI can run
the validator against committed fixtures without installing Node dependencies.
