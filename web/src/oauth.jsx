import React, { useEffect, useState } from 'react';

export function OAuthFields({ id, value, saved, set, api, Field, Button, JsonField, onError }) {
  const [status, setStatus] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [secret, setSecret] = useState('');
  const [scopeText, setScopeText] = useState((value.scopes || []).join(' '));
  const [confirmLogout, setConfirmLogout] = useState(false);
  const session = status?.session;
  const pending = session?.state === 'pending';
  const edit = (key, next) => set(`auth.oauth.${key}`, next);
  async function readStatus() {
    const next = await api('oauth/action', { id, action: 'status' });
    setStatus(next);
    return next;
  }
  useEffect(() => {
    if (!saved) { setStatus(null); return; }
    let active = true;
    api('oauth/action', { id, action: 'status' }).then(next => { if (active) setStatus(next); }).catch(e => { if (active) setError(e.message); });
    return () => { active = false; };
  }, [id, saved]);
  useEffect(() => {
    if (!pending || !saved) return;
    let active = true;
    let timer;
    const poll = async () => {
      try {
        const next = await api('oauth/action', { id, action: 'status' });
        if (active) { setStatus(next); if (next.session?.state === 'pending') timer = setTimeout(poll, 1500); }
      } catch (e) { if (active) setError(e.message); }
    };
    timer = setTimeout(poll, 1500);
    return () => { active = false; clearTimeout(timer); };
  }, [id, pending, saved]);
  async function act(action) {
    setBusy(true); setError('');
    try {
      const result = await api('oauth/action', { id, action });
      if (action === 'login' && !result.browser_opened) setError('浏览器未能自动打开，请使用下方授权链接。');
      if (action === 'status') setStatus(result); else await readStatus();
      setConfirmLogout(false);
    } catch (e) { setError(e.message); }
    finally { setBusy(false); }
  }
  async function saveSecret() {
    setBusy(true); setError('');
    try {
      const result = await api('credentials', { id: `${id}-oauth-client-secret`, value: secret });
      edit('client_secret_file', result.path); setSecret('');
    } catch (e) { setError(e.message); }
    finally { setBusy(false); }
  }
  const stateLabel = !saved ? '配置待应用' : pending ? '等待浏览器授权' : session?.state === 'error' ? '授权失败' : session?.state === 'expired' ? '授权已超时' : status?.credential?.authenticated ? '已登录' : status?.credential?.can_refresh ? '访问令牌已到期，可自动刷新' : '尚未登录';
  return <section className="form-section oauth-settings">
    <h3>OAuth 登录</h3>
    <p className="muted">填写服务商提供的 OAuth 参数。Router 负责登录、保存凭据和自动刷新。</p>
    <div className="form-grid">
      <Field label="服务商名称"><input value={value.provider || ''} onChange={e => edit('provider', e.target.value)} placeholder="自由填写，例如公司网关"/></Field>
      <Field label="授权方式"><select value={value.flow} onChange={e => edit('flow', e.target.value)}><option value="device_code">设备码授权</option><option value="authorization_code">浏览器授权 · PKCE</option></select></Field>
      <Field label="Client ID"><input value={value.client_id || ''} onChange={e => edit('client_id', e.target.value)} autoComplete="off"/></Field>
      <Field label="权限范围（空格分隔）"><input value={scopeText} onChange={e => { setScopeText(e.target.value); edit('scopes', e.target.value.split(/\s+/).filter(Boolean)); }}/></Field>
      <Field label="令牌地址" wide><input type="url" value={value.token_url || ''} onChange={e => edit('token_url', e.target.value)} placeholder="https://服务商/oauth/token"/></Field>
      {value.flow === 'device_code' ? <Field label="设备码授权地址" wide><input type="url" value={value.device_authorization_url || ''} onChange={e => edit('device_authorization_url', e.target.value)} placeholder="https://服务商/oauth/device/code"/></Field> : <>
        <Field label="浏览器授权地址" wide><input type="url" value={value.authorization_url || ''} onChange={e => edit('authorization_url', e.target.value)} placeholder="https://服务商/oauth/authorize"/></Field>
        <Field label="本机回调地址" wide hint="须符合服务商注册的回调地址；端口 0 由 Router 自动分配。"><input value={value.redirect_uri || ''} onChange={e => edit('redirect_uri', e.target.value)} placeholder="http://127.0.0.1:0/oauth/callback"/></Field>
      </>}
      <Field label="OAuth 凭据文件" wide hint="保存到 Git 工作区之外。多个后端可填写同一路径，共享同一授权；不同账号使用不同路径。"><input value={value.credential_file || ''} onChange={e => edit('credential_file', e.target.value)}/></Field>
    </div>
    <details className="advanced"><summary>Client Secret 与附加授权参数</summary>
      <div className="form-grid">
        <Field label="Client Secret 文件（可选）"><input value={value.client_secret_file || ''} onChange={e => edit('client_secret_file', e.target.value || undefined)}/></Field>
        <Field label="客户端认证方式"><select value={value.client_auth_basic ? 'basic' : 'body'} onChange={e => edit('client_auth_basic', e.target.value === 'basic')}><option value="body">请求正文</option><option value="basic">HTTP Basic</option></select></Field>
        <Field label="授权等待时间（秒）"><input type="number" min="60" max="1800" value={value.login_timeout_seconds ?? 600} onChange={e => edit('login_timeout_seconds', Number(e.target.value))}/></Field>
        <Field label="保存新的 Client Secret"><input type="password" autoComplete="new-password" value={secret} onChange={e => setSecret(e.target.value)}/></Field>
      </div>
      <Button disabled={!secret || !id || busy} onClick={saveSecret}>保存 Client Secret</Button>
      <JsonField label="授权附加参数" value={value.authorization_params || {}} onChange={next => edit('authorization_params', next)} onError={onError('oauth-authorize')} rows={3} hint="例如 audience、resource；不要填写令牌或 Client Secret。"/>
      <JsonField label="令牌附加参数" value={value.token_params || {}} onChange={next => edit('token_params', next)} onError={onError('oauth-token')} rows={3}/>
    </details>
    <div className="oauth-status" role="status">
      <strong>{stateLabel}</strong>
      {!saved && <p>保存草稿并应用修改后，再打开此后端，点击登录。</p>}
      {saved && status?.credential?.expires_at && <p>访问令牌有效至 {new Date(status.credential.expires_at * 1000).toLocaleString()}。{status.credential.can_refresh && '调用时由 Router 检查有效期并自动刷新。'}</p>}
      {saved && pending && <><p>在授权页完成登录后，此处会自动更新。{session.user_code && <>设备码：<strong className="mono">{session.user_code}</strong></>}</p><a href={session.url} target="_blank" rel="noreferrer">打开授权页面</a><p>本次授权有效至 {new Date(session.expires_at * 1000).toLocaleTimeString()}。</p></>}
      {saved && session?.message && <p className="inline-error">{session.message}</p>}
      {error && <p className="inline-error" role="alert">{error}</p>}
      <div className="oauth-actions">
        <Button primary disabled={!saved || busy || pending} onClick={() => act('login')}>{busy ? '正在处理…' : status?.credential?.authenticated ? '重新登录' : '登录并打开授权页'}</Button>
        {pending && <Button disabled={busy} onClick={() => act('cancel')}>取消本次登录</Button>}
        {saved && status?.credential?.can_refresh && <Button disabled={busy || pending} onClick={() => act('refresh')}>刷新授权</Button>}
        {saved && (status?.credential?.authenticated || status?.credential?.can_refresh) && <Button disabled={busy} onClick={() => setConfirmLogout(true)}>退出本机账号</Button>}
        {saved && <Button disabled={busy} onClick={() => act('status')}>更新状态</Button>}
      </div>
      {confirmLogout && <div><p>将删除此凭据文件，所有共享此文件的后端都需要重新登录。服务商网页会话保持原样。</p><Button danger onClick={() => act('logout')} disabled={busy}>确认退出</Button><Button onClick={() => setConfirmLogout(false)}>取消</Button></div>}
    </div>
  </section>;
}
