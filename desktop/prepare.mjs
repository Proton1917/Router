import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const desktop = fileURLToPath(new URL('./', import.meta.url));
if (process.platform !== 'darwin') throw new Error('当前桌面安装流程需要 macOS。');
execFileSync('cargo', ['build', '--release', '--locked', '--manifest-path', 'router-rs/Cargo.toml'], { cwd: root, stdio: 'inherit' });
execFileSync('npm', ['ci', '--prefix', 'web'], { cwd: root, stdio: 'inherit' });
execFileSync('npm', ['run', 'build', '--prefix', 'web'], { cwd: root, stdio: 'inherit' });
const resources = `${desktop}src-tauri/resources`;
rmSync(resources, { recursive: true, force: true });
mkdirSync(resources, { recursive: true });
cpSync(`${root}router-rs/target/release/router`, `${resources}/router`);
cpSync(`${root}web/dist`, `${resources}/web`, { recursive: true });
cpSync(`${desktop}appearance.json`, `${resources}/appearance.json`);
cpSync(`${root}router-rs/examples`, `${resources}/examples`, { recursive: true });
cpSync(`${root}scripts`, `${resources}/scripts`, { recursive: true, filter: path => !path.includes('/.work') && !path.includes('/__pycache__') });
execFileSync(`${desktop}node_modules/.bin/tauri`, ['icon', 'icon.svg', '--output', 'src-tauri/icons'], { cwd: desktop, stdio: 'inherit' });
