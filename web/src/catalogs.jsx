import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import './catalogs.css';
import { groupReasoningModels } from './reasoning-models.js';

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
  return <CatalogContext.Provider value={{catalogs,load,configure,config,api}}>{children}</CatalogContext.Provider>;
}

export function CatalogSelect({ backend, value, onSelect, label, savedProfiles = [], onProfile, disabled = false, groupReasoning = true }) {
  const {catalogs,load,configure,config} = useContext(CatalogContext);
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
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
  const models = groupReasoning ? groupReasoningModels(catalog?.models || [],settings) : catalog?.models || [];
  const selected = models.find(row=>row.id===value || Object.values(row.reasoning_models?.models || {}).includes(value));
  const selectedId = selected?.id || value;
  const rows = models.filter(row => `${row.id} ${row.name}`.toLowerCase().includes(query.toLowerCase()));
  const added = new Set(catalog?.added || []);
  const updates = (catalog?.added?.length || 0) + (catalog?.removed?.length || 0) + (catalog?.changed?.length || 0);
  async function refresh() { setBusy(true); try { await load(backend,true); } finally { setBusy(false); } }
  return <div className="catalog-select">
    <label className="field"><span className="field-label">{label}</span>
      <select aria-label={label} value={`model:${selectedId || ''}`} disabled={disabled || !backend} onChange={event => {
        if (event.target.value.startsWith('profile:')) onProfile?.(event.target.value.slice(8));
        else { const row = models.find(item => item.id === event.target.value.slice(6)); if (row) onSelect(row); }
      }}>
        {!rows.some(row => row.id === selectedId) && <option value={`model:${selectedId || ''}`}>{selected?.name || value || (busy ? '正在读取模型…' : '请选择模型')}{value ? ' · 当前目标' : ''}</option>}
        <optgroup label={`后端模型目录 · ${rows.length} 个`}>
          {rows.map(row => <option key={row.id} value={`model:${row.id}`}>{row.reasoning_models ? `${row.name} · ${Object.keys(row.reasoning_models.models).length} 档思考强度` : row.name !== row.id ? `${row.name} · ${row.id}` : row.id}{added.has(row.id) ? ' · 新增' : ''}</option>)}
        </optgroup>
        {!!savedProfiles.length && <optgroup label="已保存的路由配置">{savedProfiles.map(row => <option key={row.id} value={`profile:${row.id}`}>{row.label}</option>)}</optgroup>}
      </select>
    </label>
    <div className="catalog-tools"><input aria-label={`${label}搜索或自填`} placeholder="搜索模型，或填写完整模型 ID" value={query} onChange={e=>setQuery(e.target.value)}/><button type="button" disabled={!query.trim() || disabled} onClick={()=>onSelect(models.find(row=>row.id===query.trim()) || {id:query.trim(),name:query.trim()})}>使用此 ID</button><button type="button" disabled={busy || !backend} onClick={refresh}>{busy ? '同步中…' : '同步模型'}</button></div>
    <div className="catalog-info"><span>{models.length} 个目录模型{catalog?.updated_at ? ` · ${new Date(catalog.updated_at*1000).toLocaleTimeString()} 更新` : ''}</span><label><input type="checkbox" checked={!!settings.enabled} onChange={e=>configure(backend,{enabled:e.target.checked})}/>后台监控</label></div>
    {catalog?.error && <p className="inline-error" role="alert">{catalog.error}{models.length ? '；显示上次成功目录。' : '。可以保留当前目标或选择已保存的配置。'}</p>}
    {updates > 0 && <details className="catalog-updates"><summary>目录更新：新增 {catalog.added.length} · 移除 {catalog.removed.length} · 变更 {catalog.changed.length}</summary><p>{[...catalog.added.map(id=>`新增 ${id}`),...catalog.removed.map(id=>`移除 ${id}`),...catalog.changed.map(id=>`变更 ${id}`)].join('；')}</p><button type="button" onClick={()=>load(backend,false,true)}>标记已查看</button></details>}
    <details className="catalog-settings"><summary>目录同步设置</summary><label>检查间隔（秒）<input type="number" min="30" max="86400" value={settings.interval_seconds || 30} onChange={e=>configure(backend,{interval_seconds:Number(e.target.value)})}/></label><label>目录接口路径<input value={settings.path || '/v1/models'} onChange={e=>configure(backend,{path:e.target.value})}/></label><small>监控设置随“应用修改”保存；后台 Router 运行时持续检查。</small></details>
  </div>;
}

