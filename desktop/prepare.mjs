import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildEnvironment } from './build-environment.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const desktop = fileURLToPath(new URL('./', import.meta.url));
if (process.platform !== 'darwin') throw new Error('当前桌面安装流程需要 macOS。');
if (!existsSync(`${root}bin/spt-core`) || !existsSync(`${root}bin/SPT-LICENSE`) || !existsSync(`${root}bin/spt-licenses`)) throw new Error('请先按 README 安装 SPT worker、LICENSE 与第三方许可材料到 bin/。');
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
  cpSync(`${root}${source}`, output, { recursive: true });
}
cpSync(`${root}web/dist`, `${resources}/web`, { recursive: true });
execFileSync(`${desktop}node_modules/.bin/tauri`, ['icon', 'icon.svg', '--output', 'src-tauri/icons'], { cwd: desktop, stdio: 'inherit' });
