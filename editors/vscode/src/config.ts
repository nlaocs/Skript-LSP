import * as fs from 'node:fs';
import * as path from 'node:path';

export interface Configuration {
  serverPath: string;
  snapshotPath: string;
  addonPaths: string[];
}

export function resolveConfiguration(raw: Configuration, root: string): Configuration {
  if (!path.isAbsolute(root)) throw new Error('The project root must be absolute.');
  const resolve = (value: unknown, label: string, directory: boolean): string => {
    if (typeof value !== 'string' || value.length === 0) throw new Error(`Set skriptLsp.${label}.`);
    const absolute = path.resolve(root, value);
    let stat: fs.Stats;
    try { stat = fs.statSync(absolute); }
    catch { throw new Error(`skriptLsp.${label} does not exist: ${absolute}`); }
    if (directory ? !stat.isDirectory() : !stat.isFile()) {
      throw new Error(`skriptLsp.${label} must be a ${directory ? 'directory' : 'file'}: ${absolute}`);
    }
    return absolute;
  };
  if (!Array.isArray(raw.addonPaths)) throw new Error('skriptLsp.addonPaths must be an array.');
  const serverPath = resolve(raw.serverPath, 'serverPath', false);
  fs.accessSync(serverPath, fs.constants.X_OK);
  return {
    serverPath,
    snapshotPath: resolve(raw.snapshotPath, 'snapshotPath', true),
    addonPaths: raw.addonPaths.map((value, index) => resolve(value, `addonPaths[${index}]`, false)),
  };
}
