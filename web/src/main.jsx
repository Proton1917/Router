import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { fetchEventSource } from '@microsoft/fetch-event-source';
import './style.css';
import './desktop.css';
import { IntegrationWorkspace, IntegrationEditor } from './integrations.jsx';
import { ModsWorkspace } from './mods.jsx';
import './mods.css';

const copy = value => structuredClone(value);
const pretty = value => JSON.stringify(value, null, 2);
const clean = value => Object.fromEntries(Object.entries(value).filter(([,v]) => v !== undefined));
const launchParameters = new URLSearchParams(location.hash.slice(1));
if (launchParameters.get('desktop') === '1') sessionStorage.setItem('router-desktop', '1');
const desktop = sessionStorage.getItem('router-desktop') === '1';
if (desktop) document.documentElement.dataset.desktop = 'true';
const tokenFromUrl = launchParameters.get('token');
if (tokenFromUrl) { sessionStorage.setItem('router-token', tokenFromUrl); history.replaceState(null, '', location.pathname); }

async function api(path, body) {
  const response = await fetch(`/api/${path}`, { method: body === undefined ? 'GET' : 'POST', headers: { Authorization: `Bearer ${sessionStorage.getItem('router-token') || ''}`, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) }, body: body === undefined ? undefined : JSON.stringify(body) });
  if (response.status === 401) throw Object.assign(new Error('请运行 router web，从授权入口打开管理界面。'), { status: response.status });
  const text = await response.text();
  const data = text ? JSON.parse(text) : {};
  if (!response.ok) throw Object.assign(new Error(data.error || `请求失败（HTTP ${response.status}）`), { status: response.status });
  return data;
}

const sections = [
  ['commands', '启动命令', '01'], ['backends', 'API 后端', '02'], ['models', '模型配置', '03'],
  ['routes', '路由规则', '04'], ['templates', '启动方式', '05'], ['settings', '服务设置', '06'],
  ['integrations', '客户端接入', '07'],
  ['mods', 'Mods 管理', '08'],
];
const descriptions = {
  commands: '为不同客户端和工作方式设置独立入口。名称、模型和启动参数由你决定。',
  backends: '统一管理 API 地址和凭据来源。密钥保存到独立的受保护文件。',
  models: '选择后端、API 模型和供应商，并分别设置标准与 Fast 目标。',
  routes: '按从上到下的顺序匹配请求。命令绑定的规则由相应命令配置管理。',
  templates: '启动程序、参数和环境变量的可复用配置。命令可以在此基础上单独调整。',
  settings: '管理转发服务的网络与资源配置。服务参数修改后需要重启。',
  integrations: '选择客户端，配置它使用的 API 服务与模型，并同步接入设置。',
  mods: '管理客户端扩展，将独立的 Mod 组合绑定到启动命令。',
};

function Icon({ name = 'grid', size = 18 }) {
  const paths = {
    grid: <><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></>,
    plus: <path d="M12 5v14M5 12h14"/>, arrow: <path d="m9 5 7 7-7 7"/>, close: <path d="m6 6 12 12M18 6 6 18"/>,
    search: <><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4.5 4.5"/></>, terminal: <><path d="m5 6 5 6-5 6M13 18h6"/></>,
    refresh: <><path d="M20 11a8 8 0 1 0-2 6M20 4v7h-7"/></>, check: <path d="m5 12 4 4L19 6"/>,
    api: <><rect x="3" y="4" width="18" height="6" rx="2"/><rect x="3" y="14" width="18" height="6" rx="2"/><path d="M7 7h.01M7 17h.01M11 7h6M11 17h6"/></>,
  };
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name] || paths.grid}</svg>;
}

function Button({ children, primary, danger, className = '', ...props }) { return <button className={`button ${primary ? 'primary' : ''} ${danger ? 'danger' : ''} ${className}`} {...props}>{children}</button>; }
function Field({ label, hint, children, wide }) {
  const id = React.useId();
  const controls = React.Children.map(children, child => React.isValidElement(child) && ['input','select','textarea'].includes(child.type) ? React.cloneElement(child, { id, 'aria-label': label, 'aria-describedby': hint ? `${id}-hint` : undefined }) : child);
  return <label className={`field ${wide ? 'wide' : ''}`} htmlFor={id}><span className="field-label">{label}</span>{controls}{hint && <small id={`${id}-hint`}>{hint}</small>}</label>;
}
function Empty({ text, onAdd }) { return <div className="empty"><Icon size={34}/><h3>还没有配置内容</h3><p>{text}</p>{onAdd && <Button onClick={onAdd}><Icon name="plus"/>添加</Button>}</div>; }
function Modal({ title, eyebrow, children, close, footer, wide }) {
  useEffect(() => { const key = e => { if (e.key === 'Escape') close(); }; window.addEventListener('keydown', key); return () => window.removeEventListener('keydown', key); }, [close]);
  return <div className="modal-backdrop" onMouseDown={e => e.target === e.currentTarget && close()}><section className={`modal ${wide ? 'modal-wide' : ''}`} role="dialog" aria-modal="true" aria-label={title}><header><div><span className="eyebrow">{eyebrow || '配置编辑'}</span><h2>{title}</h2></div><button className="icon-button" onClick={close} aria-label="关闭"><Icon name="close"/></button></header><div className="modal-body">{children}</div>{footer && <footer>{footer}</footer>}</section></div>;
}

function JsonField({ label, value, onChange, onError, hint, rows = 6 }) {
  const [text, setText] = useState(pretty(value));
  const [error, setError] = useState('');
  const lastValue = React.useRef(pretty(value));
  useEffect(() => { const current = pretty(value); if (current !== lastValue.current) { lastValue.current = current; setText(current); setError(''); onError?.(false); } }, [value, onError]);
  function edit(text) { setText(text); try { const value = JSON.parse(text); lastValue.current = pretty(value); onChange(value); setError(''); onError?.(false); } catch (e) { setError(e.message); onError?.(true); } }
  return <Field label={label} hint={hint} wide><textarea className="code-input" rows={rows} value={text} onChange={e => edit(e.target.value)}/>{error && <span className="inline-error">JSON 格式尚未完成</span>}</Field>;
}

