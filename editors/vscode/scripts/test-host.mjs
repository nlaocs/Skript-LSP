import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

// Use an already installed Stable or Insiders executable. Never download Code,
// the server, or snapshot data, and never use an existing user profile.
const [code, server, snapshot] = process.argv.slice(2);
if (!code || !server || !snapshot) throw new Error('Usage: node scripts/test-host.mjs <Code executable> <skript-lsp executable> <SSG directory>');
for (const target of [code, server, snapshot]) if (!path.isAbsolute(target) || !fs.existsSync(target)) throw new Error(`Expected existing absolute path: ${target}`);
const extension = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const base = fs.mkdtempSync(path.join(os.tmpdir(), 'skript-vscode-'));
const folders = Object.fromEntries(['a', 'b', 'outer', 'outer/nested', 'cap'].map(name => [name, path.join(base, 'projects', name)]));
for (const folder of Object.values(folders)) fs.mkdirSync(folder, { recursive: true });
const loose = path.join(base, 'loose');
fs.mkdirSync(path.join(loose, 'child'), { recursive: true });
const workspaceFile = path.join(base, 'test.code-workspace');
const settings = { 'skriptLsp.serverPath': server, 'skriptLsp.snapshotPath': snapshot, 'skriptLsp.trace.server': 'verbose',
  'workbench.startupEditor': 'none', 'extensions.autoCheckUpdates': false, 'extensions.autoUpdate': false };
fs.writeFileSync(workspaceFile, JSON.stringify({ folders: Object.entries(folders).map(([name, folder]) => ({ name, path: folder })), settings }, null, 2));
const userData = path.join(base, 'profile');
fs.mkdirSync(path.join(userData, 'User'), { recursive: true });
// Loose files use the isolated profile's settings, not a guessed project file.
fs.writeFileSync(path.join(userData, 'User', 'settings.json'), JSON.stringify(settings, null, 2));
fs.writeFileSync(path.join(folders.a, 'library.sk'), 'function project_greet(value: text) :: text:\n    return {_value}\n');
fs.writeFileSync(path.join(folders.b, 'library.sk'), 'function other_project() :: text:\n    return "other"\n');
// Exhaust the existing server scan limit using only disposable, non-Skript data.
for (let index = 0; index < 50001; index++) fs.writeFileSync(path.join(folders.cap, `entry-${index}.txt`), '');
const config = path.join(base, 'test.json');
fs.writeFileSync(config, JSON.stringify({ base, folders, loose, workspaceFile }));
const env = { ...process.env, SKRIPT_LSP_TEST_CONFIG: config };
delete env.ELECTRON_RUN_AS_NODE;
delete env.VSCODE_IPC_HOOK_CLI;
console.log(`Isolated test artifacts: ${base}`);
const child = spawn(code, [workspaceFile, '--new-window', '--user-data-dir', userData, '--extensions-dir', path.join(base, 'extensions'),
  '--skip-welcome', '--skip-release-notes', `--extensionDevelopmentPath=${extension}`, `--extensionTestsPath=${path.join(extension, 'out/test/host.js')}`],
{ env, stdio: 'inherit', shell: false });
child.on('error', error => { console.error(error); process.exitCode = 1; });
child.on('exit', code => {
  console.log(`Test result: ${path.join(base, 'result.json')}`);
  process.exitCode = code ?? 1;
  const result = path.join(base, 'result.json');
  if (!fs.existsSync(result) || JSON.parse(fs.readFileSync(result, 'utf8')).status !== 'passed') process.exitCode = 1;
});
