import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildEnvironment } from './build-environment.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const desktop = fileURLToPath(new URL('./', import.meta.url));
if (process.platform !== 'darwin') throw new Error('当前桌面安装流程需要 macOS。');
const noticeLock = JSON.parse(readFileSync(`${desktop}notices-lock.json`, 'utf8'));
if (noticeLock.algorithm !== 'blake2b512') throw new Error('许可声明记录格式无效，请按 README 重新生成。');
for (const [path, expected] of Object.entries(noticeLock.files)) {
  const actual = createHash(noticeLock.algorithm).update(readFileSync(root + path)).digest('hex');
  if (actual !== expected) throw new Error(`依赖锁文件已变化，请按 README 重新生成许可声明：${path}`);
}
execFileSync('cargo', ['build', '--release', '--locked', '--manifest-path', 'router-rs/Cargo.toml'], { cwd: root, env: buildEnvironment(), stdio: 'inherit' });
execFileSync('npm', ['ci', '--prefix', 'web'], { cwd: root, stdio: 'inherit' });
execFileSync('npm', ['run', 'build', '--prefix', 'web'], { cwd: root, stdio: 'inherit' });
const resources = `${desktop}src-tauri/resources`;
rmSync(resources, { recursive: true, force: true });
mkdirSync(resources, { recursive: true });
const entries = JSON.parse(readFileSync(`${desktop}resources.json`, 'utf8'));
for (const [source, destination] of Object.entries(entries)) {
  const output = `${resources}/${destination}`;
  mkdirSync(dirname(output), { recursive: true });
  cpSync(`${root}${source}`, output);
}
cpSync(`${root}web/dist`, `${resources}/web`, { recursive: true });
execFileSync(`${desktop}node_modules/.bin/tauri`, ['icon', 'icon.svg', '--output', 'src-tauri/icons'], { cwd: desktop, stdio: 'inherit' });
