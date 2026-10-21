import React, { useEffect, useRef, useState } from 'react';
import './commands.css';
import { customizeCommandProfile } from './model-ownership.js';

export function modelLabel(config, item, profiles = config.profiles, speed = 'standard') {
  const profile = profiles[item.profile];
  const target = speed === 'fast' ? profile?.fast || profile?.standard : profile?.standard;
  const model = target?.model || item.model;
  return item.label || config.management.labels?.[`model:${model}`] || model;
}

export function CommandList({ config, rows, edit, preview, remove, toggle, dirty, ui, workspace }) {
  const { Button } = ui;
  return <div className="command-grid">{rows.map(([name, command]) => {
    const template = config.management.templates[command.template];
    const transcription = template?.env?.ROUTER_SPT === '1';
    const selectedModel = transcription ? config.management.transcription?.default_model : command.model;
    const authOption = template?.options?.['auth-mode'];
    const accountMode = authOption && (command.options?.['auth-mode'] || authOption.default) === 'account';
    const speedOption = command.model_bindings?.length ? template?.options?.['router-speed'] : undefined;
    const speed = speedOption ? command.options?.['router-speed'] || speedOption.default : 'standard';
    const bindings = transcription ? Object.entries(config.management.transcription?.models || {}).map(([id,item])=>({...item,id,model:id,visible:true})) : command.model_bindings?.length ? command.model_bindings : command.profile ? [{id:'default',model:command.model,profile:command.profile,visible:true}] : [];
    return <article className="command-card" key={name}>
      <header><div><button className="command-name" onClick={() => edit(name, command)}><span>›</span>{name}</button><p className="cell-note">{command.label || template?.label}</p></div><button className={`toggle-label ${command.enabled !== false ? 'on' : ''}`} onClick={() => toggle(name, command)}><i/>{command.enabled !== false ? '可启动' : '已停用'}</button></header>
      <div className="command-card-meta"><span>{accountMode ? '独立账号登录' : template?.label}</span><span>{accountMode ? '官方模型菜单' : `${bindings.filter(item => item.visible).length || '原生'}${bindings.length ? ' 个可选模型' : '模型'}`}</span>{speedOption && !accountMode && <span>API · {speed === 'client' ? '客户端 /fast' : speed === 'fast' ? 'Router Fast' : '标准'}</span>}</div>
      <div className="command-model-summary">{accountMode ? <p className="muted">使用独立账号的订阅额度，模型在 Codex 内选择。API 模型列表保留。</p> : bindings.map(item => {
        const profile = config.profiles[item.profile];
        const target = speed === 'fast' ? profile?.fast || profile?.standard : profile?.standard;
        return <div key={item.id} className={item.visible ? '' : 'model-hidden'}><span><strong>{modelLabel(config,item,config.profiles,speed)}</strong>{speed === 'fast' && profile?.fast && <em>Fast</em>}{selectedModel === item.model && <em>默认</em>}{!item.visible && <em>隐藏</em>}</span><small>{config.management.labels?.[`backend:${target?.target}`] || target?.target}{target?.provider ? ` · ${target.provider}` : ''}</small></div>;
      })}{!accountMode && !bindings.length && <p className="muted">{template?.request_path ? '沿用现有模型规则，可在编辑中添加模型列表。' : '使用客户端的账号与模型选择。'}</p>}</div>
      <footer><Button primary onClick={() => edit(name, command)}>编辑命令与模型</Button>{transcription && <button onClick={()=>workspace('transcription')}>转写模型与参数</button>}<button disabled={dirty} onClick={() => preview(name)}>启动预览</button><button className="delete-link" onClick={() => remove(name)}>删除</button></footer>
    </article>;
  })}</div>;
}

