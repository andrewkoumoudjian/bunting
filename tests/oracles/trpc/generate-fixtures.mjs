import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { initTRPC, tracked, TRPCError } from '@trpc/server';
import { fetchRequestHandler } from '@trpc/server/adapters/fetch';

const VERSION = '11.18.0';
const COMMIT = '6aec1578a899df50a17e4e78d5512a099b574c18';
const here = dirname(fileURLToPath(import.meta.url));
const fixtureDir = resolve(here, '../../fixtures/reference/trpc/11.18.0');
const t = initTRPC.create();
const router = t.router({
  'system.health': t.procedure.query(() => ({ apiVersion: 'bunting.v1', contractCompatible: true })),
  'echo.query': t.procedure.query(({ input }) => input ?? null),
  'orders.submit': t.procedure.mutation(({ input }) => ({ accepted: true, input })),
  'error.invalid': t.procedure.query(() => { throw new TRPCError({ code: 'BAD_REQUEST', message: 'invalid fixture input' }); }),
  'market.subscribe': t.procedure.subscription(async function* () {
    yield tracked('41', { sequence: '41', kind: 'snapshot' });
  }),
});

function normalizedHeaders(headers) {
  return Object.fromEntries([...headers.entries()].sort(([a], [b]) => a.localeCompare(b)));
}
async function invoke(name, { method = 'GET', path, query = '', body, headers = {} }) {
  const requestHeaders = new Headers(headers);
  if (body !== undefined) requestHeaders.set('content-type', 'application/json');
  const req = new Request(`https://oracle.invalid/trpc/${path}${query}`, {
    method, headers: requestHeaders, body: body === undefined ? undefined : JSON.stringify(body),
  });
  const response = await fetchRequestHandler({ endpoint: '/trpc', req, router, maxBatchSize: 16 });
  const text = await response.text();
  let normalizedBody = text;
  if ((response.headers.get('content-type') ?? '').startsWith('application/json')) normalizedBody = JSON.parse(text);
  return {
    fixture_version: 1,
    oracle: { package_version: VERSION, source_commit: COMMIT, license: 'MIT' },
    case: name,
    request: { method, path: `/trpc/${path}`, query, headers: normalizedHeaders(requestHeaders), body: body ?? null },
    response: { status: response.status, headers: normalizedHeaders(response.headers), body: normalizedBody },
  };
}

const cases = [
  await invoke('single_query', { path: 'system.health' }),
  await invoke('query_with_encoded_input', { path: 'echo.query', query: '?input=%7B%22runId%22%3A%229007199254740993%22%7D' }),
  await invoke('single_mutation', { method: 'POST', path: 'orders.submit', body: { orderId: '9007199254740993' } }),
  await invoke('bounded_query_batch', { path: 'system.health,echo.query', query: '?batch=1&input=%7B%221%22%3A%7B%22instrumentId%22%3A%227%22%7D%7D' }),
  await invoke('official_mutation_batch_reference_only', { method: 'POST', path: 'orders.submit,orders.submit', query: '?batch=1', body: { 0: { orderId: '1' }, 1: { orderId: '2' } } }),
  await invoke('structured_bad_request', { path: 'error.invalid' }),
  await invoke('unknown_procedure', { path: 'missing.procedure' }),
  await invoke('method_not_supported', { method: 'POST', path: 'system.health', body: null }),
  await invoke('http_subscription_sse', { path: 'market.subscribe', headers: { accept: 'text/event-stream' } }),
];

await mkdir(fixtureDir, { recursive: true });
const manifest = { fixture_version: 1, oracle: { package_version: VERSION, source_commit: COMMIT, license: 'MIT' }, fixtures: [] };
for (const fixture of cases) {
  const data = `${JSON.stringify(fixture, null, 2)}\n`;
  const file = `${fixture.case}.json`;
  await writeFile(resolve(fixtureDir, file), data);
  manifest.fixtures.push({ file, sha256: createHash('sha256').update(data).digest('hex') });
}
await writeFile(resolve(fixtureDir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
