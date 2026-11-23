import React, { useEffect, useRef, useState } from 'react';
import { QuotaCard } from './quotas.jsx';
import { CommandAccountQuotas } from './command-account-quotas.jsx';
import './provider-accounts.css';

function ProviderAccounts({ id, source, quotaSource, api, ui }) {
  const { Button, Field, Modal } = ui;
  const [report, setReport] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [adding, setAdding] = useState(false);
  const [renaming, setRenaming] = useState(null);
  const running = useRef(false);
  const generation = useRef(0);
  const sourceKey = JSON.stringify(source);
  const pending = report?.session?.state === 'pending';

  async function act(action, details = {}) {
    if (running.current) return;
    const current = generation.current;
    running.current = true;
    setBusy(true);
    setError('');
    try {
      const result = await api('provider-accounts/action', { id, action, ...details });
      if (current !== generation.current) return;
      setReport(result);
      if (action === 'login') {
        setAdding(false);
        if (!result.browser_opened) setError('请点击“打开授权页面”，在浏览器中完成登录。');
      }
      if (action === 'rename') setRenaming(null);
    } catch (failure) {
      if (current === generation.current) setError(failure.message);
    } finally {
      if (current === generation.current) { running.current = false; setBusy(false); }
    }
  }

  useEffect(() => {
    running.current = false;
    setReport(null);
    act('status');
    return () => { generation.current += 1; };
  }, [id, sourceKey]);

  useEffect(() => {
    if (!pending) return;
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible') act('status');
    }, 2000);
    return () => clearInterval(timer);
  }, [pending, id, sourceKey]);

  const accounts = report?.accounts || [];
  let number = 1;
  while (accounts.some(account => account.label === `账号 ${number}`)) number += 1;
  const session = report?.session;
  const notice = <>
    {error && !adding && !renaming && <p className="inline-error" role="alert">{error}</p>}
    {pending && <div className="provider-login" role="status">
      <strong>等待 {source.label} 授权</strong>
      {session.user_code && <p>设备码 <code>{session.user_code}</code></p>}
      <div className="provider-account-actions"><Button disabled={busy} onClick={() => act('login')}>打开授权页面</Button><Button disabled={busy} onClick={() => act('cancel')}>取消本次登录</Button></div>
    </div>}
    {!pending && session?.message && session.state !== 'cancelled' && <p className={session.state === 'error' ? 'inline-error' : 'muted'} role="status">{session.message}</p>}
  </>;
  const add = <Button aria-label={`添加 ${source.label} 账号`} disabled={busy || pending} onClick={() => { setError(''); setAdding(true); }}>添加账号</Button>;
  return <div className="quota-service">
    {accounts.length ? accounts.map((account, index) => <QuotaCard key={account.id} id={source.quota_source} source={quotaSource} api={api} Button={Button} account={account.id}
      title={`${source.label} · ${account.label}`} notice={index === 0 ? notice : undefined} addAccount={index === 0 ? add : undefined}
      identity={<p className="provider-account-identity">{account.email && <span>{account.email}</span>}{account.preferred && <span className="subtle-badge">优先使用</span>}{(account.disabled || account.unavailable) && <span>{account.disabled ? '已停用' : '账号暂不可用'}</span>}</p>}
      actions={<div className="provider-account-actions">
        {!account.preferred && <Button disabled={busy || account.disabled} onClick={() => act('prefer', { account: account.id })}>优先使用</Button>}
        <Button disabled={busy} onClick={() => { setError(''); setRenaming({ account: account.id, label: account.label }); }}>修改名称</Button>
      </div>}/>) : <QuotaCard title={source.label} Button={Button} notice={<>{notice}
        <p className="muted">{report ? '添加账号以连接此服务。' : error ? '请刷新账号状态。' : '正在读取账号…'}</p></>}
        addAccount={add} actions={error ? <div className="provider-account-actions"><Button disabled={busy} onClick={() => act('status')}>刷新账号</Button></div> : undefined}/>}
    {adding && <Modal title={`添加账号 ${number}`} eyebrow={source.label} close={() => setAdding(false)} footer={<><Button onClick={() => setAdding(false)}>取消</Button><Button primary disabled={busy} onClick={() => act('login')}>{busy ? '正在打开…' : '打开授权页面'}</Button></>}>
      <p>在浏览器中登录要添加的账号并完成授权，账号会出现在此页面。</p>
      {error && <p className="inline-error" role="alert">{error}</p>}
    </Modal>}
    {renaming && <Modal title="修改账号名称" eyebrow={source.label} close={() => setRenaming(null)} footer={<><Button onClick={() => setRenaming(null)}>取消</Button><Button primary disabled={busy || !renaming.label.trim()} onClick={() => act('rename', renaming)}>保存名称</Button></>}>
      <Field label="账号名称"><input autoFocus maxLength={80} value={renaming.label} onChange={event => setRenaming({ ...renaming, label: event.target.value })}/></Field>
      {error && <p className="inline-error" role="alert">{error}</p>}
    </Modal>}
  </div>;
}

export function ProviderAccountOverview({ providers, config, revision, api, ui, onManageCommand }) {
  const sources = Object.entries(providers || {});
  const { commands, templates, quota_sources: quotas } = config.management;
  const subscriptions = Object.entries(commands).filter(([, value]) => value.enabled !== false && templates[value.template]?.account_adapter);
  const standalone = Object.entries(quotas || {}).filter(([id]) => !sources.some(([, source]) => source.quota_source === id));
  if (!sources.length && !standalone.length && !subscriptions.length) return null;
  return <section className="quota-overview" aria-label="账号与额度">
    <div className="quota-heading"><h2>账号与额度</h2></div>
    <div className="quota-grid">
      {sources.map(([id, source]) => <ProviderAccounts key={id} id={id} source={source} quotaSource={quotas?.[source.quota_source]} api={api} ui={ui}/>)}
      {subscriptions.map(([name, command]) => <CommandAccountQuotas key={name} name={name} revision={revision} api={api} Button={ui.Button}
        accountMode={(command.options?.['auth-mode'] || templates[command.template].options?.['auth-mode']?.default) === 'account'} onManage={onManageCommand}/>)}
      {standalone.map(([id, source]) => <QuotaCard key={id} id={id} source={source} api={api} Button={ui.Button}/>)}
    </div>
  </section>;
}
