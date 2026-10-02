import React, { useEffect, useState } from 'react';

export function IntegrationWorkspace({ config, revision, dirty, api, onEdit, onCommand, ui }) {
  const { Button } = ui;
  const [reports, setReports] = useState({});
  const [working, setWorking] = useState({});
  const integrations = config.management.integrations || {};
  async function perform(id, action) {
    setWorking(old => ({ ...old, [id]: true }));
    try {
      const result = await api('integrations/action', { id, action, revision });
      setReports(old => ({ ...old, [id]: result }));
    } catch (error) {
      setReports(old => ({ ...old, [id]: { ok: false, error: error.message } }));
    } finally { setWorking(old => ({ ...old, [id]: false })); }
  }
  useEffect(() => { Object.keys(integrations).forEach(id => perform(id, 'inspect')); }, [integrations]);
  return <div className="integration-grid">
    {Object.entries(integrations).map(([id, spec], index) => {
      const report = reports[id];
      const visible = (spec.slots || []).filter(slot => slot.visible);
      const modelRow = slot => {
        const profile = config.profiles[slot.profile];
        return <div key={slot.id}><span>{slot.label || config.management.labels?.[`profile:${slot.profile}`] || profile?.standard.model}</span><small>{config.management.labels?.[`backend:${profile?.standard.target}`] || profile?.standard.target}{profile?.standard.provider ? ` · ${profile.standard.provider}` : ''}</small></div>;
      };
      return <article className="integration-card" key={id}>
        <header><span className="integration-number">{String(index + 1).padStart(2, '0')}</span><span className={`status-pill ${report?.configured ? '' : 'offline'}`}><i/>{working[id] ? '正在检查' : report?.configured ? '配置已接入' : report?.error ? '需要处理' : '待接入'}</span></header>
        <h2>{spec.label}</h2><p className="integration-description">{spec.description}</p>
        <div className="integration-endpoint"><span>{spec.require_https ? 'HTTPS 接入地址' : '接入地址'}</span><code>{spec.gateway_url}</code></div>
        {spec.commands?.length > 0 && <div className="integration-commands">{spec.commands.map(name => <button key={name} onClick={() => onCommand(name)}><strong>{name}</strong><span>配置命令 →</span></button>)}</div>}
        {visible.length > 0 && <div className="integration-models">{visible.slice(0, 3).map(modelRow)}{visible.length > 3 && <details className="integration-more"><summary>查看其余 {visible.length - 3} 个模型入口</summary><div className="integration-models">{visible.slice(3).map(modelRow)}</div></details>}</div>}
        {report?.applications && <div className="integration-checks">{report.applications.map(app => <span key={app.label} className={app.error ? 'integration-error' : ''}>{app.configured ? '✓' : '○'} {app.label} · {app.error || (app.configured ? '地址已配置' : app.detected ? '地址待同步' : '尚未完成插件接入')}</span>)}</div>}
        {report?.checks && <div className="integration-checks">{report.checks.map(check => <span key={check.label}>✓ {check.label}{check.count !== undefined ? ` · ${check.count} 个模型` : ''}</span>)}</div>}
        {report?.error && <div className="integration-error" role="alert">{report.error}</div>}
        {report?.message && <p className="integration-report">{report.message}</p>}
        <footer><Button primary onClick={() => onEdit(id)}>配置接入</Button><Button disabled={dirty || working[id]} onClick={() => perform(id, 'sync')}>应用已保存配置</Button><button disabled={dirty || working[id]} onClick={() => perform(id, 'check')}>检查连接</button></footer>
      </article>;
    })}
    {!Object.keys(integrations).length && <div className="empty"><h3>尚未配置客户端接入</h3><p>管理配置中的 integrations 声明客户端入口与同步程序。</p></div>}
  </div>;
}