function App() {
  const [snapshot, setSnapshot] = useState(null);
  const [draft, setDraft] = useState(null);
  const [status, setStatus] = useState(null);
  const [section, setSection] = useState('commands');
  const [query, setQuery] = useState('');
  const [editor, setEditor] = useState(null);
  const [integrationEditor, setIntegrationEditor] = useState(null);
  const [modsEditing, setModsEditing] = useState(false);
  const [preview, setPreview] = useState(null);
  const [details, setDetails] = useState(null);
  const [confirm, setConfirm] = useState(null);
  const [notice, setNotice] = useState(null);
  const [fatal, setFatal] = useState('');
  const [busy, setBusy] = useState(false);
  const [pendingConfig, setPendingConfig] = useState(null);
  const latest = useRef({});
  const restarting = useRef(false);
  const dirty = snapshot && draft && pretty(snapshot.config) !== pretty(draft);
  latest.current = { snapshot, dirty, busy, editing: !!(editor || integrationEditor || modsEditing || preview || confirm) };
  const notify = (message, error = false) => setNotice({ message, error });

  function acceptConfig(data) { setSnapshot(data); setDraft(copy(data.config)); setStatus(data.status); setFatal(''); setPendingConfig(null); }
  async function load() { const data = await api('state'); acceptConfig(data); }
  useEffect(() => { load().catch(e => setFatal(e.message)); }, []);
  useEffect(() => { if (!notice) return; const id = setTimeout(() => setNotice(null), 7000); return () => clearTimeout(id); }, [notice]);
  useEffect(() => {
    if (!snapshot) return;
    const controller = new AbortController();
    let shownError = false;
    let updates = Promise.resolve();
    fetchEventSource('/api/changes', {
      signal: controller.signal,
      openWhenHidden: true,
      headers: { Authorization: `Bearer ${sessionStorage.getItem('router-token') || ''}` },
      onopen(response) {
        if (!response.ok) throw Object.assign(new Error(`自动更新请求失败（HTTP ${response.status}）`), { status: response.status });
        if (!response.headers.get('content-type')?.startsWith('text/event-stream')) throw new Error('自动更新返回格式不正确。');
        shownError = false;
      },
      onmessage(event) {
        if (event.event !== 'configuration') return;
        updates = updates.then(async () => {
          const change = JSON.parse(event.data);
          if (change.error) throw new Error(change.error);
          if (change.revision === latest.current.snapshot?.revision || controller.signal.aborted) return;
          const data = await api('state');
          if (controller.signal.aborted) return;
          if (latest.current.dirty || latest.current.editing || latest.current.busy) setPendingConfig(data);
          else acceptConfig(data);
        }).catch(error => { if (!(restarting.current && error instanceof TypeError)) notify(error.message, true); });
      },
      onclose() { throw new Error('自动更新暂时中断。'); },
      onerror(error) {
        if (error.status === 401 || error.status === 403) throw error;
        if (!shownError && !restarting.current) { notify(`${error.message} 将重试自动更新。`, true); shownError = true; }
        return 1000;
      },
    }).catch(error => { if (!controller.signal.aborted) notify(error.message, true); });
    return () => controller.abort();
  }, [!!snapshot]);
  useEffect(() => {
    if (!pendingConfig) return;
    if (pendingConfig.revision === snapshot?.revision) setPendingConfig(null);
    else if (!dirty && !editor && !integrationEditor && !modsEditing && !preview && !confirm && !busy) acceptConfig(pendingConfig);
  }, [pendingConfig, snapshot, dirty, editor, integrationEditor, modsEditing, preview, confirm, busy]);

  async function restartService() {
    restarting.current = true;
    try {
      const operation = await api('service/restart', {});
      for (let attempt = 0; attempt < 50; attempt++) {
        await new Promise(resolve => setTimeout(resolve, 200));
        let data;
        try { data = await api('state'); }
        catch (error) { if (error instanceof TypeError || [502, 503].includes(error.status)) continue; throw error; }
        if (data.status.restart_error) throw new Error(data.status.restart_error);
        if (data.status.pid !== operation.pid) { setStatus(data.status); return; }
      }
      throw new Error('router 重启未完成，请检查配置的重启命令。');
    } finally { restarting.current = false; }
  }

  async function task(action) { setBusy(true); setNotice(null); try { return await action(); } catch (e) { notify(e.message, true); } finally { setBusy(false); } }
  function mutate(callback) { setDraft(old => { const next = copy(old); callback(next); return next; }); }
  function entries(kind = section) {
    if (!draft) return [];
    if (['integrations','mods'].includes(kind)) return [];
    const collection = kind === 'commands' ? draft.management.commands : kind === 'templates' ? draft.management.templates : kind === 'backends' ? draft.runtime.backends : kind === 'models' ? draft.profiles : draft.routes;
    return Array.isArray(collection) ? collection.map(value => [value.id, value]) : Object.entries(collection || {});
  }
  const rows = useMemo(() => entries().filter(([id, value]) => `${id} ${JSON.stringify(value)}`.toLowerCase().includes(query.toLowerCase())), [draft, section, query]);
  const label = id => draft?.management.labels?.[`profile:${id}`] || id;
  const startupModel = command => {
    if (command.model) return command.model;
    if (!command.profile) return '跟随客户端选择';
    const protocol = draft.runtime.server.endpoints[draft.management.templates[command.template]?.request_path];
    return draft.management.profile_models?.[command.profile]?.[protocol] || draft.profiles[command.profile]?.standard.model || '由模型配置决定';
  };

  function add() {
    const initial = section === 'commands' ? { template: Object.keys(draft.management.templates)[0] || '', enabled: true, model: null, profile: null, args: [], env: {}, unset_env: [] }
      : section === 'backends' ? { base_url: '', auth: { headers: { authorization: 'Bearer {token}' }, token_file: '' } }
      : section === 'models' ? { standard: { target: Object.keys(draft.runtime.backends)[0] || '', model: '' } }
      : section === 'routes' ? { id: '', match: { models: [] }, profile: Object.keys(draft.profiles)[0] || '' }
      : { label: '', program: '', args: [], env: {}, unset_env: [], model_flags: [], request_headers: {} };
    setEditor({ kind: section, id: '', value: initial, fresh: true });
  }

  function saveRecord(kind, oldId, id, value, extras) {
    mutate(config => {
      const collection = kind === 'commands' ? config.management.commands : kind === 'templates' ? config.management.templates : kind === 'backends' ? config.runtime.backends : kind === 'models' ? config.profiles : config.routes;
      if (Array.isArray(collection)) { const index = collection.findIndex(row => row.id === oldId); if (index < 0) collection.splice(Math.max(0, collection.length - 1), 0, { ...value, id }); else collection[index] = { ...value, id }; }
      else { if (id !== oldId && collection[id]) throw new Error('此名称已存在'); if (oldId && id !== oldId) delete collection[oldId]; collection[id] = value; }
      if (kind === 'commands' && oldId && oldId !== id) Object.values(config.management.integrations || {}).forEach(client => { client.commands = client.commands.map(name => name === oldId ? id : name); });
      if (kind === 'models') {
        config.management.labels ||= {};
        config.management.labels[`profile:${id}`] = extras.label || id;
        config.management.profile_models ||= {};
        config.management.profile_models[id] = extras.profileModels;
        if (extras.bindCommand) { const command = config.management.commands[extras.bindCommand]; command.profile = id; command.model ||= extras.alias || value.standard.model; }
        if (extras.alias) config.routes.splice(Math.max(0, config.routes.length - 1), 0, { id: `model-${id}`, match: { models: [extras.alias] }, profile: id });
      }
      if (kind === 'backends') { config.management.backend_protocols ||= {}; config.management.backend_protocols[id] = extras.protocols; }
    });
    setEditor(null);
    notify('已更新草稿，点击“应用修改”后生效。');
  }

  function remove(kind, id) {
    setConfirm({ title: `删除 ${id}`, message: '删除会先写入草稿。应用时将检查命令、模型和路由之间的引用关系。', action: () => { mutate(config => { const collection = kind === 'commands' ? config.management.commands : kind === 'templates' ? config.management.templates : kind === 'backends' ? config.runtime.backends : kind === 'models' ? config.profiles : config.routes; if (Array.isArray(collection)) collection.splice(collection.findIndex(row => row.id === id), 1); else delete collection[id]; if (kind === 'commands') Object.values(config.management.integrations || {}).forEach(client => { client.commands = client.commands.filter(name => name !== id); }); }); setConfirm(null); } });
  }

  async function prepareApply() { await task(async () => { const data = await api('preview', { revision: snapshot.revision, config: draft }); setPreview(data); }); }
  async function apply() {
    await task(async () => {
      const serverChanged = pretty(snapshot.config.runtime.server) !== pretty(preview.config.runtime.server);
      const commandsChanged = pretty(snapshot.config.management.commands) !== pretty(preview.config.management.commands);
      const data = await api('apply', { revision: snapshot.revision, config: preview.config });
      setSnapshot({ ...snapshot, config: data.config, revision: data.revision }); setDraft(copy(data.config)); setPreview(null);
      if (commandsChanged) await api('commands/install', {});
      if (serverChanged) await restartService();
      for (const [id, integration] of Object.entries(data.config.management.integrations || {})) {
        if (pretty(snapshot.config.management.integrations?.[id]) === pretty(integration)) continue;
        try { await api('integrations/action', { id, action: 'sync', revision: data.revision }); }
        catch (error) { throw new Error(`路由配置已保存；${integration.label} 接入同步失败：${error.message}`); }
      }
      notify(serverChanged ? '配置已应用，router 已重启。' : '配置已应用。后续请求使用当前配置。');
      await load();
    });
  }

  function editSettings() { setEditor({ kind: 'settings', id: 'server', value: copy(draft.runtime.server) }); }
  function moveRoute(id, delta) { mutate(config => { const index = config.routes.findIndex(row => row.id === id); const target = index + delta; if (target < 0 || target >= config.routes.length) return; const [route] = config.routes.splice(index, 1); config.routes.splice(target, 0, route); }); }

  if (fatal || !draft) return <div className="loading"><div className="wordmark">router<span>.</span></div>{fatal && <><h2>无法读取配置</h2><p>{fatal}</p><Button onClick={() => load().catch(error => setFatal(error.message))}>重试读取</Button></>}</div>;
  const active = sections.find(([id]) => id === section);
  const metrics = [['启动命令', Object.keys(draft.management.commands).length], ['API 后端', Object.keys(draft.runtime.backends).length], ['模型配置', Object.keys(draft.profiles).length], ['路由规则', draft.routes.length]];

  return <div className="application">
    {desktop && <div className="desktop-titlebar" data-tauri-drag-region><span>Router</span></div>}
    <aside className="sidebar"><div className="wordmark">router<span>.</span></div><div className="workspace-tag"><span className="status-dot"/>本机工作空间</div><nav>{sections.map(([id, title, number]) => <button className={section === id ? 'active' : ''} key={id} onClick={() => { setSection(id); setQuery(''); }}><span className="nav-number">{number}</span>{title}<Icon name="arrow" size={14}/></button>)}</nav><div className="sidebar-footer"><div className="terminal-mark"><Icon name="terminal"/><code>{desktop ? 'Router Desktop' : 'router web'}</code></div><p>配置保存在本机<br/>命令与路由由你定义</p><span>v{status?.version}</span></div></aside>
    <main>
      {pendingConfig && pendingConfig.revision !== snapshot.revision && <div className="config-update" role="status">配置有更新。当前编辑已保留，完成编辑后可刷新读取。</div>}
      <header className="topbar"><div className="breadcrumb">工作空间 <span>/</span> {active[1]}</div><div className="topbar-right"><span className="workspace-label">本机配置</span><button className="icon-button" aria-label="刷新配置" onClick={() => dirty ? setConfirm({ title: '重新读取配置', message: '当前草稿尚未应用。重新读取会放弃这些草稿修改。', action: () => task(async () => { await load(); setConfirm(null); }) }) : task(load)}><Icon name="refresh"/></button></div></header>
      <div className="page-content"><div className="page-heading"><div><div className="eyebrow">LOCAL ROUTING / {active[2]}</div><h1>{active[1]}</h1><p>{descriptions[section]}</p></div><div className="heading-actions">{section === 'commands' && <Button disabled={busy} onClick={() => task(async () => { const result = await api('commands/install', {}); notify(`已同步 ${result.installed.length} 个终端命令。`); })}><Icon name="terminal"/>同步终端入口</Button>}{section === 'settings' ? <Button primary onClick={editSettings}>编辑服务参数</Button> : !['integrations','mods'].includes(section) && <Button primary onClick={add}><Icon name="plus"/>新增{active[1].replace('启动','').replace('API ','').replace('配置','')}</Button>}</div></div>
        {section === 'integrations' && <IntegrationWorkspace config={draft} revision={snapshot.revision} dirty={dirty} api={api} ui={{Button,Field,Modal}} onEdit={setIntegrationEditor} onCommand={name => setEditor({kind:'commands',id:name,value:copy(draft.management.commands[name])})}/>}
        {section === 'mods' && <ModsWorkspace config={draft} dirty={dirty} api={api} mutate={mutate} editing={setModsEditing} ui={{Button,Field,Modal}}/>}
        {!['integrations','mods'].includes(section) && <div className="summary-strip">{metrics.map(([title, count]) => <div key={title}><strong>{count.toString().padStart(2, '0')}</strong><span>{title}</span></div>)}<div className="summary-note"><span className="mini-label">配置状态</span><span className={dirty ? 'text-amber' : 'text-green'}>{dirty ? '有未应用的修改' : '与本机文件一致'}</span></div></div>}
        {!['settings','integrations','mods'].includes(section) && <div className="table-toolbar"><label className="search"><Icon name="search"/><input aria-label="搜索配置" value={query} onChange={e => setQuery(e.target.value)} placeholder={`搜索${active[1]}…`}/><kbd>⌕</kbd></label><span>{rows.length} 项配置</span></div>}
        {section === 'commands' && <div className="table-panel"><table><thead><tr><th>终端命令</th><th>启动方式</th><th>默认模型与路由</th><th>状态</th><th/></tr></thead><tbody>{rows.map(([id, value]) => <tr key={id}><td><button className="command-name" onClick={() => setEditor({ kind: section, id, value: copy(value) })}><span>›</span>{id}</button><p className="cell-note">{value.label || '自定义命令入口'}</p></td><td>{draft.management.templates[value.template]?.label || value.template}<p className="cell-note">{draft.management.templates[value.template]?.deferred ? '保留客户端启动逻辑' : '使用配置的启动参数'}</p></td><td><span className="model-name">{value.profile ? label(value.profile) : draft.management.templates[value.template]?.request_path ? '使用现有路由' : '客户端原生模型'}</span><p className="cell-note mono">{startupModel(value)}</p></td><td><button className={`toggle-label ${value.enabled !== false ? 'on' : ''}`} onClick={() => mutate(config => { config.management.commands[id].enabled = value.enabled === false; })}><i/>{value.enabled !== false ? '可启动' : '已停用'}</button></td><td className="row-actions"><button onClick={() => task(async () => setDetails({ title: `${id} · 启动预览`, value: await api('commands/preview', { name: id, args: [] }) }))} disabled={dirty}>预览</button><button onClick={() => setEditor({ kind: section, id, value: copy(value) })}>编辑</button><button className="delete-link" onClick={() => remove(section, id)}>删除</button></td></tr>)}</tbody></table>{!rows.length && <Empty text="添加一个名称，将它关联到启动方式和模型路由。" onAdd={add}/>}</div>}
        {section === 'backends' && <div className="backend-grid">{rows.map(([id, value]) => <article className="backend-card" key={id}><header><div className="backend-icon"><Icon name="api"/></div><span className="subtle-badge">{value.auth?.token_command ? '命令凭据' : value.auth?.token_env ? '环境变量' : value.auth?.token_file ? '凭据文件' : '未配置认证'}</span></header><h3>{id}</h3><div className="backend-url mono">{value.base_url}</div><div className="backend-detail"><span>关联模型</span><strong>{Object.values(draft.profiles).filter(profile => profile.standard.target === id || profile.fast?.target === id).length}</strong></div><div className="backend-detail"><span>凭据来源</span><span className="truncate mono" title={value.auth?.token_file || value.auth?.token_env || value.auth?.token_command?.join(' ')}>{value.auth?.token_file?.split('/').pop() || value.auth?.token_env || value.auth?.token_command?.[0]?.split('/').pop() || '—'}</span></div><footer><Button onClick={() => setEditor({ kind: section, id, value: copy(value) })}>编辑接入</Button><button onClick={() => task(async () => setDetails({ title: `${id} · 模型列表`, value: await api('backends/probe', { id, path: '/v1/models' }) }))} disabled={dirty}>读取模型列表</button><button className="delete-link" onClick={() => remove(section, id)}>删除</button></footer></article>)}{!rows.length && <Empty text="填写 API 地址，并设置凭据来源。" onAdd={add}/>}</div>}
        {section === 'models' && <div className="table-panel"><table><thead><tr><th>模型配置</th><th>标准目标</th><th>Fast 目标</th><th>关联路由</th><th/></tr></thead><tbody>{rows.map(([id, value]) => <tr key={id}><td><strong>{label(id)}</strong><p className="cell-note mono">{id}</p></td><td><span className="mono">{value.standard.model || value.standard.model_template || '保留请求模型'}</span><p className="cell-note">{value.standard.target}{value.standard.provider && ` · ${value.standard.provider}`}</p></td><td><span className="mono">{value.fast?.model || (value.fast ? '保留请求模型' : '沿用标准目标')}</span><p className="cell-note">{value.fast?.target || '—'}</p></td><td>{draft.routes.filter(route => route.profile === id).length} 条</td><td className="row-actions"><button onClick={() => setEditor({ kind: section, id, value: copy(value) })}>编辑</button><button className="delete-link" onClick={() => remove(section, id)}>删除</button></td></tr>)}</tbody></table>{!rows.length && <Empty text="为 API 后端定义可调用的模型配置。" onAdd={add}/>}</div>}
        {section === 'routes' && <div className="table-panel"><table><thead><tr><th>顺序与规则</th><th>匹配条件</th><th>模型配置</th><th/></tr></thead><tbody>{rows.map(([id, value]) => { const managed = draft.management.generated_routes?.includes(id); const match = value.match; return <tr key={id}><td><span className="route-order">{String(draft.routes.findIndex(row => row.id === id) + 1).padStart(2, '0')}</span><strong className="mono">{id}</strong>{managed && <p className="cell-note">由命令配置管理</p>}</td><td><div className="chips">{match.contexts?.map(item => <span key={`c${item}`}>{item}</span>)}{match.models?.map(item => <span className="mono" key={item}>{item}</span>)}{match.headers?.map(item => <span key={item.name}>{item.name}</span>)}{!Object.keys(match).length && <span className="amber-chip">默认路由</span>}{(match.model_prefixes?.length || match.model_contains?.length || match.last_user_contains_all?.length) > 0 && <span>内容与名称条件</span>}</div></td><td>{label(value.profile)}</td><td className="row-actions"><button aria-label={`上移 ${id}`} onClick={() => moveRoute(id, -1)} disabled={managed}>↑</button><button aria-label={`下移 ${id}`} onClick={() => moveRoute(id, 1)} disabled={managed}>↓</button><button disabled={managed} onClick={() => setEditor({ kind: section, id, value: copy(value) })}>编辑</button><button disabled={managed} className="delete-link" onClick={() => remove(section, id)}>删除</button></td></tr>; })}</tbody></table></div>}
        {section === 'templates' && <div className="backend-grid">{rows.map(([id, value]) => <article className="backend-card template-card" key={id}><header><div className="backend-icon"><Icon name="terminal"/></div><span className="subtle-badge">{value.deferred ? '启动器集成' : '直接启动'}</span></header><h3>{value.label || id}</h3><p className="template-description">{value.description || '自定义客户端启动方式'}</p><code className="backend-url">{value.program}</code><div className="backend-detail"><span>使用此方式的命令</span><strong>{Object.values(draft.management.commands).filter(command => command.template === id).length}</strong></div><footer><Button onClick={() => setEditor({ kind: section, id, value: copy(value) })}>编辑启动方式</Button><button className="delete-link" onClick={() => remove(section, id)}>删除</button></footer></article>)}</div>}
        {section === 'settings' && <><div className="settings-grid"><article className="settings-card"><div className="eyebrow">NETWORK</div><h3>router 服务</h3><dl><dt>监听地址</dt><dd>{draft.runtime.server.listen}</dd><dt>连接超时</dt><dd>{draft.runtime.server.connect_timeout_ms / 1000} 秒</dd><dt>请求超时</dt><dd>{draft.runtime.server.request_timeout_ms / 1000} 秒</dd><dt>请求大小上限</dt><dd>{draft.runtime.server.max_request_bytes / 1048576} MiB</dd></dl><Button onClick={() => task(async () => { await restartService(); notify('router 已重启。'); await load(); })} disabled={dirty || busy}><Icon name="refresh"/>重启服务</Button></article><article className="settings-card"><div className="eyebrow">BODY PROCESSING</div><h3>正文与资源</h3><dl><dt>读写缓冲</dt><dd>{draft.runtime.server.body_processing.io_buffer_bytes / 1024} KiB</dd><dt>正文处理并发数</dt><dd>{draft.runtime.server.body_processing.max_concurrent_requests}</dd><dt>控制字段读取上限</dt><dd>{draft.runtime.server.body_processing.metadata_limit_bytes / 1048576} MiB</dd><dt>暂存目录</dt><dd className="wrap-path">{draft.runtime.server.body_processing.spool_directory}</dd></dl></article></div><div className="config-location"><div><span className="eyebrow">CONFIGURATION</span><p className="mono">{status?.config_path}</p><small>凭据目录：{draft.management.credential_directory}</small></div><Button onClick={() => setEditor({ kind: 'raw', id: 'config', value: copy(draft) })}>编辑完整配置</Button></div></>}
        <div className={`savebar ${dirty ? 'is-dirty' : ''}`}><div><span className={`status-dot ${dirty ? 'amber' : ''}`}/><strong>{dirty ? '草稿尚未应用' : '配置已同步'}</strong><span>{dirty ? '应用前会检查引用关系和路由验证用例。' : `${draft.validation_cases.length} 个路由验证用例`}</span></div><div>{dirty && <Button onClick={() => setConfirm({ title: '放弃草稿修改', message: '恢复到最近读取的本机配置。', action: () => { setDraft(copy(snapshot.config)); setConfirm(null); } })}>放弃草稿</Button>}<Button primary disabled={!dirty || busy} onClick={prepareApply}>{busy ? '正在处理…' : '应用修改'}<Icon name="arrow" size={15}/></Button></div></div>
      </div>
    </main>
    {notice && <div className={`toast ${notice.error ? 'toast-error' : ''}`} role="status"><Icon name={notice.error ? 'close' : 'check'}/><span>{notice.message}</span><button onClick={() => setNotice(null)} aria-label="关闭通知">×</button></div>}
    {editor && <Editor key={`${editor.kind}:${editor.id}`} editor={editor} config={draft} close={() => setEditor(null)} notify={notify} save={(id, value, extras) => { if (editor.kind === 'settings') { mutate(config => { config.runtime.server = value; }); setEditor(null); } else if (editor.kind === 'raw') { setDraft(value); setEditor(null); } else saveRecord(editor.kind, editor.id, id, value, extras); }}/ >}
    {integrationEditor && <IntegrationEditor key={integrationEditor} id={integrationEditor} config={draft} ui={{Button,Field,Modal}} close={() => setIntegrationEditor(null)} onCommand={name => { setIntegrationEditor(null); setEditor({kind:'commands',id:name,value:copy(draft.management.commands[name])}); }} save={value => { mutate(config => { config.management.integrations[integrationEditor] = value; }); setIntegrationEditor(null); }}/ >}
    {preview && <Modal title="应用配置修改" eyebrow="REVIEW CHANGES" close={() => setPreview(null)} wide footer={<><Button onClick={() => setPreview(null)}>继续编辑</Button><Button primary disabled={busy} onClick={apply}>{pretty(snapshot.config.runtime.server) !== pretty(preview.config.runtime.server) ? '应用并重启服务' : '确认应用'}</Button></>}><div className="review-summary"><Icon name="check"/><div><strong>配置结构与路由验证通过</strong><p>{preview.validation_changes.length} 个验证用例的预期结果将随配置更新。</p></div></div>{pretty(snapshot.config.runtime.server) !== pretty(preview.config.runtime.server) && <div className="callout">转发服务参数发生变化，应用后会重启服务。</div>}{preview.validation_changes.length > 0 && <div className="review-list">{preview.validation_changes.map(change => <div key={change.name}><strong>{change.name}</strong><code>{change.after.rule_id} → {change.after.target} / {change.after.model}</code></div>)}</div>}<p className="muted">生效后，新的请求使用保存的路由配置。终端命令的启动参数在下次运行时读取。</p></Modal>}
    {details && <Modal title={details.title} close={() => setDetails(null)} wide footer={<Button onClick={() => setDetails(null)}>关闭</Button>}><pre className="code-preview">{pretty(details.value)}</pre></Modal>}
    {confirm && <Modal title={confirm.title} close={() => setConfirm(null)} footer={<><Button onClick={() => setConfirm(null)}>取消</Button><Button danger onClick={confirm.action}>确认</Button></>}><p>{confirm.message}</p></Modal>}
  </div>;
}

