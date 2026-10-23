import React, { useEffect, useRef, useState } from 'react';
import './quotas.css';

function timestamp(value) {
  if (!value) return '上游未提供';
  const date = new Date(typeof value === 'number' ? value * 1000 : value);
  return Number.isNaN(date.getTime()) ? '上游时间格式无效' : date.toLocaleString();
}

function QuotaCard({ id, source, api, Button }) {
  const [result, setResult] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const current = useRef(null);
  const sourceKey = JSON.stringify(source);
  useEffect(() => {
    const session = {};
    current.current = session;
    let timer;
    async function read(refresh = false) {
      if (session.pending || current.current !== session) return;
      session.pending = true;
      setBusy(true);
      try {
        const next = await api('quotas/read', { id, refresh });
        if (current.current === session) { setResult(next); setError(''); }
      } catch (failure) {
        if (current.current === session) setError(failure.message);
      } finally {
        session.pending = false;
        if (current.current === session) setBusy(false);
      }
    }
    current.current.read = read;
    setResult(null);
    setError('');
    read();
    const tick = () => {
      if (document.visibilityState === 'visible') read();
      timer = setTimeout(tick, Math.max(15, source.ttl_seconds || 60) * 1000);
    };
    const visible = () => { if (document.visibilityState === 'visible') read(); };
    timer = setTimeout(tick, Math.max(15, source.ttl_seconds || 60) * 1000);
    document.addEventListener('visibilitychange', visible);
    return () => { current.current = null; clearTimeout(timer); document.removeEventListener('visibilitychange', visible); };
  }, [id, sourceKey]);
  const failure = error || result?.error;
  return <article className="quota-card" aria-label={`${source.label} 配额`} aria-busy={busy}>
    <header><div><h3>{source.label}</h3><p>{source.backends.join(' · ')}</p></div><Button disabled={busy} onClick={() => current.current?.read(true)}>{busy ? '查询中…' : '刷新配额'}</Button></header>
    {failure && <p className="inline-error" role="alert">{failure}{result?.data && '；下方保留上次成功查询的数据。'}</p>}
    {!result?.data && !failure && <p className="muted" role="status">正在读取账号配额…</p>}
    {result?.data?.groups.map((group, groupIndex) => <section className="quota-group" key={`${group.label}-${groupIndex}`}>
      <h4>{group.label}</h4>
      {group.windows.map((window, index) => <div className={`quota-window ${window.disabled ? 'quota-disabled' : ''}`} key={index}>
        <div className="quota-caption"><span>{window.label}</span><strong>{window.remaining_percent == null ? '未提供' : `剩余 ${window.remaining_percent.toLocaleString(undefined, { maximumFractionDigits: 2 })}%`}</strong></div>
        {window.remaining_percent != null && <progress max="100" value={window.remaining_percent} aria-label={`${group.label} ${window.label}剩余百分比`}/>}
        <small>{window.disabled && '当前不可用 · '}重置时间：{timestamp(window.reset_at)}</small>
      </div>)}
    </section>)}
    {!!result?.data?.details.length && <dl className="quota-details">{result.data.details.map((detail, index) => <div key={index}><dt>{detail.label}</dt><dd>{detail.value}</dd></div>)}</dl>}
    <footer>{result?.updated_at ? `更新于 ${timestamp(result.updated_at)}${failure ? ' · 数据待更新' : ''}` : '等待首次成功查询'}</footer>
  </article>;
}

export function QuotaOverview({ config, api, Button }) {
  const sources = Object.entries(config.management.quota_sources || {});
  if (!sources.length) return null;
  return <section className="quota-overview" aria-label="账号配额">
    <div className="quota-heading"><h2>账号配额</h2><p>页面可见时按配置间隔刷新。时间按本机时区显示。</p></div>
    <div className="quota-grid">{sources.map(([id, source]) => <QuotaCard key={id} id={id} source={source} api={api} Button={Button}/>)}</div>
  </section>;
}
