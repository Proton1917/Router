import React, { useState } from 'react';
import './commands.css';
import { customizeCommandProfile } from './model-ownership.js';

export function modelLabel(config, item, profiles = config.profiles) {
  const model = profiles[item.profile]?.standard.model || item.model;
  return item.label || config.management.labels?.[`model:${model}`] || model;
}

export function CommandList({ config, rows, edit, preview, remove, toggle, dirty, ui }) {
  const { Button } = ui;
  return <div className="command-grid">{rows.map(([name, command]) => {
    const template = config.management.templates[command.template];
    const bindings = command.model_bindings?.length ? command.model_bindings : command.profile ? [{id:'default',model:command.model,profile:command.profile,visible:true}] : [];
    return <article className="command-card" key={name}>
      <header><div><button className="command-name" onClick={() => edit(name, command)}><span>›</span>{name}</button><p className="cell-note">{command.label || template?.label}</p></div><button className={`toggle-label ${command.enabled !== false ? 'on' : ''}`} onClick={() => toggle(name, command)}><i/>{command.enabled !== false ? '可启动' : '已停用'}</button></header>
      <div className="command-card-meta"><span>{template?.label}</span><span>{bindings.filter(item => item.visible).length || '原生'}{bindings.length ? ' 个可选模型' : '模型'}</span></div>
      <div className="command-model-summary">{bindings.map(item => {
        const target = config.profiles[item.profile]?.standard;
        return <div key={item.id} className={item.visible ? '' : 'model-hidden'}><span><strong>{modelLabel(config,item)}</strong>{command.model === item.model && <em>默认</em>}{!item.visible && <em>隐藏</em>}</span><small>{config.management.labels?.[`backend:${target?.target}`] || target?.target}{target?.provider ? ` · ${target.provider}` : ''}</small></div>;
      })}{!bindings.length && <p className="muted">{template?.request_path ? '沿用现有模型规则，可在编辑中添加模型列表。' : '使用客户端的账号与模型选择。'}</p>}</div>
      <footer><Button primary onClick={() => edit(name, command)}>编辑命令与模型</Button><button disabled={dirty} onClick={() => preview(name)}>启动预览</button><button className="delete-link" onClick={() => remove(name)}>删除</button></footer>
    </article>;
  })}</div>;
}