export function CommandEditor({ editor, config, close, save, ui, api, workspace }) {
  const { Button, Field, Modal, JsonField } = ui;
  const [name, setName] = useState(editor.id);
  const [editingKey] = useState(() => editor.id || `draft-${crypto.randomUUID()}`);
  const [value, setValue] = useState(structuredClone(editor.value));
  const [profiles, setProfiles] = useState({});
  const [error, setError] = useState('');
  const [invalid, setInvalid] = useState({});
  const [contextLimits, setContextLimits] = useState({});
  const [loadingLimits, setLoadingLimits] = useState(false);
  const [limitsError, setLimitsError] = useState('');
  const discoveryRequest = useRef(0);
  const template = config.management.templates[value.template];
  const transcription = template?.env?.ROUTER_SPT === '1';
  const accountEnabled = !!template?.env?.ROUTER_CODEX_ACCOUNT_ARGS;
  const accountMode = accountEnabled && (value.options?.['auth-mode'] || template.options?.['auth-mode']?.default) === 'account';
  const accountHome = value.env?.ROUTER_CODEX_ACCOUNT_HOME ?? template?.env?.ROUTER_CODEX_ACCOUNT_HOME?.replaceAll('{{ config.management.credential_directory }}',config.management.credential_directory).replaceAll('{{ command_name }}',name) ?? '';
  const protocol = config.runtime.server.endpoints[template?.request_path];
  const allProfiles = {...config.profiles,...profiles};
  const eligible = Object.entries(allProfiles).filter(([, profile]) => profile.standard.model && (!config.management.backend_protocols?.[profile.standard.target] || config.management.backend_protocols[profile.standard.target].includes(protocol)));
  const backends = Object.keys(config.runtime.backends).filter(key => !config.management.backend_protocols?.[key] || config.management.backend_protocols[key].includes(protocol));
  const bindings = value.model_bindings || [];
  const discoveryEnabled = !!(config.management.context_discovery && template?.context_window_range);
  const contextSignature = JSON.stringify([value.template,config.management.context_discovery,bindings.map(item => [item.id,allProfiles[item.profile]?.standard,allProfiles[item.profile]?.fast]),Object.entries(config.runtime.backends).map(([id,backend]) => [id,backend.base_url])]);
  async function loadContextLimits(refresh = false) {
    const requestId = ++discoveryRequest.current;
    setLoadingLimits(true); setLimitsError(''); setContextLimits({});
    try {
      const candidate = {...config,profiles:allProfiles,management:{...config.management,commands:{...config.management.commands,[editingKey]:value}}};
      const result = await api('models/context',{config:candidate,command_name:editingKey,refresh});
      if (discoveryRequest.current === requestId) setContextLimits(result.commands[editingKey] || {});
    } catch (error) { if (discoveryRequest.current === requestId) setLimitsError(error.message); }
    finally { if (discoveryRequest.current === requestId) setLoadingLimits(false); }
  }
  useEffect(() => {
    setContextLimits({});
    if (!discoveryEnabled) return;
    setLoadingLimits(true);
    const timer = setTimeout(() => loadContextLimits(),250);
    return () => { clearTimeout(timer); discoveryRequest.current++; };
  },[contextSignature,discoveryEnabled]);
  function contextMaximum(item) { return Math.min(template.context_window_range.max,contextLimits[item.id]?.max_tokens ?? Infinity); }
  function contextHint(item) {
    const maximum = contextMaximum(item);
    const entry = contextLimits[item.id];
    const detail = loadingLimits ? '正在读取后端模型与供应商上限。' : limitsError ? limitsError : entry?.destinations.map(row => `${row.speed === 'fast' ? 'Fast' : '标准'}：${row.max_tokens == null ? row.error || row.message || '上限未知' : `${row.max_tokens.toLocaleString()} tokens`}${row.provider ? ` · ${row.provider}` : ''}${row.resolved_models ? ` · ${row.resolved_models.join('、')}` : ''}`).join('；');
    if (maximum < template.context_window_range.min) return `${detail} 后端上限低于此启动方式的最小可填写预算，请留空，启动时自动使用已识别上限。`;
    return `${detail || ''} ${entry?.complete ? '已按全部目标限制预算。' : discoveryEnabled && !loadingLimits ? '未公布数值的目标，其最大窗口仍未知。' : ''} 可填写 ${template.context_window_range.min.toLocaleString()}～${maximum.toLocaleString()} tokens。留空沿用启动器基线，并受已识别上限限制；修改后重新启动命令。`;
  }
  const set = (key, next) => setValue(old => ({...old,[key]:next}));
  const label = item => modelLabel(config,item,allProfiles);
  function update(index, changes) { setValue(old => ({...old,model_bindings:old.model_bindings.map((item,at) => at === index ? {...item,...changes} : item)})); }
  function selectProfile(index, profile) {
    const item = bindings[index];
    const model = config.management.profile_models?.[profile]?.[protocol] || allProfiles[profile].standard.model;
    const existing = editor.value.model_bindings?.some(row => row.id === item.id);
    const aliases = [...new Set([...(item.aliases || []),...(existing && item.model && item.model !== model ? [item.model] : [])])].filter(alias => alias !== model);
    setValue(old => ({...old,model:old.model === item.model ? model : old.model,model_bindings:old.model_bindings.map((row,at) => at === index ? {...row,profile,model,aliases,label:'',description:''} : row)}));
  }
  function add() {
    const profile = eligible[0]?.[0];
    if (!profile) { setError('请先添加支持此启动方式的 API 后端和模型配置。'); return; }
    const model = config.management.profile_models?.[profile]?.[protocol] || allProfiles[profile].standard.model;
    setValue(old => {
      const previous = old.model_bindings?.length ? old.model_bindings : old.profile ? [{id:'primary',profile:old.profile,model:old.model || config.management.profile_models?.[old.profile]?.[protocol] || allProfiles[old.profile].standard.model,label:'',description:'',visible:true,aliases:[],route_ids:[]}] : [];
      return {...old,profile:null,match_all_models:false,model:old.model || (old.profile ? previous[0].model : editor.fresh ? model : null),model_bindings:[...previous,{id:`m${crypto.randomUUID().slice(0,8)}`,model,profile,label:'',description:'',visible:true,aliases:[],route_ids:[]}]};
    });
  }
  function customize(index, modify) {
    const item = bindings[index];
    const editingConfig = {...config,profiles:allProfiles,management:{...config.management,commands:{...config.management.commands,[editingKey]:value}}};
    const {id,profile} = customizeCommandProfile(editingConfig,editingKey,item.id,modify);
    setProfiles(old => ({...old,[id]:profile}));
    update(index,{profile:id});
  }
  function destination(index, which) {
    const item = bindings[index];
    const target = allProfiles[item.profile][which];
    return <div className="form-grid command-destination">
      <Field label={`${which === 'fast' ? 'Fast' : '标准'} API 后端`}><select value={target.target} onChange={event => customize(index,next => { next[which] = {target:event.target.value,model:''}; })}>{backends.map(id => <option key={id} value={id}>{config.management.labels?.[`backend:${id}`] || id}</option>)}</select></Field>
      <Field label={`${which === 'fast' ? 'Fast' : '标准'} API 模型 ID`}><input value={target.model || ''} onChange={event => customize(index,next => { next[which].model = event.target.value; })}/></Field>
      <Field label={`${which === 'fast' ? 'Fast' : '标准'}供应商`}><input value={target.provider || ''} placeholder="使用后端默认选择" onChange={event => customize(index,next => { if (event.target.value) next[which].provider = event.target.value; else delete next[which].provider; })}/></Field>
      <Field label={`${which === 'fast' ? 'Fast' : '标准'}服务等级`}><select value={target.field_policy?.set?.['/service_tier'] || ''} onChange={event => customize(index,next => { next[which].field_policy ||= {}; next[which].field_policy.set ||= {}; if (event.target.value) next[which].field_policy.set['/service_tier'] = event.target.value; else delete next[which].field_policy.set['/service_tier']; })}><option value="">跟随请求</option><option value="default">标准</option><option value="priority">优先</option><option value="flex">弹性</option>{target.field_policy?.set?.['/service_tier'] && !['default','priority','flex'].includes(target.field_policy.set['/service_tier']) && <option>{target.field_policy.set['/service_tier']}</option>}</select></Field>
    </div>;
  }
  function submit() {
    try {
      if (!/^[A-Za-z0-9_][A-Za-z0-9_-]{0,63}$/.test(name)) throw new Error('命令名称需为 1～64 个字母、数字、连字符或下划线。');
      if (name !== editor.id && config.management.commands[name]) throw new Error('命令名称已存在。');
      if (Object.values(invalid).some(Boolean)) throw new Error('请修正未完成的 JSON 字段。');
      if (accountEnabled && !accountHome.trim()) throw new Error('请填写独立账号目录。');
      if (bindings.length && !bindings.some(item => item.visible)) throw new Error('至少需要一个可见模型。');
      const ids = new Set();
      for (const item of bindings) {
        if (!item.model.trim() || !allProfiles[item.profile]) throw new Error('请完整设置每个模型条目。');
        if (item.context_window != null && (!template.context_window_range || !Number.isSafeInteger(item.context_window) || item.context_window < template.context_window_range.min || item.context_window > template.context_window_range.max)) throw new Error('请在启动方式允许的范围内填写上下文预算。');
        if (item.context_window != null && discoveryEnabled) {
          if (loadingLimits || !contextLimits[item.id]) throw new Error(limitsError || '请等待模型上下文上限读取完成。');
          const failed = contextLimits[item.id].destinations.find(row => row.error);
          if (failed) throw new Error(failed.error);
          if (item.context_window > contextMaximum(item)) throw new Error(`${label(item)} 的上下文预算不能超过 ${contextMaximum(item).toLocaleString()} tokens。`);
        }
        for (const id of [item.model,...(item.aliases || [])]) { if (ids.has(id.toLowerCase())) throw new Error(`模型标识重复：${id}`); ids.add(id.toLowerCase()); }
      }
      if (bindings.length && value.model && !bindings.some(item => item.visible && item.model === value.model)) throw new Error('默认模型需要在可见列表中。');
      save(name,value,{profiles:Object.fromEntries(Object.entries(profiles).filter(([id]) => bindings.some(item => item.profile === id)))});
    } catch (error) { setError(error.message); }
  }
  return <Modal title={editor.fresh ? '新增启动命令' : `${name} · 命令与模型`} eyebrow="启动命令" close={close} wide footer={<><span className="footer-note">路由应用后生效；菜单在下次启动时读取</span><Button onClick={close}>取消</Button><Button primary onClick={submit}>保存草稿</Button></>}>
    {transcription && <div className="callout">模型、分段、输出与保留设置和语音转写页面同步。<Button onClick={()=>{close();workspace('transcription');}}>编辑转写模型与参数</Button></div>}
    <div className="form-grid">
      <Field label="命令名称"><input value={name} onChange={event => setName(event.target.value)}/></Field>
      <Field label="显示说明"><input value={value.label || ''} onChange={event => set('label',event.target.value)}/></Field>
      <Field label="启动方式"><select value={value.template} onChange={event => setValue(old => ({...old,template:event.target.value,options:{},mod_profile:config.management.templates[event.target.value]?.supports_mods ? old.mod_profile : undefined}))}>{Object.entries(config.management.templates).map(([id,template]) => <option key={id} value={id}>{template.label}</option>)}</select></Field>
      <Field label={accountEnabled ? 'API 默认启动模型' : '默认启动模型'} hint={transcription ? '与语音转写页面的默认模型同步。' : accountMode ? '此选择保留给 API 模式；账号模式在 Codex 内选择官方模型。' : '跟随启动器时，保留此命令的现有默认选择。'}>
        {transcription ? <input disabled value={config.management.transcription?.default_model || '尚未配置'}/> : bindings.length ? <select disabled={accountMode} value={value.model || ''} onChange={event => set('model',event.target.value || null)}><option value="">跟随客户端或启动器</option>{bindings.filter(item => item.visible).map(item => <option key={item.id} value={item.model}>{label(item)}</option>)}</select> : <input disabled={accountMode} value={value.model || ''} onChange={event => set('model',event.target.value || null)} placeholder="跟随客户端或启动器"/>}
      </Field>
    </div>
    {Object.entries(template?.options || {}).filter(([key]) => !accountMode || key !== 'service-tier').map(([key,option]) => <Field key={key} label={option.label} hint={key === 'auth-mode' ? '设置默认启动模式；终端也可使用 --router-mode api 或 --router-mode account 临时选择。' : key === 'router-speed' && !bindings.length ? '在可选模型中添加模型后，可以选择 Router 服务等级。' : undefined}><select disabled={key === 'router-speed' && !bindings.length} value={value.options?.[key] || option.default} onChange={event => setValue(old => ({...old,options:{...old.options,[key]:event.target.value}}))}>{Object.entries(option.choices).map(([id,choice]) => <option key={id} value={id}>{choice.label}</option>)}</select></Field>)}
    {accountEnabled && <section className="form-section"><h3>独立账号登录</h3><Field label="账号配置与凭据目录" hint="此目录保存该入口的登录凭据、配置与会话。使用独立目录即可同时保留其他账号。"><input value={accountHome} onChange={event => setValue(old => ({...old,env:{...old.env,ROUTER_CODEX_ACCOUNT_HOME:event.target.value}}))}/></Field><p className="muted">首次登录：<code>{name || '命令名称'} --router-mode account login</code>；查看登录状态：<code>{name || '命令名称'} --router-mode account login status</code>。账号模式的模型与 Fast 在 Codex 内选择，API 模型配置会保留。</p></section>}
    {template?.request_path && !accountMode && <section className="command-model-editor"><header><div><h3>可选模型 <span>{bindings.length}</span></h3><p>显示名称留空时跟随后端模型。菜单、切换提示与状态名称使用同一份配置。</p></div><div>{discoveryEnabled && <Button disabled={loadingLimits} onClick={() => loadContextLimits(true)}>刷新模型上限</Button>}<Button onClick={add}>＋ 添加模型</Button></div></header>
      {!bindings.length && <div className="callout">{value.profile ? `当前绑定：${config.management.labels?.[`profile:${value.profile}`] || value.profile}。` : '当前沿用现有路由规则。'}<Button onClick={() => { if (value.profile) setValue(old => ({...old,profile:null,match_all_models:false,model_bindings:[{id:'primary',profile:old.profile,model:old.model || config.management.profile_models?.[old.profile]?.[protocol] || allProfiles[old.profile].standard.model,label:'',description:'',visible:true,aliases:[],route_ids:[]}]})); else add(); }}>管理模型列表</Button></div>}
      {bindings.map((item,index) => { const profile = allProfiles[item.profile]; const target = profile?.standard; return <article className="command-model-item" key={item.id}>
        <header><div><span className="model-position">{String(index+1).padStart(2,'0')}</span><strong>{label(item)}</strong><small>{config.management.labels?.[`backend:${target?.target}`] || target?.target}</small></div><div className="model-item-actions"><button aria-label={`上移模型 ${index+1}`} disabled={index === 0} onClick={() => { const next = [...bindings]; [next[index-1],next[index]] = [next[index],next[index-1]]; set('model_bindings',next); }}>↑</button><button aria-label={`下移模型 ${index+1}`} disabled={index === bindings.length-1} onClick={() => { const next = [...bindings]; [next[index+1],next[index]] = [next[index],next[index+1]]; set('model_bindings',next); }}>↓</button><button className="delete-link" aria-label={`删除模型 ${index+1}`} onClick={() => setValue(old => ({...old,model:old.model === item.model ? null : old.model,model_bindings:old.model_bindings.filter(row => row.id !== item.id)}))}>删除</button></div></header>
        <div className="form-grid"><Field label={`模型 ${index+1} 的 API 后端`}><select value={target?.target || ''} onChange={event => { const found = eligible.find(([,profile]) => profile.standard.target === event.target.value); if (found) selectProfile(index,found[0]); else customize(index,next => { next.standard = {target:event.target.value,model:''}; delete next.fast; }); }}>{backends.map(id => <option key={id} value={id}>{config.management.labels?.[`backend:${id}`] || id}</option>)}</select></Field><Field label={`模型 ${index+1} 的模型配置`}><select value={item.profile} onChange={event => selectProfile(index,event.target.value)}>{eligible.filter(([,profile]) => profile.standard.target === target?.target).map(([id,profile]) => <option key={id} value={id}>{config.management.labels?.[`profile:${id}`] || profile.standard.model}</option>)}</select></Field><Field label={`模型 ${index+1} 的显示名称`} hint="留空时自动跟随当前 API 模型。"><input value={item.label || ''} placeholder={modelLabel(config,{...item,label:''},allProfiles)} onChange={event => update(index,{label:event.target.value})}/></Field><Field label={`模型 ${index+1} 的菜单说明`}><input value={item.description || ''} placeholder="自动显示模型与服务来源" onChange={event => update(index,{description:event.target.value})}/></Field></div>
        <div className="command-model-options"><label className="check-field"><input type="checkbox" checked={item.visible} onChange={event => update(index,{visible:event.target.checked})}/>在客户端菜单显示</label><span>Fast：{profile?.fast ? [profile.fast.model || item.model,config.management.labels?.[`backend:${profile.fast.target}`] || profile.fast.target,profile.fast.provider].filter(Boolean).join(' · ') : '沿用标准目标'}</span></div>
        {template.context_window_range && <Field label={`模型 ${index+1} 的上下文预算（tokens）`} hint={contextHint(item)}><input type="number" min={template.context_window_range.min} max={contextMaximum(item)} step="1" value={item.context_window ?? ''} placeholder="沿用启动器基线" onChange={event => update(index,{context_window:event.target.value === '' ? null : Number(event.target.value)})}/></Field>}
        <details className="advanced"><summary>自定义标准与 Fast 目标</summary><p className="muted">独享配置直接更新；共享配置会生成此入口使用的独立副本。</p>{destination(index,'standard')}<label className="check-field"><input type="checkbox" checked={!!profile?.fast} onChange={event => customize(index,next => { if (event.target.checked) next.fast = structuredClone(next.standard); else delete next.fast; })}/>单独配置 Fast 目标</label>{profile?.fast && destination(index,'fast')}</details>
        <details className="advanced"><summary>客户端标识与兼容路由</summary><Field label={`模型 ${index+1} 的客户端标识`}><input value={item.model} onChange={event => update(index,{model:event.target.value})}/></Field><Field label={`模型 ${index+1} 的兼容名称`} hint="每行一个，保留已有会话所用的名称。"><textarea rows={2} value={(item.aliases || []).join('\n')} onChange={event => update(index,{aliases:event.target.value.split('\n').filter(Boolean)})}/></Field>{item.route_ids?.length > 0 && <p className="muted">同时更新 {item.route_ids.length} 条已关联的兼容路由。</p>}</details>
      </article>; })}
    </section>}
    {template?.supports_mods && config.management.mods && <Field label="Mod 组合"><select value={value.mod_profile || ''} onChange={event => set('mod_profile',event.target.value || undefined)}><option value="">继承客户端设置</option>{Object.entries(config.management.mods.profiles).map(([id,item]) => <option key={id} value={id}>{item.label}</option>)}</select></Field>}
    <details className="advanced"><summary>启动参数与环境变量</summary>
      <Field label="程序覆盖"><input value={value.program || ''} onChange={event => set('program',event.target.value || null)}/></Field>
      <Field label="路由插入位置"><select value={value.route_before || ''} onChange={event => set('route_before',event.target.value || null)}><option value="">使用启动方式配置</option>{config.routes.filter(route => !config.management.generated_routes?.includes(route.id)).map(route => <option key={route.id} value={route.id}>{route.id}</option>)}</select></Field>
      {[['args','附加参数',[]],['env','环境变量',{}],['unset_env','清除继承的环境变量',[]]].map(([key,label,empty]) => <JsonField key={key} label={label} value={value[key] || empty} onChange={next => set(key,next)} onError={bad => setInvalid(old => ({...old,[key]:bad}))} rows={3}/>)}
    </details>
    {error && <div className="integration-error" role="alert">{error}</div>}
  </Modal>;
}
