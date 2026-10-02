import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const inputs = process.argv.slice(2);
if (inputs.length !== 2) throw new Error('请传入 router 与桌面项目的 cargo-about JSON 报告。');
const sections = new Map();
function add(id, content, names) {
  if (!content || !id || !names.length) throw new Error('许可声明内容不完整。');
  const key = id + '\0' + content;
  const existing = sections.get(key) || { id, content, names: new Set() };
  for (const name of names) existing.names.add(name);
  sections.set(key, existing);
}
for (const input of inputs) {
  const report = JSON.parse(readFileSync(input, 'utf8'));
  for (const license of report.licenses) {
    const names = license.used_by.map(item => item.crate)
      .filter(crate => !['router', 'router-desktop'].includes(crate.name))
      .map(crate => `${crate.name} ${crate.version}`);
    if (names.length) add(license.id, license.text, names);
  }
}
const graph = JSON.parse(execFileSync('npm', ['ls', '--prefix', 'web', '--omit=dev', '--all', '--json'], { cwd: root, encoding: 'utf8' }));
const visited = new Set();
function npmLicenses(dependencies) {
  for (const [name, dependency] of Object.entries(dependencies || {})) {
    const identity = `${name} ${dependency.version}`;
    if (visited.has(identity)) continue;
    visited.add(identity);
    const directory = `${root}web/node_modules/${name}`;
    const metadata = JSON.parse(readFileSync(`${directory}/package.json`, 'utf8'));
    if (metadata.version !== dependency.version) throw new Error(`依赖版本不匹配：${name}`);
    const files = readdirSync(directory).filter(file => /^(licen[sc]e|notice)([.-].*)?$/i.test(file));
    if (!files.length) throw new Error(`依赖缺少许可文件：${name}`);
    for (const file of files) add(metadata.license, readFileSync(`${directory}/${file}`, 'utf8'), [identity]);
    npmLicenses(dependency.dependencies);
  }
}
npmLicenses(graph.dependencies);
const ordered = [...sections.values()].map(section => ({ ...section, names: [...section.names].sort() }))
  .sort((a, b) => a.id.localeCompare(b.id) || a.names.join().localeCompare(b.names.join()));
const output = ['Router — Third-Party Notices', '', 'This distribution includes software under the following licenses.', 'The Router project license is provided in LICENSE.', ''];
for (const section of ordered) output.push(section.id, section.names.join('\n'), '', section.content.trim(), '');
writeFileSync(`${root}desktop/THIRD_PARTY_NOTICES.txt`, output.join('\n') + '\n');
const locks = ['router-rs/Cargo.lock', 'desktop/src-tauri/Cargo.lock', 'web/package-lock.json'];
const files = Object.fromEntries(locks.map(path => [path, createHash('blake2b512').update(readFileSync(root + path)).digest('hex')]));
writeFileSync(`${root}desktop/notices-lock.json`, JSON.stringify({ algorithm: 'blake2b512', files }, null, 2) + '\n');
console.log(`已生成 ${ordered.length} 组许可声明，包含 ${visited.size} 个网页运行依赖。`);
