import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


def at(value, keys):
    for key in keys:
        if not isinstance(value, dict) or key not in value:
            return None
        value = value[key]
    return value


def positive(value):
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None


class Discovery:
    def __init__(self, config, directory, refresh=False, client_headers=None):
        self.config = config
        self.directory = Path(directory)
        self.settings = config['management']['context_discovery']['settings']
        self.refresh = refresh
        self.client_headers = {name.lower():value for name,value in (client_headers or {}).items()}
        self.responses = {}
        self.headers = {}
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        self.cache = Path(self.settings['cache_directory'])
        if not self.cache.is_absolute():
            self.cache = self.directory / self.cache
        self.cache.mkdir(parents=True, exist_ok=True)
        if not positive(self.settings['ttl_seconds']) or not positive(self.settings['timeout_seconds']):
            raise ValueError('上下文目录缓存时间和超时必须为正整数')

    def catalog_url(self, target, path):
        source = self.settings.get('sources', {}).get(target, {})
        backend = self.config['runtime']['backends'][target]
        return source.get('base_url', backend['base_url']).rstrip('/') + path

    def get(self, target, path):
        backend = self.config['runtime']['backends'][target]
        source = self.settings.get('sources', {}).get(target, {})
        url = self.catalog_url(target, path)
        if url in self.responses:
            return self.responses[url]
        if target not in self.headers:
            auth = backend.get('auth', {})
            token = ''
            if auth.get('token_file'):
                token_source = Path(auth['token_file'])
                token = (token_source if token_source.is_absolute() else self.directory / token_source).read_text().strip()
            elif auth.get('token_env'):
                token = os.environ[auth['token_env']]
            elif auth.get('token_command'):
                token = subprocess.run(auth['token_command'], check=True, capture_output=True, text=True, timeout=self.settings['timeout_seconds']).stdout.strip()
            elif auth.get('oauth'):
                settings = dict(auth['oauth'])
                for key in ('credential_file', 'client_secret_file'):
                    if settings.get(key) and not Path(settings[key]).is_absolute():
                        settings[key] = str(self.directory / settings[key])
                token = subprocess.run([os.environ['ROUTER_EXECUTABLE'], 'oauth-token', '--settings', json.dumps(settings)], check=True, capture_output=True, text=True, timeout=75).stdout.strip()
            if auth and not token:
                raise ValueError('目录凭据不可用：' + target)
            self.headers[target] = {name: text.format(token=token) for name, text in source.get('auth_headers', auth.get('headers', {})).items()}
        request = urllib.request.Request(url, headers=self.headers[target])
        with self.opener.open(request, timeout=self.settings['timeout_seconds']) as response:
            self.responses[url] = json.load(response)
        return self.responses[url]

    def destination(self, destination):
        target = destination['target']
        model = destination.get('model', '')
        provider = destination.get('provider', '')
        source = self.settings.get('sources', {}).get(target)
        base = {'target': target, 'model': model, 'provider': provider, 'max_tokens': None}
        if not source or not model:
            return {**base, 'message': '此后端尚未提供上下文目录配置'}
        if any(model.startswith(prefix) for prefix in source.get('unsupported_prefixes', [])):
            return {**base, 'message': '此入口的目录没有公布预设的上下文上限'}
        identity = {'base_url': self.catalog_url(target, ''), 'target': target, 'model': model, 'provider': provider, 'source': source}
        key = hashlib.blake2b(json.dumps(identity, sort_keys=True).encode(), digest_size=16).hexdigest()
        cache = self.cache / (key + '.json')
        if cache.exists() and not self.refresh:
            saved = json.loads(cache.read_text())
            if 0 <= time.time() - saved['checked_at'] <= self.settings['ttl_seconds']:
                return saved['result']
        preset = source.get('presets')
        if preset and model.startswith(preset['prefix']):
            path = preset['path'].format(preset=urllib.parse.quote(model[len(preset['prefix']):], safe=''))
            configuration = at(self.get(target, path), preset['configuration'])
            if not isinstance(configuration, dict):
                raise ValueError('预设目录没有返回当前版本配置：' + model)
            models = configuration.get(preset['models']) or [configuration.get(preset['model'])]
            providers = [provider] if provider else at(configuration, preset['providers']) or ['']
            if not isinstance(models, list) or not models or not all(isinstance(item, str) and item for item in models):
                raise ValueError('预设目录缺少模型标识：' + model)
            if any(item.startswith(preset['prefix']) for item in models):
                raise ValueError('预设目录需要返回实际 API 模型：' + model)
            if not isinstance(providers, list) or not all(isinstance(item, str) for item in providers):
                raise ValueError('预设目录的供应商配置无效：' + model)
            resolved = [self.destination({'target':target, 'model':item, 'provider':provider}) for item in models for provider in providers]
            limits = [row['max_tokens'] for row in resolved if row['max_tokens'] is not None]
            result = {**base, 'max_tokens': min(limits) if len(limits) == len(resolved) else None, 'resolved_models': models, 'source': self.catalog_url(target, path), 'checked_at': int(time.time())}
            if result['max_tokens'] is None:
                result['message'] = '预设中的模型目录没有公布完整上下文上限'
            self.save(cache, result)
            return result
        data = self.get(target, source['path'])
        rows = at(data, source['rows'])
        if not isinstance(rows, list):
            raise ValueError('模型目录没有返回配置的模型数组：' + target)
        matches = [row for row in rows if row.get(source['id']) == model]
        if not matches:
            result = {**base, 'message': '后端模型目录未收录此 API 标识', 'checked_at':int(time.time()), 'source':self.catalog_url(target, source['path'])}
            self.save(cache, result)
            return result
        if len(matches) > 1:
            raise ValueError('模型目录的配置模型标识重复：' + model)
        window = at(matches[0], source['context'])
        if window is not None and not positive(window):
            raise ValueError('模型目录的上下文上限必须为正整数：' + model)
        limits = [window] if positive(window) else []
        result = {**base, 'source': self.catalog_url(target, source['path'])}
        endpoints = source.get('endpoints')
        if endpoints and limits:
            path = endpoints['path'].format(model=urllib.parse.quote(model, safe='/'))
            rows = at(self.get(target, path), endpoints['rows'])
            if not isinstance(rows, list) or not rows:
                raise ValueError('模型目录未返回供应商端点：' + model)
            if provider:
                rows = [row for row in rows if row.get(endpoints['provider'], '') == provider or row.get(endpoints['provider'], '').startswith(provider + '/')]
            if not rows:
                raise ValueError('模型目录未返回配置供应商：' + model + ' · ' + provider)
            for row in rows:
                value = at(row, endpoints['context'])
                if not positive(value):
                    raise ValueError('供应商端点没有有效上下文上限：' + model)
                limits.append(value)
                if endpoints.get('input_limit'):
                    value = at(row, endpoints['input_limit'])
                    if value is not None and not positive(value):
                        raise ValueError('供应商端点的输入上限必须为正整数：' + model)
                    if positive(value):
                        limits.append(value)
            result['source'] = self.catalog_url(target, path)
        result['max_tokens'] = min(limits) if limits else None
        if not limits:
            result['message'] = '后端模型目录没有公布上下文上限'
        result['checked_at'] = int(time.time())
        self.save(cache, result)
        return result

    def save(self, cache, result):
        with tempfile.NamedTemporaryFile(mode='w', dir=self.cache, delete=False) as output:
            json.dump({'checked_at': result['checked_at'], 'result': result}, output)
            temporary = output.name
        os.replace(temporary, cache)

    def command(self, name):
        command = self.config['management']['commands'][name]
        entries = {}
        for binding in command.get('model_bindings', []):
            profile = self.config['profiles'][binding['profile']]
            override = profile.get('provider_override')
            provider = None
            if override:
                header = self.client_headers.get(override['header'].lower(), '')
                fields = {key.strip():value.strip() for key,value in (item.split('=',1) for item in header.split(';') if '=' in item)}
                if fields.get(override['mode_key']) == override['mode_value']:
                    provider = fields.get(override['value_key'])
            destinations = []
            for speed in ['standard', 'fast']:
                if speed not in profile:
                    continue
                try:
                    destination = {**profile[speed], 'provider':provider} if provider else profile[speed]
                    row = self.destination(destination)
                except (OSError, ValueError, KeyError, subprocess.SubprocessError, urllib.error.URLError) as error:
                    reason = str(error.reason) if isinstance(error, urllib.error.URLError) else str(error) if isinstance(error, ValueError) else type(error).__name__
                    row = {'target': profile[speed]['target'], 'model': profile[speed].get('model'), 'max_tokens': None, 'error': '上下文目录获取失败：' + reason}
                destinations.append({**row, 'speed': speed})
            limits = [row['max_tokens'] for row in destinations if row['max_tokens'] is not None]
            entries[binding['id']] = {'max_tokens': min(limits) if limits else None, 'complete': all(row['max_tokens'] is not None for row in destinations), 'destinations': destinations}
        return entries


def discover(request):
    discovery = Discovery(request['config'], request['config_directory'], request.get('refresh', False), request.get('headers'))
    names = [request['command_name']] if request.get('command_name') else [name for name, command in request['config']['management']['commands'].items() if any(binding.get('context_window') is not None for binding in command.get('model_bindings', []))]
    return {'commands': {name: discovery.command(name) for name in names}}


def validate_budget(name, binding, entry):
    errors = [row['error'] for row in entry['destinations'] if row.get('error')]
    if errors:
        raise ValueError(name + ' · ' + binding['id'] + '：' + '; '.join(errors))
    if binding.get('context_window') is not None and entry['max_tokens'] is not None and binding['context_window'] > entry['max_tokens']:
        raise ValueError(f"{name} · {binding['id']} 的上下文预算不能超过后端上限 {entry['max_tokens']} tokens")


if __name__ == '__main__':
    try:
        print(json.dumps(discover(json.load(sys.stdin)), ensure_ascii=False))
    except (OSError, ValueError, KeyError) as error:
        print('上下文上限识别失败：' + str(error), file=sys.stderr)
        raise SystemExit(1)
