import { homedir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export function buildEnvironment() {
  const root = fileURLToPath(new URL('../', import.meta.url)).replace(/\/$/, '');
  const inherited = process.env.CARGO_ENCODED_RUSTFLAGS !== undefined
    ? process.env.CARGO_ENCODED_RUSTFLAGS.split('\x1f')
    : (process.env.RUSTFLAGS || '').split(/\s+/);
  const mappings = [
    [homedir(), '/build-home'],
    [process.env.CARGO_HOME || join(homedir(), '.cargo'), '/cargo'],
    [process.env.RUSTUP_HOME || join(homedir(), '.rustup'), '/rustup'],
    [root, '/router'],
  ];
  const flags = inherited.filter(Boolean);
  for (const [from, to] of mappings) {
    const flag = `--remap-path-prefix=${from}=${to}`;
    if (!flags.includes(flag)) flags.push(flag);
  }
  return { ...process.env, CARGO_ENCODED_RUSTFLAGS: flags.join('\x1f') };
}