export function CommandEditor({ editor, config, close, save, ui }) {
  const { Button, Field, Modal, JsonField } = ui;
  const [name, setName] = useState(editor.id);
  const [editingKey] = useState(() => editor.id || `draft-${crypto.randomUUID()}`);
  const [value, setValue] = useState(structuredClone(editor.value));
  const [profiles, setProfiles] = useState({});
  const [error, setError] = useState('');
  const [invalid, setInvalid] = useState({});
  const template = config.management.templates[value.template];
  const protocol = config.runtime.server.endpoints[template?.request_path];
  const allProfiles = {...config.profiles,...profiles};
  const eligible = Object.entries(allProfiles).filter(([, profile]) => profile.standard.model && (!config.management.backend_protocols?.[profile.standard.target] || config.management.backend_protocols[profile.standard.target].includes(protocol)));
  const backends = Object.keys(config.runtime.backends).filter(key => !config.management.backend_protocols?.[key] || config.management.backend_protocols[key].includes(protocol));
  const bindings = value.model_bindings || [];
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
      if (bindings.length && !bindings.some(item => item.visible)) throw new Error('至少需要一个可见模型。');
      const ids = new Set();
      for (const item of bindings) {
        if (!item.model.trim() || !allProfiles[item.profile]) throw new Error('请完整设置每个模型条目。');
        for (const id of [item.model,...(item.aliases || [])]) { if (ids.has(id.toLowerCase())) throw new Error(`模型标识重复：${id}`); ids.add(id.toLowerCase()); }
      }
      if (bindings.length && value.model && !bindings.some(item => item.visible && item.model === value.model)) throw new Error('默认模型需要在可见列表中。');
      save(name,value,{profiles:Object.fromEntries(Object.entries(profiles).filter(([id]) => bindings.some(item => item.profile === id)))});
    } catch (error) { setError(error.message); }
  }
  return <Modal title={editor.fresh ? '新增启动命令' : `${name} · 命令与模型`} eyebrow="启动命令" close={close} wide footer={<><span className="footer-note">路由应用后生效；菜单在下次启动时读取</span><Button onClick={close}>取消</Button><Button primary onClick={submit}>保存草稿</Button></>}>
    <div className="form-grid"><Field label="命令名称"><input value={name} onChange={event => setName(event.target.value)}/></Field><Field label="显示说明"><input value={value.label || ''} onChange={event => set('label',event.target.value)}/></Field><Field label="启动方式"><select value={value.template} onChange={event => setValue(old => ({...old,template:event.target.value,options:{},mod_profile:config.management.templates[event.target.value]?.supports_mods ? old.mod_profile : undefined}))}>{Object.entries(config.management.templates).map(([id,template]) => <option key={id} value={id}>{template.label}</option>)}</select></Field><Field label="默认启动模型" hint="跟随启动器时，保留此命令的现有默认选择。">{bindings.length ? <select value={value.model || ''} onChange={event => set('model',event.target.value || null)}><option value="">跟随客户端或启动器</option>{bindings.filter(item => item.visible).map(item => <option key={item.id} value={item.model}>{label(item)}</option>)}</select> : <input value={value.model || ''} onChange={event => set('model',event.target.value || null)} placeholder="跟随客户端或启动器"/>}</Field></div>
    {template?.request_path && <section className="command-model-editor"><header><div><h3>可选模型 <span>{bindings.length}</span></h3><p>显示名称留空时跟随后端模型。菜单、切换提示与状态名称使用同一份配置。</p></div><Button onClick={add}>＋ 添加模型</Button></header>
      {!bindings.length && <div className="callout">{value.profile ? `当前绑定：${config.management.labels?.[`profile:${value.profile}`] || value.profile}。` : '当前沿用现有路由规则。'}<Button onClick={() => { if (value.profile) setValue(old => ({...old,profile:null,match_all_models:false,model_bindings:[{id:'primary',profile:old.profile,model:old.model || config.management.profile_models?.[old.profile]?.[protocol] || allProfiles[old.profile].standard.model,label:'',description:'',visible:true,aliases:[],route_ids:[]}]})); else add(); }}>管理模型列表</Button></div>}
      {bindings.map((item,index) => { const profile = allProfiles[item.profile]; const target = profile?.standard; return <article className="command-model-item" key={item.id}>
        <header><div><span className="model-position">{String(index+1).padStart(2,'0')}</span><strong>{label(item)}</strong><small>{config.management.labels?.[`backend:${target?.target}`] || target?.target}</small></div><div className="model-item-actions"><button aria-label={`上移模型 ${index+1}`} disabled={index === 0} onClick={() => { const next = [...bindings]; [next[index-1],next[index]] = [next[index],next[index-1]]; set('model_bindings',next); }}>↑</button><button aria-label={`下移模型 ${index+1}`} disabled={index === bindings.length-1} onClick={() => { const next = [...bindings]; [next[index+1],next[index]] = [next[index],next[index+1]]; set('model_bindings',next); }}>↓</button><button className="delete-link" aria-label={`删除模型 ${index+1}`} onClick={() => setValue(old => ({...old,model:old.model === item.model ? null : old.model,model_bindings:old.model_bindings.filter(row => row.id !== item.id)}))}>删除</button></div></header>
        <div className="form-grid"><Field label={`模型 ${index+1} 的 API 后端`}><select value={target?.target || ''} onChange={event => { const found = eligible.find(([,profile]) => profile.standard.target === event.target.value); if (found) selectProfile(index,found[0]); else customize(index,next => { next.standard = {target:event.target.value,model:''}; delete next.fast; }); }}>{backends.map(id => <option key={id} value={id}>{config.management.labels?.[`backend:${id}`] || id}</option>)}</select></Field><Field label={`模型 ${index+1} 的模型配置`}><select value={item.profile} onChange={event => selectProfile(index,event.target.value)}>{eligible.filter(([,profile]) => profile.standard.target === target?.target).map(([id,profile]) => <option key={id} value={id}>{config.management.labels?.[`profile:${id}`] || profile.standard.model}</option>)}</select></Field><Field label={`模型 ${index+1} 的显示名称`} hint="留空时自动跟随当前 API 模型。"><input value={item.label || ''} placeholder={modelLabel(config,{...item,label:''},allProfiles)} onChange={event => update(index,{label:event.target.value})}/></Field><Field label={`模型 ${index+1} 的菜单说明`}><input value={item.description || ''} placeholder="自动显示模型与服务来源" onChange={event => update(index,{description:event.target.value})}/></Field></div>
        <div className="command-model-options"><label className="check-field"><input type="checkbox" checked={item.visible} onChange={event => update(index,{visible:event.target.checked})}/>在客户端菜单显示</label><span>Fast：{profile?.fast ? [profile.fast.model || item.model,config.management.labels?.[`backend:${profile.fast.target}`] || profile.fast.target,profile.fast.provider].filter(Boolean).join(' · ') : '沿用标准目标'}</span></div>
        <details className="advanced"><summary>自定义标准与 Fast 目标</summary><p className="muted">独享配置直接更新；共享配置会生成此入口使用的独立副本。</p>{destination(index,'standard')}<label className="check-field"><input type="checkbox" checked={!!profile?.fast} onChange={event => customize(index,next => { if (event.target.checked) next.fast = structuredClone(next.standard); else delete next.fast; })}/>单独配置 Fast 目标</label>{profile?.fast && destination(index,'fast')}</details>
        <details className="advanced"><summary>客户端标识与兼容路由</summary><Field label={`模型 ${index+1} 的客户端标识`}><input value={item.model} onChange={event => update(index,{model:event.target.value})}/></Field><Field label={`模型 ${index+1} 的兼容名称`} hint="每行一个，保留已有会话所用的名称。"><textarea rows={2} value={(item.aliases || []).join('\n')} onChange={event => update(index,{aliases:event.target.value.split('\n').filter(Boolean)})}/></Field>{item.route_ids?.length > 0 && <p className="muted">同时更新 {item.route_ids.length} 条已关联的兼容路由。</p>}</details>
      </article>; })}
    </section>}
    {Object.entries(template?.options || {}).map(([key,option]) => <Field key={key} label={option.label}><select value={value.options?.[key] || option.default} onChange={event => setValue(old => ({...old,options:{...old.options,[key]:event.target.value}}))}>{Object.entries(option.choices).map(([id,choice]) => <option key={id} value={id}>{choice.label}</option>)}</select></Field>)}
    {template?.supports_mods && config.management.mods && <Field label="Mod 组合"><select value={value.mod_profile || ''} onChange={event => set('mod_profile',event.target.value || undefined)}><option value="">继承客户端设置</option>{Object.entries(config.management.mods.profiles).map(([id,item]) => <option key={id} value={id}>{item.label}</option>)}</select></Field>}
    <details className="advanced"><summary>启动参数与环境变量</summary>
      <Field label="程序覆盖"><input value={value.program || ''} onChange={event => set('program',event.target.value || null)}/></Field>
      <Field label="路由插入位置"><select value={value.route_before || ''} onChange={event => set('route_before',event.target.value || null)}><option value="">使用启动方式配置</option>{config.routes.filter(route => !config.management.generated_routes?.includes(route.id)).map(route => <option key={route.id} value={route.id}>{route.id}</option>)}</select></Field>
      {[['args','附加参数',[]],['env','环境变量',{}],['unset_env','清除继承的环境变量',[]]].map(([key,label,empty]) => <JsonField key={key} label={label} value={value[key] || empty} onChange={next => set(key,next)} onError={bad => setInvalid(old => ({...old,[key]:bad}))} rows={3}/>)}
    </details>
    {error && <div className="integration-error" role="alert">{error}</div>}
  </Modal>;
}