export function catalogDestination(config, backend, row) {
  return {...structuredClone(config.management.model_catalogs?.[backend]?.destination || {}),target:backend,model:row.id,...(row.reasoning_models ? {reasoning_models:structuredClone(row.reasoning_models)} : {})};
}

export function ReasoningControls({ destination, onChange, label }) {
  const {catalogs,config} = useContext(CatalogContext);
  const policy = destination?.reasoning_models;
  const available = groupReasoningModels(catalogs[destination?.target]?.models || [],config.management.model_catalogs?.[destination?.target]);
  const group = available.find(row=>Object.values(row.reasoning_models?.models || {}).includes(destination?.model));
  if (!policy) return group ? <button type="button" className="button" onClick={()=>onChange({...destination,reasoning_models:{...structuredClone(group.reasoning_models),default_effort:Object.keys(group.reasoning_models.models).find(effort=>group.reasoning_models.models[effort]===destination.model) || group.reasoning_models.default_effort}})}>启用思考强度切换</button> : null;
  const update = changes => {
    const next = {...policy,...changes};
    onChange({...destination,model:next.models[next.default_effort],reasoning_models:next});
  };
  const efforts = Object.keys(policy.models);
  return <section className="reasoning-controls">
    <div className="form-grid">
      <label className="field"><span className="field-label">{label} · 强度控制</span><select aria-label={`${label}强度控制`} value={policy.follow_request ? 'client' : 'fixed'} onChange={event=>update({follow_request:event.target.value==='client'})}><option value="client">跟随客户端</option><option value="fixed">使用固定强度</option></select></label>
      <label className="field"><span className="field-label">{policy.follow_request ? '默认思考强度' : '思考强度'}</span><select aria-label={`${label}思考强度`} value={policy.default_effort} onChange={event=>update({default_effort:event.target.value})}>{efforts.map(effort=><option key={effort} value={effort}>{policy.labels?.[effort] || effort}</option>)}</select><small>{policy.follow_request ? '客户端指定强度时优先；未指定时使用此默认值。' : '此强度用于每次请求。'}</small></label>
    </div>
    <details><summary>思考强度对应模型</summary>{efforts.map(effort=><CatalogSelect key={effort} backend={destination.target} value={policy.models[effort]} label={`${label} · ${policy.labels?.[effort] || effort}`} groupReasoning={false} onSelect={row=>update({models:{...policy.models,[effort]:row.id}})}/>)}</details>
  </section>;
}

export function ProviderSelect({ backend, model, value, onChange, label }) {
  const {config,api} = useContext(CatalogContext);
  const [result,setResult] = useState({providers:[]});
  const [busy,setBusy] = useState(false);
  const list = React.useId();
  const supported = !!config.management.model_catalogs?.[backend]?.providers;
  useEffect(()=>{
    let active = true;
    setResult({providers:[]});
    if (!supported || !model) {setBusy(false);return;}
    setBusy(true);
    api('catalogs/providers',{backend,model}).then(result=>{if(active)setResult(result);}).catch(error=>{if(active)setResult({providers:[],error:error.message});}).finally(()=>{if(active)setBusy(false);});
    return ()=>{active=false;};
  },[backend,model,supported,api]);
  return <label className="field"><span className="field-label">{label}</span><input aria-label={label} list={list} value={value || ''} onChange={event=>onChange(event.target.value)} placeholder="选择或填写 provider；留空由后端选择"/><datalist id={list}>{result.providers.map(row=><option key={row.id} value={row.id}>{row.name}</option>)}</datalist><small>{busy ? '正在读取该模型的供应商…' : result.error ? `${result.error}；可手动填写供应商标识。` : result.providers.length ? `${result.providers.length} 个供应商，可搜索或手动填写。` : '填写后使用此供应商标识。'}</small></label>;
}
