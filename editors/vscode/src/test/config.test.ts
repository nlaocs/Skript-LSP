import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { Configuration, resolveConfiguration } from '../config';

test('resolves literal paths relative to the explicit root and preserves addon order', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'skript-extension-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.writeFileSync(path.join(root, 'server & test'), '');
  fs.chmodSync(path.join(root, 'server & test'), 0o755);
  fs.mkdirSync(path.join(root, 'ssg data'));
  for (const file of ['b.wasm', 'a.wasm']) fs.writeFileSync(path.join(root, file), '');
  const config: Configuration = { serverPath: 'server & test', snapshotPath: 'ssg data', addonPaths: ['b.wasm', 'a.wasm'] };
  assert.deepEqual(resolveConfiguration(config, root), {
    serverPath: path.join(root, config.serverPath), snapshotPath: path.join(root, config.snapshotPath),
    addonPaths: config.addonPaths.map(file => path.join(root, file)),
  });
  assert.throws(() => resolveConfiguration(config, '.'), /root must be absolute/);
  assert.throws(() => resolveConfiguration({ ...config, serverPath: '' }, root), /Set skriptLsp.serverPath/);
  assert.throws(() => resolveConfiguration({ ...config, serverPath: 'missing' }, root), /does not exist/);
  assert.throws(() => resolveConfiguration({ ...config, snapshotPath: config.serverPath }, root), /directory/);
  assert.throws(() => resolveConfiguration({ ...config, addonPaths: ['ssg data'] }, root), /file/);
  assert.throws(() => resolveConfiguration({ ...config, addonPaths: null as unknown as string[] }, root), /array/);
  assert.throws(() => resolveConfiguration({ ...config, addonPaths: [7 as unknown as string] }, root), /addonPaths\[0\]/);
});
