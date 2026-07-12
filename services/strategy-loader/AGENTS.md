# strategy-loader instructions

This minimal TypeScript service owns only Cloudflare Worker Loader calls and Tail Worker attachment. It does not own market state, strategy state, scheduling, risk or order submission.

Use `LOADER.get()` with an immutable ID derived from source, wrapper, SDK, compatibility and limits hashes. The callback must return byte-identical WorkerCode for the same ID. Never assume isolate reuse or rely on Python globals.

Always set `globalOutbound: null`, the pinned compatibility date and `python_workers` flag. Provide no direct storage, Durable Object, Queue, secret, credential or order capability. Apply versioned CPU/subrequest limits and enforce bounded modules, request, state, output, action and log sizes.

Invoke only the fixed wrapper contract and return a typed bounded result to the trusted dispatch consumer. Attach a Tail Worker for correlated operational logs; Tail output is not canonical market state.