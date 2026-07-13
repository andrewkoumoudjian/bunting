import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../..');
const schema = JSON.parse(await readFile(resolve(root, 'schemas/trpc/bunting.v1.json')));
const fixtureDir = resolve(root, 'tests/fixtures/reference/trpc/11.18.0');
const manifest = JSON.parse(await readFile(resolve(fixtureDir, 'manifest.json')));
const expectedCommit = '6aec1578a899df50a17e4e78d5512a099b574c18';
if (schema.wire_contract.oracle.source_git_head !== expectedCommit || schema.wire_contract.oracle.version !== '11.18.0') throw new Error('contract oracle pin mismatch');
if (schema.wire_contract.batching.max_query_calls !== 16 || schema.wire_contract.batching.mutation_batching !== 'rejected') throw new Error('batch contract mismatch');
if (!schema.wire_contract.unsupported_features.includes('websocket_transport')) throw new Error('unsupported features incomplete');
for (const entry of manifest.fixtures) {
  const data = await readFile(resolve(fixtureDir, entry.file));
  const actual = createHash('sha256').update(data).digest('hex');
  if (actual !== entry.sha256) throw new Error(`fixture hash mismatch: ${entry.file}`);
  const fixture = JSON.parse(data);
  if (fixture.oracle.source_commit !== expectedCommit || fixture.oracle.package_version !== '11.18.0') throw new Error(`fixture provenance mismatch: ${entry.file}`);
}
const files = (await readdir(fixtureDir)).filter((file) => file.endsWith('.json') && file !== 'manifest.json');
if (files.length !== manifest.fixtures.length) throw new Error('unmanifested fixture');
console.log(`validated contract and ${files.length} pinned tRPC fixtures`);
