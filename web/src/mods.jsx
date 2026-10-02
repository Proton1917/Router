import React, { useEffect, useState } from 'react';
import defaults from '../../router-rs/examples/management.json';

export function ModsWorkspace({ config, dirty, api, mutate, editing, ui }) {
  const { Button, Field, Modal } = ui;
  const mods = config.management.mods;
  const [selected, setSelected] = useState('');
  const [report, setReport] = useState(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState(null);
  const [label, setLabel] = useState('');
  useEffect(() => { editing(!!editor); return () => editing(false); }, [!!editor]);
  const profiles = mods?.profiles || {};
  const catalog = mods?.catalog || {};
  const active = profiles[selected] ? selected : Object.keys(profiles)[0] || '';
  async function inspect() {
    setBusy(true); setError('');
    try { setReport(await api('mods/action', { action: 'inspect' })); }
    catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  }
  useEffect(() => { if (mods && !dirty) inspect(); }, [!!mods, dirty]);
  function createProfile() {
    if (!label.trim()) { setError('请填写组合名称。'); return; }
    const id = `group-${crypto.randomUUID().slice(0, 8)}`;
    mutate(next => { next.management.mods.profiles[id] = { label: label.trim(), plugins: {} }; });
    setSelected(id); setLabel(''); setError('');
  }
  function choice(id, state) {
    mutate(next => {
      const plugins = next.management.mods.profiles[active].plugins;
      if (state === 'inherit') delete plugins[id];
      else plugins[id] = state === 'on';
    });
  }
  function removeProfile() {
    if (Object.values(config.management.commands).some(command => command.mod_profile === active)) {
      setError('这个组合仍绑定启动命令，请先在下方解除绑定。'); return;
    }
    mutate(next => { delete next.management.mods.profiles[active]; });
    setSelected('');
  }
  function saveEntry() {
    if (!editor.value.label.trim() || !editor.value.plugin_id.trim()) { setError('请填写名称和插件标识。'); return; }
    if (Object.entries(catalog).some(([key, value]) => key !== editor.id && value.plugin_id === editor.value.plugin_id)) { setError('插件标识已经存在。'); return; }
    const value = { ...editor.value };
    if (!value.path) delete value.path;
    mutate(next => { next.management.mods.catalog[editor.id] = value; });
    setEditor(null); setError('');
  }
  async function validate(id) {
    setBusy(true); setError('');
    try { const result = await api('mods/action', { action: 'validate', entry: id }); setReport(old => ({ ...old, message: result.message })); }
    catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  }
  if (!mods) return <div className="empty"><h3>配置 Mods 管理</h3><p>创建管理配置后，在“客户端与运行环境”中填写本机的程序和脚本路径。</p><Button primary onClick={() => mutate(next => { next.management.mods = structuredClone(defaults.mods); })}>创建 Mods 管理配置</Button></div>;
  return <div className="mods-workspace">
    <div className="mods-toolbar"><div><span className="eyebrow">CLIENT EXTENSIONS</span><p>{report?.version || (dirty ? '应用配置后读取客户端版本' : '正在读取客户端版本')}</p></div><Button disabled={busy || dirty} onClick={inspect}>读取已安装插件</Button><Button primary onClick={() => { setError(''); setEditor({ id: `mod-${crypto.randomUUID().slice(0, 8)}`, value: { label: '', plugin_id: '', description: '', path: '', builtin: false, availability: '' } }); }}>添加 Mod</Button></div>
    <section className="settings-card mods-profiles"><h3>Mod 组合</h3><div className="form-grid"><Field label="当前编辑的组合"><select value={active} onChange={event => setSelected(event.target.value)}><option value="" disabled>选择一个组合</option>{Object.entries(profiles).map(([id, profile]) => <option key={id} value={id}>{profile.label}</option>)}</select></Field><Field label="新组合名称"><input value={label} onChange={event => setLabel(event.target.value)} placeholder="输入你自己的组合名称"/></Field></div><div className="mods-actions"><Button onClick={createProfile}>创建组合</Button>{active && <Button danger onClick={removeProfile}>删除当前组合</Button>}</div>{active && <Field label="组合名称"><input value={profiles[active].label} onChange={event => mutate(next => { next.management.mods.profiles[active].label = event.target.value; })}/></Field>}</section>
    <div className="mods-grid">{Object.entries(catalog).map(([id, entry]) => {
      const value = profiles[active]?.plugins[id];
      return <article className="backend-card mod-card" key={id}><header><span className="subtle-badge">{entry.builtin ? '内置' : entry.path ? '本地目录' : '已安装插件'}</span><code>{entry.plugin_id}</code></header><h3>{entry.label}</h3><p className="template-description">{entry.description || '按组合控制此插件的加载状态。'}</p>{entry.availability && <p className="mod-availability">{entry.availability}</p>}<Field label={`${entry.label} · 组合状态`}><select disabled={!active} value={value === undefined ? 'inherit' : value ? 'on' : 'off'} onChange={event => choice(id, event.target.value)}><option value="inherit">继承 Claude Code 设置</option><option value="on">启用</option><option value="off">停用</option></select></Field><footer><Button onClick={() => setEditor({ id, value: structuredClone(entry) })}>编辑</Button><button disabled={busy || dirty} onClick={() => validate(id)}>校验</button>{!entry.builtin && <button onClick={() => { if (Object.values(profiles).some(profile => id in profile.plugins)) { setError('请先将各组合中的此项设为继承，再移除条目。'); return; } mutate(next => { delete next.management.mods.catalog[id]; }); }}>移除条目</button>}</footer></article>;
    })}</div>
    <section className="settings-card"><h3>启动命令使用的组合</h3><p className="muted">保存并应用后，下一次启动命令时读取。组合设置优先于客户端的用户与项目设置，组织策略继续生效。</p><div className="form-grid">{Object.entries(config.management.commands).filter(([, command]) => config.management.templates[command.template]?.supports_mods).map(([name, command]) => <Field label={`${name} 的 Mod 组合`} key={name}><select value={command.mod_profile || ''} onChange={event => mutate(next => { if (event.target.value) next.management.commands[name].mod_profile = event.target.value; else delete next.management.commands[name].mod_profile; })}><option value="">继承客户端设置</option>{Object.entries(profiles).map(([id, profile]) => <option key={id} value={id}>{profile.label}</option>)}</select></Field>)}</div></section>
    <p className="muted">Mods 随所属插件启停，该插件包含的其他组件也会一同启停。本地目录使用 Claude Code 的 --plugin-dir 加载，已有会话保持当前加载状态。</p>
    {report?.installed?.length > 0 && <details className="advanced"><summary>客户端已安装插件</summary><pre className="code-preview">{JSON.stringify(report.installed, null, 2)}</pre></details>}
    {report?.message && <pre className="mods-validation">{report.message}</pre>}
    <details className="advanced"><summary>客户端与运行环境</summary><div className="form-grid"><Field label="Claude Code 程序"><input value={mods.settings.program} onChange={event => mutate(next => { next.management.mods.settings.program = event.target.value; })}/></Field><Field label="Python 程序" hint="此 Python 环境需要 packaging。"><input value={mods.adapter.program} onChange={event => mutate(next => { next.management.mods.adapter.program = event.target.value; })}/></Field><Field label="Mods 适配脚本"><input value={mods.adapter.args[0] || ''} onChange={event => mutate(next => { next.management.mods.adapter.args[0] = event.target.value; })}/></Field><Field label="临时设置目录"><input value={mods.settings.launch_directory} onChange={event => mutate(next => { next.management.mods.settings.launch_directory = event.target.value; })}/></Field></div></details>
    {error && <div className="integration-error" role="alert">{error}</div>}
    {editor && <Modal title="Mod 设置" close={() => setEditor(null)} footer={<><Button onClick={() => setEditor(null)}>取消</Button><Button primary onClick={saveEntry}>保存草稿</Button></>}><div className="form-grid">{[['label','显示名称'],['plugin_id','插件标识'],['description','说明'],['availability','使用条件']].map(([key,title]) => <Field key={key} label={title}><input value={editor.value[key] || ''} readOnly={editor.value.builtin && key === 'plugin_id'} onChange={event => setEditor(old => ({...old,value:{...old.value,[key]:event.target.value}}))}/></Field>)}{!editor.value.builtin && <Field label="本地插件目录或 ZIP 路径" wide hint="使用绝对路径或 ~；已安装插件留空。本地插件标识为清单 name 加 @inline。"><input value={editor.value.path || ''} onChange={event => setEditor(old => ({...old,value:{...old.value,path:event.target.value || undefined}}))}/></Field>}</div>{error && <div className="integration-error" role="alert">{error}</div>}</Modal>}
  </div>;
}
