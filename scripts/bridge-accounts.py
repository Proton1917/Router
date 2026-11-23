import fcntl
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

from bridge_client import Bridge


def save(path, value):
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            os.fchmod(output.fileno(), 0o600)
            json.dump(value, output, ensure_ascii=False)
            output.flush()
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def public_accounts(rows, state):
    labels = state.setdefault('labels', {})
    for row in sorted(rows, key=lambda item: (item.get('created_at', ''), item['id'])):
        if row['id'] not in labels:
            number = 1
            while f'账号 {number}' in labels.values():
                number += 1
            labels[row['id']] = f'账号 {number}'
    preferred = next((row['id'] for row in sorted(rows, key=lambda row: (-int(row.get('priority', 0)), row['id']))
                      if not row.get('disabled', False)), None)
    return [{'id': row['id'], 'label': labels[row['id']], 'email': row.get('email', ''),
             'disabled': row.get('disabled', False), 'unavailable': row.get('unavailable', False),
             'preferred': row['id'] == preferred}
            for row in rows]


def poll(bridge, state):
    session = state.get('session')
    if not session or session['state'] != 'pending':
        return
    result = bridge.call('GET', 'oauth/status', params={'state': session['id']})
    if result['status'] == 'ok':
        session['state'] = 'complete'
        session['message'] = 'Google 授权已完成，账号列表已更新。'
    elif result['status'] == 'error':
        session['state'] = 'error'
        session['message'] = '授权失败或已过期，请重新登录。'
    elif time.time() >= session['expires_at']:
        bridge.call('DELETE', 'oauth/session', params={'state': session['id']})
        session['state'] = 'expired'
        session['message'] = '本次授权已到期，可以重新登录。'
    if session['state'] != 'pending':
        session.pop('url', None)


def perform(bridge, request, state):
    action = request['action']
    poll(bridge, state)
    rows = bridge.accounts()
    accounts = public_accounts(rows, state)
    if action == 'login':
        if state.get('session', {}).get('state') != 'pending':
            preferred = next((row for row in accounts if row['preferred']), None)
            if preferred:
                account = next(row for row in rows if row['id'] == preferred['id'])
                if int(account.get('priority', 0)) <= 0:
                    bridge.call('PATCH', 'credentials/fields', json={'name': account['name'], 'priority': 1})
                    account['priority'] = 1
            result = bridge.call('GET', 'oauth/auth-url', params={'provider': bridge.settings['provider'], 'is_webui': 'true'})
            url = urlsplit(result['url'])
            params = dict(parse_qsl(url.query))
            params['prompt'] = 'select_account consent'
            state['session'] = {'id': result['state'], 'url': urlunsplit(url._replace(query=urlencode(params))),
                                'state': 'pending', 'expires_at': int(time.time()) + 300}
    elif action == 'cancel':
        session = state.get('session')
        if session and session['state'] == 'pending':
            result = bridge.call('DELETE', 'oauth/session', params={'state': session['id']})
            if result['cancelled']:
                session.update(state='cancelled', message='本次登录已取消。')
                session.pop('url', None)
            else:
                poll(bridge, state)
    elif action in ('prefer', 'rename'):
        account = next((row for row in rows if row['id'] == request.get('account')), None)
        if account is None:
            raise ValueError('请刷新后选择已有账号')
        if action == 'prefer':
            if account.get('disabled'):
                raise ValueError('此账号已停用，请在桥接服务中启用账号')
            if not next(row for row in accounts if row['id'] == account['id'])['preferred']:
                priority = max(int(row.get('priority', 0)) for row in rows) + 1
                bridge.call('PATCH', 'credentials/fields', json={'name': account['name'], 'priority': priority})
                account['priority'] = priority
        else:
            label = (request.get('label') or '').strip()
            if not label or len(label) > 80:
                raise ValueError('账号名称须为 1–80 个字符')
            state['labels'][account['id']] = label
    elif action != 'status':
        raise ValueError('账号操作无效')
    return {'ok': True, 'accounts': public_accounts(rows, state), 'session': state.get('session')}


def quota_credential(bridge, account):
    row = next((row for row in bridge.accounts() if row['id'] == account), None)
    if row is None:
        raise ValueError('请刷新后选择已有账号')
    return {'ok': True, 'credential_file': row['path']}


def main():
    payload = json.load(sys.stdin)
    settings, request = payload['settings'], payload['request']
    bridge = Bridge(settings)
    try:
        if request['action'] == 'quota_credential':
            return quota_credential(bridge, request.get('account'))
        path = Path(settings['state_file']).expanduser()
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with path.with_suffix('.lock').open('a') as lock:
            os.fchmod(lock.fileno(), 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            original = path.read_text() if path.exists() else '{}'
            state = json.loads(original)
            result = perform(bridge, request, state)
            if state != json.loads(original):
                save(path, state)
            return result
    finally:
        bridge.http.close()


if __name__ == '__main__':
    try:
        result = main()
    except (OSError, KeyError, TypeError, json.JSONDecodeError):
        result = {'ok': False, 'error': '账号管理配置或服务返回格式无效'}
    except ValueError as error:
        result = {'ok': False, 'error': str(error)}
    print(json.dumps(result, ensure_ascii=False))
