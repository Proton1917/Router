import React, { useEffect, useState } from 'react';

export function SubscriptionAccounts({ name, value, setValue, template, appliedConfig, revision, configPath, api, ui }) {
  const { Button, Field } = ui;
  const encoded = value.env?.ROUTER_SUBSCRIPTION_ACCOUNTS ?? template.env.ROUTER_SUBSCRIPTION_ACCOUNTS;
  let accounts;
  try { const parsed = JSON.parse(encoded); accounts = Array.isArray(parsed) ? parsed : null; } catch { accounts = null; }
  const [report, setReport] = useState(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState(null);
  const [logout, setLogout] = useState(null);
  const [session, setSession] = useState('');
  const [switching, setSwitching] = useState(null);
  const [message, setMessage] = useState('');
  const activeSessions = (report?.sessions || []).filter(item => item.run_id && item.session_id && ['running','switching','exhausted'].includes(item.state));
  const applied = appliedConfig?.management.commands[name];
  const saved = applied?.template === value.template && JSON.stringify(applied.env || {}) === JSON.stringify(value.env || {});
  async function action(action, account) {
    setBusy(true); setError('');
    try {
      const result = await api('accounts/action', {name,action,account,revision,session:action === 'switch' ? session : undefined});
      if (action === 'status') {
        setReport(result);
        const active = result.sessions.filter(item => item.run_id && item.session_id && ['running','switching','exhausted'].includes(item.state));
        setSession(previous => active.some(item => item.run_id === previous) ? previous : active[0]?.run_id || '');
        if (pending && result.accounts.some(row => row.id === pending.id && row.authenticated)) setPending(null);
        if (switching && !active.some(item => item.run_id === switching.session && (item.account !== switching.account || item.state === 'switching'))) setSwitching(null);
      }
      if (action === 'login') setPending({id:account,started:Date.now()});
      if (action === 'switch') { setMessage(result.message); setSwitching({account,session,started:Date.now()}); }
      if (action === 'logout') { setLogout(null); setReport(null); }
    } catch (failure) { setError(failure.message); setPending(null); }
    finally { setBusy(false); }
  }
  useEffect(() => { if (saved && accounts?.length) action('status'); else setReport(null); }, [saved,name,encoded]);
  useEffect(() => {
    if (!pending || !saved) return;
    const timer = setTimeout(() => { if (Date.now() - pending.started < 600000) action('status'); else setPending(null); }, 3000);
    return () => clearTimeout(timer);
  }, [pending,saved,report]);
  useEffect(() => {
    if (!switching || !saved) return;
    const timer = setTimeout(() => { if (Date.now() - switching.started < 600000) action('status'); else setSwitching(null); }, 2000);
    return () => clearTimeout(timer);
  }, [switching,saved,report]);
  function update(next) { setValue(old => ({...old,env:{...old.env,ROUTER_SUBSCRIPTION_ACCOUNTS:JSON.stringify(next)}})); }
  function add() {
    const id = `account-${crypto.randomUUID().slice(0,8)}`;
    const directory = configPath.slice(0,configPath.lastIndexOf('/'));
    update([...(accounts || []),{id,label:accounts?.length ? `备用账号 ${accounts.length}` : '主账号',directory:`${directory}/credentials/subscriptions/${name}/${id}`}]);
  }
  return <section className="form-section subscription-accounts">
    <h3>官方订阅主备账号</h3>
    <p className="muted">平时固定使用主账号。官方客户端报告额度耗尽后，自动重启到备用账号并恢复当前对话；一轮切换中每个账号尝试一次。模型和权限沿用官方客户端的会话恢复规则。</p>
    {activeSessions.length > 0 && <Field label="要切换账号的会话" hint="手动切换在当前回复结束后执行，并恢复这段对话。"><select value={session} onChange={event => setSession(event.target.value)}>{activeSessions.map(item => <option key={item.run_id} value={item.run_id}>{item.session_id.slice(0,8)} · {accounts?.find(account => account.id === item.account)?.label || item.account}{item.cwd ? ` · ${item.cwd}` : ''}</option>)}</select></Field>}
    {accounts === null && <p role="alert" className="inline-error">账号列表格式错误，请在完整 JSON 中修正配置。</p>}
    {(accounts || []).map((account,index) => {
      const status = report?.accounts.find(row => row.id === account.id);
      return <article className="command-model-item" key={account.id}>
        <header><strong>{index === 0 ? '主账号' : `备用账号 ${index}`}</strong><span>{pending?.id === account.id ? '等待官方授权' : status?.authenticated ? '已登录' : status ? '待登录' : '等待查询'}</span></header>
        <div className="form-grid">
          <Field label={`账号 ${index+1} 名称`}><input value={account.label} onChange={event => update(accounts.map((row,position) => position === index ? {...row,label:event.target.value} : row))}/></Field>
          <Field label={`账号 ${index+1} 配置目录`} hint="每个账号使用独立目录；沿用已有账号时填写该账号的 Claude Code 配置目录。"><input value={account.directory} onChange={event => update(accounts.map((row,position) => position === index ? {...row,directory:event.target.value} : row))}/></Field>
        </div>
        {status?.email && <p className="muted">{status.email}{status.subscription ? ` · ${status.subscription}` : ''}</p>}
        <div className="subscription-actions">
          <Button primary disabled={!saved || busy || pending?.id === account.id} onClick={() => action('login',account.id)}>官方登录</Button>
          {activeSessions.length > 0 && <Button disabled={!saved || busy || !!switching || !status?.authenticated || activeSessions.find(item => item.run_id === session)?.account === account.id} onClick={() => action('switch',account.id)}>切换到此账号</Button>}
          {index > 0 && <Button onClick={() => update([account,...accounts.filter(row => row.id !== account.id)])}>设为主账号</Button>}
          <Button onClick={() => update(accounts.filter(row => row.id !== account.id))}>移出账号列表</Button>
          {status?.authenticated && <Button disabled={!saved || busy} onClick={() => setLogout(account.id)}>退出登录</Button>}
        </div>
        {logout === account.id && <div className="callout"><p>将由官方客户端退出此账号。使用同一配置目录的其他会话也会受影响。</p><Button danger onClick={() => action('logout',account.id)}>确认退出</Button><Button onClick={() => setLogout(null)}>取消</Button></div>}
      </article>;
    })}
    <div className="subscription-actions"><Button onClick={add}>添加账号</Button><Button disabled={!saved || busy || !accounts?.length} onClick={() => action('status')}>刷新登录与切换状态</Button></div>
    {!saved && <p className="muted">保存草稿并应用修改后，可执行官方登录。</p>}
    {pending && <p className="callout">在官方客户端打开的浏览器中完成登录，页面会自动更新账号状态。</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    {message && <p role="status" className="callout">{message}</p>}
    {report?.sessions?.length > 0 && <details><summary>最近会话与账号切换</summary>{report.sessions.map(session => <p className="muted" key={session.supervisor_pid}>{session.account} · {({running:'运行中',switching:'正在切换',exhausted:'等待额度恢复',finished:'已结束',interrupted:'已中断'})[session.state] || session.state} · 切换 {session.switches.length} 次{session.switches.map(change => `；${change.from} → ${change.to}`).join('')}</p>)}</details>}
    <p className="muted">主备切换入口用于本机交互终端。原生恢复保留对话与工具结果；后台 Bash 和监控任务在切换后需重新建立。</p>
  </section>;
}
