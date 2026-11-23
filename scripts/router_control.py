from http.client import HTTPConnection
import json
from pathlib import Path
from urllib.parse import urlsplit


class RouterControl:
    def __init__(self, path, config):
        manager = config['management']
        address = urlsplit('http://' + manager['listen'])
        if address.hostname not in ('127.0.0.1', 'localhost', '::1') or address.netloc != manager['listen']:
            raise ValueError('Router 管理接口使用本机回环地址')
        token_file = Path(path).parent / manager['token_file']
        self.headers = {'Authorization': 'Bearer ' + token_file.read_text().strip(), 'Content-Type': 'application/json'}
        self.connection = HTTPConnection(address.hostname, address.port, timeout=30)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.connection.close()

    def request(self, endpoint, body=None):
        payload = json.dumps(body).encode() if body is not None else None
        self.connection.request('GET' if payload is None else 'POST', '/api/' + endpoint, payload, self.headers)
        response = self.connection.getresponse()
        content = response.read()
        if response.status >= 300:
            message = json.loads(content).get('error', '') if content and response.getheader('content-type', '').startswith('application/json') else ''
            raise ValueError(f'Router {endpoint} 返回 HTTP {response.status} {message}'.strip())
        return json.loads(content)
