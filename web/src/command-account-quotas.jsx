import React, { useEffect, useRef, useState } from 'react';
import { QuotaDisplay } from './quotas.jsx';

export function CommandAccountQuotas({ name, accountMode, revision, api, Button, onManage }) {
  const [report, setReport] = useState(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState('');
  const refresh = useRef(null);

  useEffect(() => {
    let active = true;
    let pending = false;
    async function read() {
      if (pending) return;
      pending = true;
      setBusy(true);
      try {
        const next = await api('accounts/action', { name, action: 'overview' });
        if (active) { setReport(next); setError(''); }
      } catch (failure) {
        if (active) setError(failure.message);
      } finally {
        pending = false;
        if (active) setBusy(false);
      }
    }
    refresh.current = read;
    read();
    const timer = setInterval(() => { if (document.visibilityState === 'visible') read(); }, 60000);
    return () => { active = false; refresh.current = null; clearInterval(timer); };
  }, [name, revision]);

  const accounts = report?.accounts.filter(account => accountMode || account.authenticated) || [];
  const provider = report?.provider;
  const manage = <Button onClick={() => onManage(name)}>管理 {name} 账号</Button>;
  if (!accounts.length) {
    if (!accountMode && !error) return null;
    return <QuotaDisplay name={provider || `${name} 官方账号`} busy={busy} error={error}
      controls={manage} notice={!error && <p className="muted">{report ? '在启动命令中添加并登录账号。' : '正在读取账号状态…'}</p>}/>;
  }
  return <div className="quota-service">{accounts.map((account, index) => <QuotaDisplay key={account.id} name={`${provider} · ${account.label}`}
    identity={<p className="provider-account-identity">
      <span>{name}{account.subscription && ` · ${account.subscription === 'pro' ? 'Pro' : account.subscription}`}</span>
      {account.email && <span>{account.email}</span>}
      {account.preferred && <span className="subtle-badge">优先使用</span>}
      {!account.authenticated && <span>待登录</span>}
    </p>}
    controls={<>{account.authenticated && <Button disabled={busy} onClick={() => refresh.current?.()}>{busy ? '读取中…' : '读取额度'}</Button>}{index === 0 && manage}
      {index > 0 && !account.authenticated && <Button onClick={() => onManage(name)}>管理账号</Button>}</>}
    busy={busy} result={account.quota} error={index === 0 ? error : ''} showQuota={account.authenticated} recorded
    emptyMessage="等待会话报告额度。"/>)}</div>;
}
