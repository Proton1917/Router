from datetime import datetime, timezone
from http.client import HTTPConnection, HTTPException
import json as codec
from pathlib import Path
from urllib.parse import urlencode, urlsplit


class Bridge:
    def __init__(self, settings):
        self.settings = settings
        self.base = settings['base_url'].rstrip('/')
        address = urlsplit(self.base)
        if (address.scheme != 'http' or address.hostname not in ('127.0.0.1', 'localhost', '::1')
                or address.username or address.password or address.path or address.query or address.fragment):
            raise ValueError('账号管理地址须使用本机回环 HTTP 地址')
        self.http = HTTPConnection(address.hostname, address.port, timeout=20)
        self.headers = {'Authorization': 'Bearer ' + Path(settings['token_file']).expanduser().read_text().strip(),
                        'Content-Type': 'application/json'}

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.http.close()

    def call(self, method, path, *, json=None, params=None, version=8):
        endpoint = f'/v{version}/management/{path}'
        if params:
            endpoint += '?' + urlencode(params)
        try:
            self.http.request(method, endpoint, codec.dumps(json).encode() if json is not None else None, self.headers)
            response = self.http.getresponse()
            content = response.read(1_048_577)
        except (OSError, HTTPException) as error:
            raise ValueError('连接桥接服务失败，请检查本机服务状态') from error
        if response.status >= 300:
            raise ValueError(f'桥接服务账号操作失败（HTTP {response.status}）')
        if len(content) > 1_048_576:
            raise ValueError('桥接服务响应超过 1 MiB')
        return codec.loads(content)

    def accounts(self):
        return [row for row in self.call('GET', 'credentials')['files']
                if row['provider'] == self.settings['provider'] and not row.get('runtime_only')]

    def refresh(self, account):
        result = self.call('POST', 'credentials/refresh', json={'name': account['name'], 'auth_index': account['auth_index']})
        if result.get('ok') is not True:
            raise ValueError('桥接服务未确认续期成功')

    def ensure_available(self):
        accounts = [row for row in self.accounts() if not row.get('disabled')]
        if not accounts or any(not row.get('unavailable') for row in accounts):
            return
        for account in sorted(accounts, key=lambda row: (-int(row.get('priority', 0)), row['id'])):
            if not account.get('path'):
                continue
            credential = codec.loads(Path(account['path']).read_text())
            expiry = credential.get('expired')
            if expiry and datetime.fromisoformat(expiry) <= datetime.now(timezone.utc):
                self.refresh(account)
                return