export function IntegrationEditor({ id, config, close, save, onCommand, ui }) {
  const { Button, Field, Modal } = ui;
  const [spec, setSpec] = useState(structuredClone(config.management.integrations[id]));
  const [error, setError] = useState('');
  const profiles = Object.entries(config.profiles).filter(([key, profile]) => {
    const protocols = config.management.backend_protocols?.[profile.standard.target];
    return profile.standard.model && (!protocols || protocols.includes(spec.protocol));
  });
  const backends = Object.entries(config.runtime.backends).filter(([key]) => profiles.some(([, profile]) => profile.standard.target === key));
  const label = key => config.management.labels?.[`profile:${key}`] || config.profiles[key]?.standard.model || key;
  function slot(index, changes) { setSpec(old => ({ ...old, slots: old.slots.map((value, at) => at === index ? { ...value, ...changes } : value) })); }
  function submit() {
    try {
      const url = new URL(spec.gateway_url);
      if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new Error('请填写不含凭据的 HTTP 或 HTTPS 接入地址。');
      if (spec.require_https && url.protocol !== 'https:') throw new Error('此客户端需要 HTTPS 接入地址。');
      if (spec.slots.length && !spec.slots.some(item => item.visible)) throw new Error('至少需要显示一个模型入口。');
      if (spec.slots.some(item => !profiles.some(([id]) => id === item.profile))) throw new Error('请为每个入口选择支持客户端协议的模型。');
      save(spec);
    } catch (error) { setError(error.message); }
  }
  return <Modal title={`${spec.label} · 接入设置`} eyebrow="客户端接入" close={close} wide footer={<><span className="footer-note">保存后通过“应用修改”同步客户端</span><Button onClick={close}>取消</Button><Button primary onClick={submit}>保存接入草稿</Button></>}>
    <p className="integration-editor-intro">{spec.description}</p>
    <Field label="客户端接入地址" hint={spec.require_https ? '使用可信的 HTTPS 网关，检查连接时同时验证证书。' : '客户端将向此网关发送请求。'}><input value={spec.gateway_url} readOnly={spec.commands.length > 0} onChange={e => setSpec(old => ({ ...old, gateway_url: e.target.value }))}/></Field>
    {spec.commands.length > 0 && <section className="form-section"><h3>选择终端入口</h3><div className="integration-commands">{spec.commands.map(name => <button key={name} onClick={() => onCommand(name)}><strong>{name}</strong><span>模型、启动方式与参数 →</span></button>)}</div></section>}
    {spec.slots.map((item, index) => {
      const backend = config.profiles[item.profile]?.standard.target || '';
      return <section className="integration-slot" key={item.id}>
        <header><h3>模型入口 {index + 1}</h3><label className="check-field"><input type="checkbox" checked={item.visible} onChange={e => slot(index, { visible: e.target.checked })}/>在客户端显示</label></header>
        <div className="form-grid"><Field label={`入口 ${index + 1} 的 API 服务`}><select value={backend} onChange={e => slot(index, { profile: profiles.find(([, profile]) => profile.standard.target === e.target.value)?.[0] || '' })}>{backends.map(([key]) => <option key={key} value={key}>{config.management.labels?.[`backend:${key}`] || key}</option>)}</select></Field>
          <Field label={`入口 ${index + 1} 的模型`}><select value={item.profile} onChange={e => slot(index, { profile: e.target.value })}>{profiles.filter(([, profile]) => profile.standard.target === backend).map(([key]) => <option key={key} value={key}>{label(key)}</option>)}</select></Field>
          <Field label={`入口 ${index + 1} 的显示名称`} hint="留空时跟随目标模型与后端名称。" wide><input value={item.label} placeholder={label(item.profile)} onChange={e => slot(index, { label: e.target.value })}/></Field>
        </div>
        <details className="advanced"><summary>客户端兼容标识与上下文</summary><p className="muted">兼容标识保留为 <code>{item.model}</code>，后端模型由上方选择决定。</p><div className="form-grid">{['context_window', 'max_output_tokens'].filter(key => item.metadata?.[key] !== undefined).map(key => <Field key={key} label={key === 'context_window' ? '上下文窗口' : '最大输出 tokens'}><input type="number" min="1" value={item.metadata[key]} onChange={e => slot(index, { metadata: { ...item.metadata, [key]: Number(e.target.value) } })}/></Field>)}</div></details>
      </section>;
    })}
    {spec.reload_hint && <div className="callout">{spec.reload_hint}</div>}
    {error && <div className="integration-error" role="alert">{error}</div>}
  </Modal>;
}