function Editor({ editor, config, close, save, notify }) {
  const [id, setId] = useState(editor.id);
  const [value, setValue] = useState(copy(editor.value));
  const [errors, setErrors] = useState({});
  const [raw, setRaw] = useState(false);
  const [rawText, setRawText] = useState(pretty(editor.value));
  const [secret, setSecret] = useState('');
  const [savingSecret, setSavingSecret] = useState(false);
  const [extras, setExtras] = useState({ label: config.management.labels?.[`profile:${editor.id}`] || '', bindCommand: '', alias: '', protocols: config.management.backend_protocols?.[editor.id] || [], profileModels: config.management.profile_models?.[editor.id] || {} });
  const kind = editor.kind;
  const title = kind === 'settings' ? '编辑服务参数' : kind === 'raw' ? '编辑完整配置' : `${editor.fresh ? '新增' : '编辑'}${sections.find(([key]) => key === kind)?.[1] || ''}`;
  function set(path, next) { setValue(old => { const value = copy(old); const parts = path.split('.'); let target = value; for (const part of parts.slice(0,-1)) target = target[part] ||= {}; const leaf = parts.at(-1); if (next === undefined) delete target[leaf]; else target[leaf] = next; return value; }); }
  const errorAt = name => invalid => setErrors(old => ({ ...old, [name]: invalid }));
  const option = (values, labels) => Object.keys(values).map(key => <option key={key} value={key}>{labels?.(key) || key}</option>);
  function submit() {
    try {
      if (Object.values(errors).some(Boolean) && !raw) throw new Error('请先修正未完成的 JSON 字段。');
      const record = raw || kind === 'raw' ? JSON.parse(rawText) : clean(value);
      if (!record || typeof record !== 'object' || Array.isArray(record)) throw new Error('配置必须为 JSON 对象。');
      if (kind === 'raw' && (!record.management?.commands || !record.management?.templates || !record.runtime?.server?.body_processing || !record.runtime?.backends || !record.profiles || !Array.isArray(record.routes))) throw new Error('完整配置需包含 management、runtime、profiles 和 routes。');
      if (kind === 'settings' && (!record.listen || !record.body_processing || !record.endpoints)) throw new Error('服务设置需保留监听地址、正文处理配置和接口路径。');
      if (kind === 'models' && !record.standard?.target) throw new Error('模型配置需要标准目标和后端。');
      if (!['settings','raw'].includes(kind) && !/^[A-Za-z0-9_][A-Za-z0-9_-]{0,63}$/.test(id)) throw new Error('名称需为 1～64 个字母、数字、下划线或连字符。');
      const collection = kind === 'commands' ? config.management.commands : kind === 'templates' ? config.management.templates : kind === 'backends' ? config.runtime.backends : kind === 'models' ? config.profiles : {};
      if (id !== editor.id && collection[id]) throw new Error('此名称已存在。');
      if (kind === 'models' && editor.fresh && !extras.bindCommand && !extras.alias) throw new Error('请选择一个关联命令，或填写可调用名称来创建路由。');
      if (kind === 'routes' && !Object.keys(record.match || {}).some(key => Array.isArray(record.match[key]) ? record.match[key].length : record.match[key] !== undefined) && editor.fresh) throw new Error('请填写至少一个路由匹配条件。');
      save(id, record, extras);
    } catch (e) { notify(e.message, true); }
  }
  async function saveSecret() {
    setSavingSecret(true);
    try { const data = await api('credentials', { id: `${id}-api-key`, value: secret }); set('auth', { headers: value.auth?.headers || { authorization: 'Bearer {token}' }, token_file: data.path }); setSecret(''); notify('密钥已保存到受保护文件，配置中只保留路径。'); } catch (e) { notify(e.message, true); } finally { setSavingSecret(false); }
  }
  const authType = value.auth?.token_file !== undefined ? 'file' : value.auth?.token_env !== undefined ? 'env' : value.auth?.token_command ? 'command' : 'none';
  function changeAuth(type) { const headers = value.auth?.headers || { authorization: 'Bearer {token}' }; set('auth', type === 'none' ? undefined : { headers, ...(type === 'file' ? { token_file: '' } : type === 'env' ? { token_env: '' } : { token_command: [] }) }); }
  function destination(which, label) {
    const destination = value[which];
    const tier = destination.field_policy?.set?.['/service_tier'] || '';
    return <section className="form-section"><h3>{label}</h3><div className="form-grid">
      <Field label="API 后端"><select value={destination.target || ''} onChange={e => set(`${which}.target`, e.target.value)}>{option(config.runtime.backends)}</select></Field>
      <Field label="API 模型 ID"><input value={destination.model || ''} onChange={e => set(`${which}.model`, e.target.value || undefined)} placeholder="由后端提供的模型名称"/></Field>
      <Field label="供应商锁定" hint="留空时使用后端的默认选择。"><input value={destination.provider || ''} onChange={e => set(`${which}.provider`, e.target.value || undefined)} placeholder="可选"/></Field>
      <Field label="服务等级" hint="Fast 使用上游快速服务等级，按该等级计费。"><select value={tier} onChange={e => set(`${which}.field_policy.set./service_tier`, e.target.value || undefined)}><option value="">跟随客户端选择</option><option value="default">标准</option><option value="priority">Fast · 快速</option><option value="flex">Flex · 弹性</option>{tier && !['default','priority','flex'].includes(tier) && <option value={tier}>{tier}</option>}</select></Field>
      <Field label="推理字段策略"><select value={destination.reasoning_adapter || ''} onChange={e => set(`${which}.reasoning_adapter`, e.target.value || undefined)}><option value="">保持原请求</option>{option(config.runtime.reasoning_policies)}</select></Field>
    </div></section>;
  }
  return <Modal title={title} close={close} wide footer={<><span className="footer-note">保存到草稿，应用后生效</span><Button onClick={close}>取消</Button><Button primary onClick={submit}>保存草稿</Button></>}>
    {kind !== 'raw' && <div className="editor-mode"><button className={!raw ? 'selected' : ''} onClick={() => { try { if (raw) setValue(JSON.parse(rawText)); setRaw(false); } catch (e) { notify(e.message,true); } }}>常用设置</button><button className={raw ? 'selected' : ''} onClick={() => { setRawText(pretty(value)); setRaw(true); }}>完整对象 JSON</button></div>}
    {raw || kind === 'raw' ? <Field label="JSON 配置" wide hint="保留未修改字段。应用时会进行完整校验。"><textarea className="code-input large" value={rawText} onChange={e => setRawText(e.target.value)} rows={23}/></Field> : <>
      {!['settings','raw'].includes(kind) && <div className="form-grid"><Field label={kind === 'commands' ? '命令名称' : '配置标识'} hint={kind === 'commands' ? '在终端中输入的名称，可自行命名。' : '用于配置引用，创建后保持稳定。'}><input autoFocus value={id} disabled={!editor.fresh && kind !== 'commands'} onChange={e => setId(e.target.value)} placeholder={kind === 'commands' ? '例如 cco' : '输入标识'}/></Field>{kind === 'commands' && <Field label="显示说明"><input value={value.label || ''} onChange={e => set('label',e.target.value)} placeholder="此命令的用途"/></Field>}{kind === 'models' && <Field label="显示名称"><input value={extras.label} onChange={e => setExtras(old => ({...old,label:e.target.value}))} placeholder="便于识别的模型名称"/></Field>}</div>}
      {kind === 'commands' && <><div className="form-grid">
        <Field label="启动方式"><select value={value.template} onChange={e => setValue(old => ({...old,template:e.target.value,options:{},mod_profile:config.management.templates[e.target.value]?.supports_mods ? old.mod_profile : undefined}))}>{option(config.management.templates, key => config.management.templates[key].label || key)}</select></Field>
        <Field label="默认启动模型" hint="留空时沿用客户端或启动器的选择。"><input list="model-aliases" value={value.model || ''} onChange={e => set('model',e.target.value || null)} placeholder="跟随客户端选择"/><datalist id="model-aliases">{[...new Set(config.routes.flatMap(route => route.match.models || []))].map(model => <option value={model} key={model}/>)}</datalist></Field>
        <Field label="独立模型路由" hint="绑定后，命令携带独立标记来选择模型配置。"><select value={value.profile || ''} onChange={e => set('profile',e.target.value || null)}><option value="">使用现有路由规则</option>{option(config.profiles, key => config.management.labels?.[`profile:${key}`] || key)}</select></Field>
        {Object.entries(config.management.templates[value.template]?.options || {}).map(([key, setting]) => <Field key={key} label={setting.label}><select value={value.options?.[key] || setting.default} onChange={e => setValue(old => ({...old,options:{...old.options,[key]:e.target.value}}))}>{Object.entries(setting.choices).map(([id,choice]) => <option key={id} value={id}>{choice.label}</option>)}</select></Field>)}
        {config.management.templates[value.template]?.supports_mods && config.management.mods && <Field label="Mod 组合" hint="下一次启动时读取组合。"><select value={value.mod_profile || ''} onChange={e => set('mod_profile', e.target.value || undefined)}><option value="">继承客户端设置</option>{Object.entries(config.management.mods.profiles).map(([id,profile]) => <option value={id} key={id}>{profile.label}</option>)}</select></Field>}
      </div><div className="callout">{config.management.templates[value.template]?.description || '可以独立设置程序、参数和环境变量。'}</div><details className="advanced"><summary>路由顺序、启动参数与环境变量</summary><div className="form-grid">
        <Field label="路由插入位置"><select value={value.route_before || ''} onChange={e => set('route_before',e.target.value || null)}><option value="">使用启动方式的默认位置</option>{config.routes.filter(route => !config.management.generated_routes?.includes(route.id)).map(route => <option key={route.id} value={route.id}>在 {route.id} 之前</option>)}</select></Field>
        <Field label="程序覆盖" hint="留空时使用启动方式中的程序。"><input value={value.program || ''} onChange={e => set('program',e.target.value || null)} placeholder="可选程序路径"/></Field><Field label="路由绑定范围"><select value={value.match_all_models ? 'all' : 'model'} onChange={e => set('match_all_models',e.target.value === 'all')}><option value="model">匹配默认启动模型</option><option value="all">匹配此命令携带标记的请求</option></select></Field><JsonField label="附加参数（JSON 数组）" value={value.args || []} onChange={next => set('args',next)} onError={errorAt('args')} rows={4}/><JsonField label="环境变量（JSON 对象）" value={value.env || {}} onChange={next => set('env',next)} onError={errorAt('env')} hint="真实 API 密钥在后端凭据中保存。此处用于客户端启动设置。"/><JsonField label="清除继承的环境变量（JSON 数组）" value={value.unset_env || []} onChange={next => set('unset_env',next)} onError={errorAt('unset')} rows={3}/></div></details></>}
      {kind === 'backends' && <><Field label="API 基础地址" wide><input value={value.base_url} onChange={e => set('base_url',e.target.value)} placeholder="https://api.example.com"/></Field><section className="form-section"><h3>认证与凭据</h3><div className="form-grid"><Field label="凭据来源"><select value={authType} onChange={e => changeAuth(e.target.value)}><option value="file">独立凭据文件</option><option value="env">环境变量</option><option value="command">凭据命令 / OAuth helper</option><option value="none">无需认证</option></select></Field>{authType === 'file' && <Field label="凭据文件路径"><input value={value.auth.token_file} onChange={e => set('auth.token_file',e.target.value)} placeholder="受保护文件的绝对路径"/></Field>}{authType === 'env' && <Field label="环境变量名称"><input value={value.auth.token_env} onChange={e => set('auth.token_env',e.target.value)} placeholder="PROVIDER_API_KEY"/></Field>}{authType === 'command' && <JsonField label="凭据命令（程序与参数数组）" value={value.auth.token_command} onChange={next => set('auth.token_command',next)} onError={errorAt('authcommand')} rows={3}/>}</div>{authType === 'file' && <div className="secret-box"><Field label="新增或更新 API Key" hint="保存后输入框会清空，后端配置只记录文件路径。"><input type="password" autoComplete="new-password" value={secret} onChange={e => setSecret(e.target.value)} placeholder="粘贴新的 API Key"/></Field><Button disabled={!secret || !id || savingSecret} onClick={saveSecret}>{savingSecret ? '正在保存…' : '保存密钥文件'}</Button></div>}{authType !== 'none' && <JsonField key={authType} label="认证请求头模板" value={value.auth.headers} onChange={next => set('auth.headers',next)} onError={errorAt('headers')} hint="用 {token} 引用凭据，例如 Authorization: Bearer {token}。" rows={4}/>}</section><details className="advanced"><summary>路径重写与供应商参数</summary><JsonField label="路径重写" value={value.path_rewrites || {}} onChange={next => set('path_rewrites',next)} onError={errorAt('paths')}/><JsonField label="供应商字段模板" value={value.provider_fields || {}} onChange={next => set('provider_fields',next)} onError={errorAt('provider')}/></details></>}
      {kind === 'models' && <>{destination('standard','标准目标')}<label className="check-field"><input type="checkbox" checked={!!value.fast} onChange={e => set('fast',e.target.checked ? copy(value.standard) : undefined)}/>单独配置 Fast 目标</label>{value.fast && destination('fast','Fast 目标')}{editor.fresh && <section className="form-section"><h3>关联入口</h3><div className="form-grid"><Field label="关联到现有命令"><select value={extras.bindCommand} onChange={e => setExtras(old => ({...old,bindCommand:e.target.value}))}><option value="">选择命令</option>{option(config.management.commands)}</select></Field><Field label="创建可调用名称" hint="填写后会新增一条按模型名称匹配的路由。"><input value={extras.alias} onChange={e => setExtras(old => ({...old,alias:e.target.value}))} placeholder="可选的客户端模型名称"/></Field></div></section>}<details className="advanced"><summary>字段策略、缓存和协议适配</summary><JsonField label="标准目标完整配置" value={value.standard} onChange={next => set('standard',next)} onError={errorAt('standard')} rows={12}/>{value.fast && <JsonField label="Fast 目标完整配置" value={value.fast} onChange={next => set('fast',next)} onError={errorAt('fast')} rows={10}/>}</details></>}
      {kind === 'routes' && <><Field label="目标模型配置"><select value={value.profile} onChange={e => set('profile',e.target.value)}>{option(config.profiles)}</select></Field><JsonField label="匹配条件" value={value.match} onChange={next => set('match',next)} onError={errorAt('match')} rows={13} hint="支持 contexts、models、model_prefixes、headers 等现有匹配字段。"/><label className="check-field"><input type="checkbox" checked={!!value.use_settings_fast_mode} onChange={e => set('use_settings_fast_mode',e.target.checked)}/>使用客户端设置中的 Fast 状态</label></>}
      {kind === 'templates' && <><div className="form-grid"><Field label="名称"><input value={value.label || ''} onChange={e => set('label',e.target.value)}/></Field><Field label="启动程序"><input value={value.program} onChange={e => set('program',e.target.value)} placeholder="程序名称或绝对路径"/></Field><Field label="说明" wide><input value={value.description || ''} onChange={e => set('description',e.target.value)}/></Field></div><JsonField label="基础参数" value={value.args || []} onChange={next => set('args',next)} onError={errorAt('args')}/><JsonField label="环境变量模板" value={value.env || {}} onChange={next => set('env',next)} onError={errorAt('env')}/><details className="advanced"><summary>接口标记与启动集成</summary><JsonField label="完整启动方式" value={value} onChange={next => setValue(next)} onError={errorAt('template')} rows={18} hint="支持 command_name、model、gateway_url、config 等模板变量。"/></details></>}
      {kind === 'settings' && <><div className="form-grid"><Field label="监听地址"><input value={value.listen} onChange={e => set('listen',e.target.value)}/></Field><Field label="健康检查路径"><input value={value.health_path} onChange={e => set('health_path',e.target.value)}/></Field><Field label="连接超时（毫秒）"><input type="number" min="1" value={value.connect_timeout_ms} onChange={e => set('connect_timeout_ms',Number(e.target.value))}/></Field><Field label="请求超时（毫秒）"><input type="number" min="1" value={value.request_timeout_ms} onChange={e => set('request_timeout_ms',Number(e.target.value))}/></Field><Field label="请求大小上限（MiB）"><input type="number" min="1" value={value.max_request_bytes / 1048576} onChange={e => set('max_request_bytes',Number(e.target.value) * 1048576)}/></Field><Field label="正文处理并发数"><input type="number" min="1" value={value.body_processing.max_concurrent_requests} onChange={e => set('body_processing.max_concurrent_requests',Number(e.target.value))}/></Field><Field label="读写缓冲（KiB）"><input type="number" min="1" value={value.body_processing.io_buffer_bytes / 1024} onChange={e => set('body_processing.io_buffer_bytes',Number(e.target.value) * 1024)}/></Field><Field label="暂存目录"><input value={value.body_processing.spool_directory} onChange={e => set('body_processing.spool_directory',e.target.value)}/></Field></div><JsonField label="接口路径与协议" value={value.endpoints} onChange={next => set('endpoints',next)} onError={errorAt('endpoints')} rows={10}/></>}
      {kind === 'templates' && <label className="check-field"><input type="checkbox" checked={!!value.supports_mods} onChange={e => set('supports_mods',e.target.checked)}/>支持 Mods 启动适配</label>}
      {kind === 'backends' && <section className="form-section"><h3>支持的接口协议</h3><div className="protocol-options">{['messages','responses','chat_completions'].map(protocol => <label className="check-field" key={protocol}><input type="checkbox" checked={extras.protocols.includes(protocol)} onChange={e => setExtras(old => ({...old,protocols:e.target.checked ? [...old.protocols,protocol] : old.protocols.filter(item => item !== protocol)}))}/>{protocol}</label>)}</div><p className="muted">按后端实际支持的接口选择。命令绑定模型时会检查协议。</p></section>}
      {kind === 'models' && <details className="advanced"><summary>客户端使用的模型名称</summary><div className="form-grid">{['messages','responses'].map(protocol => <Field label={`${protocol} 启动名称`} key={protocol} hint="可填写客户端兼容名称，留空时使用 API 模型 ID。"><input value={extras.profileModels[protocol] || ''} onChange={e => setExtras(old => ({...old,profileModels:clean({...old.profileModels,[protocol]:e.target.value || undefined})}))}/></Field>)}</div></details>}
    </>}
  </Modal>;
}

createRoot(document.getElementById('root')).render(<App/>);
