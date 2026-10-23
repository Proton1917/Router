import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import './catalogs.css';

const CatalogContext = createContext(null);

export function CatalogProvider({ config, mutate, api, children }) {
  const [catalogs, setCatalogs] = useState({});
  const pending = useRef(new Map());
  const load = useCallback((backend, refresh = false, acknowledge = false) => {
    if (pending.current.has(backend)) return pending.current.get(backend);
    const request = api('catalogs/read', {backend, refresh, acknowledge}).then(data => {
      setCatalogs(old => ({...old,[backend]:data}));
      return data;
    }).catch(error => {
      setCatalogs(old => ({...old,[backend]:{...old[backend],error:error.message}}));
    }).finally(() => pending.current.delete(backend));
    pending.current.set(backend, request);
    return request;
  }, [api]);
  const configure = (backend, patch) => mutate(next => {
    next.management.model_catalogs ||= {};
    next.management.model_catalogs[backend] = {...next.management.model_catalogs[backend],...patch};
  });
  return <CatalogContext.Provider value={{catalogs,load,configure,config}}>{children}</CatalogContext.Provider>;
}

export function CatalogSelect({ backend, value, onSelect, label, savedProfiles = [], onProfile, disabled = false }) {
  const {catalogs,load,configure,config} = useContext(CatalogContext);
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [manual, setManual] = useState('');
  const catalog = catalogs[backend];
  const settings = config.management.model_catalogs?.[backend] || {};
  useEffect(() => {
    if (!backend || disabled) return;
    let active = true;
    setBusy(true);
    load(backend,true).finally(() => { if (active) setBusy(false); });
    const timer = setInterval(() => { if (document.visibilityState === 'visible') load(backend); }, 30000);
    return () => {active = false; clearInterval(timer);};
  }, [backend,disabled,load]);
  const models = catalog?.models || [];
  const rows = models.filter(row => `${row.id} ${row.name}`.toLowerCase().includes(query.toLowerCase()));
  const added = new Set(catalog?.added || []);
  const updates = (catalog?.added?.length || 0) + (catalog?.removed?.length || 0) + (catalog?.changed?.length || 0);
  async function refresh() { setBusy(true); try { await load(backend,true); } finally { setBusy(false); } }
  return <div className="catalog-select">
    <label className="field"><span className="field-label">{label}</span>
      <select aria-label={label} value={`model:${value || ''}`} disabled={disabled || !backend} onChange={event => {
        if (event.target.value.startsWith('profile:')) onProfile?.(event.target.value.slice(8));
        else { const row = models.find(item => item.id === event.target.value.slice(6)); if (row) onSelect(row); }
      }}>
        {!rows.some(row => row.id === value) && <option value={`model:${value || ''}`}>{value || (busy ? '正在读取模型…' : '请选择模型')}{value ? ' · 当前目标' : ''}</option>}
        <optgroup label={`后端模型目录 · ${rows.length} 个`}>
          {rows.map(row => <option key={row.id} value={`model:${row.id}`}>{row.name !== row.id ? `${row.name} · ${row.id}` : row.id}{added.has(row.id) ? ' · 新增' : ''}</option>)}
        </optgroup>
        {!!savedProfiles.length && <optgroup label="已保存的路由配置">{savedProfiles.map(row => <option key={row.id} value={`profile:${row.id}`}>{row.label}</option>)}</optgroup>}
      </select>
    </label>
    <div className="catalog-tools"><input aria-label={`${label}搜索`} placeholder="搜索模型名称或 ID" value={query} onChange={e=>setQuery(e.target.value)}/><button type="button" disabled={busy || !backend} onClick={refresh}>{busy ? '同步中…' : '同步模型'}</button></div>
    <div className="catalog-info"><span>{models.length} 个目录模型{catalog?.updated_at ? ` · ${new Date(catalog.updated_at*1000).toLocaleTimeString()} 更新` : ''}</span><label><input type="checkbox" checked={!!settings.enabled} onChange={e=>configure(backend,{enabled:e.target.checked})}/>后台监控</label></div>
    {catalog?.error && <p className="inline-error" role="alert">{catalog.error}{models.length ? '；显示上次成功目录。' : '。可以保留当前目标或选择已保存的配置。'}</p>}
    {updates > 0 && <details className="catalog-updates"><summary>目录更新：新增 {catalog.added.length} · 移除 {catalog.removed.length} · 变更 {catalog.changed.length}</summary><p>{[...catalog.added.map(id=>`新增 ${id}`),...catalog.removed.map(id=>`移除 ${id}`),...catalog.changed.map(id=>`变更 ${id}`)].join('；')}</p><button type="button" onClick={()=>load(backend,false,true)}>标记已查看</button></details>}
    <details className="catalog-settings"><summary>目录同步设置</summary><label>检查间隔（秒）<input type="number" min="30" max="86400" value={settings.interval_seconds || 30} onChange={e=>configure(backend,{interval_seconds:Number(e.target.value)})}/></label><label>目录接口路径<input value={settings.path || '/v1/models'} onChange={e=>configure(backend,{path:e.target.value})}/></label><small>监控设置随“应用修改”保存；后台 Router 运行时持续检查。</small></details>
    <details className="catalog-settings"><summary>手动填写模型 ID</summary><div className="catalog-tools"><input aria-label={`${label}手动 ID`} value={manual} onChange={e=>setManual(e.target.value)} placeholder="服务商提供的 API 模型 ID"/><button type="button" disabled={!manual.trim()} onClick={()=>onSelect({id:manual.trim(),name:manual.trim()})}>使用此 ID</button></div></details>
  </div>;
}

export function catalogDestination(config, backend, model) {
  return {...structuredClone(config.management.model_catalogs?.[backend]?.destination || {}),target:backend,model};
}
