using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "bunting-worker", worker = .buntingWorker),
    (name = "d1-stub", worker = .d1Stub),
  ],
  sockets = [
    (name = "http", address = "127.0.0.1:8787", http = (), service = "bunting-worker"),
  ],
);

const buntingWorker :Workerd.Worker = (
  modules = [
    (name = "shim.mjs", esModule = embed "../build/worker/shim.mjs"),
    (name = "index.wasm", wasm = embed "../build/worker/index.wasm"),
  ],
  compatibilityDate = "2026-07-12",
  bindings = [
    (
      name = "ORIGIN_DB",
      wrapped = (
        moduleName = "cloudflare-internal:d1-api",
        innerBindings = [(name = "fetcher", service = "d1-stub")],
      ),
    ),
    (name = "FIX_SESSIONS", durableObjectNamespace = "FixSessionObject"),
    (name = "BUNTING_FIX_DESTINATIONS", text = "127.0.0.1:9877"),
    (name = "BUNTING_API_TOKEN", text = "workerd-smoke-token"),
    (name = "BUNTING_API_PARTICIPANT_ID", text = "1"),
  ],
  globalOutbound = "d1-stub",
  cacheApiOutbound = "d1-stub",
  durableObjectNamespaces = [
    (
      className = "FixSessionObject",
      uniqueKey = "bunting.workerd.fix-session.v1",
      enableSql = true,
    ),
  ],
  durableObjectStorage = (inMemory = void),
);

const d1Stub :Workerd.Worker = (
  modules = [
    (name = "d1-stub.mjs", esModule = embed "d1-stub.mjs"),
  ],
  compatibilityDate = "2026-07-12",
  globalOutbound = "d1-stub",
);
