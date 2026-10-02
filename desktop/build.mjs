import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { buildEnvironment } from './build-environment.mjs';

const desktop = fileURLToPath(new URL('./', import.meta.url));
execFileSync(`${desktop}node_modules/.bin/tauri`, ['build', '--bundles', 'app', ...process.argv.slice(2), '--', '--locked'], {
  cwd: desktop,
  env: buildEnvironment(),
  stdio: 'inherit',
});
execFileSync('pixi', ['run', '--manifest-path', `${desktop}pixi.toml`, 'installer'], {
  cwd: desktop,
  env: process.env,
  stdio: 'inherit',
});
