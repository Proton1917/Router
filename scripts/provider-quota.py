from importlib.util import module_from_spec, spec_from_file_location
import json
import math
import sys
import urllib.error
import urllib.request
from pathlib import Path


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None


def fetch(settings, token, body=None):
    proxy = settings.get('proxy_url')
    handlers = [NoRedirect()]
    if proxy is not None:
        handlers.append(urllib.request.ProxyHandler({'https': proxy} if proxy else {}))
    opener = urllib.request.build_opener(*handlers)
    request = urllib.request.Request(settings['url'], None if body is None else json.dumps(body).encode(),
                                     {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json', **settings.get('headers', {})})
    with opener.open(request, timeout=settings.get('timeout_seconds', 20)) as response:
        raw = response.read(262145)
        if len(raw) > 262144:
            raise ValueError('配额响应超过 256 KiB')
        return json.loads(raw)


def percent(value, multiplier=1):
    if value is None:
        return None
    number = float(value) * multiplier
    if not math.isfinite(number) or not 0 <= number <= 100:
        raise ValueError('上游配额百分比格式无效')
    return number


def load_mapper(settings, directory):
    path = settings.get('mapper')
    if path:
        file = Path(directory) / Path(path).expanduser()
    elif kind := settings.get('kind'):
        file = Path(__file__).with_name(f'quota_{kind}.py')
    else:
        return None
    spec = spec_from_file_location('quota_mapper', file)
    mapper = module_from_spec(spec)
    spec.loader.exec_module(mapper)
    return mapper


def fetch_through_bridge(settings, directory, mapper):
    from bridge_client import Bridge

    credential = (Path(directory) / Path(settings['credential_file']).expanduser()).resolve()
    with Bridge(settings['bridge']) as bridge:
        account = next((row for row in bridge.accounts() if row.get('path') and Path(row['path']).resolve() == credential), None)
        if account is None or account.get('disabled'):
            raise ValueError('所选账号在桥接服务中不可用')
        fields = settings.get('body_credentials', getattr(mapper, 'body_credentials', {}))
        payload = settings.get('request_body')
        if fields:
            payload = {**(payload or {}), **{key: account[field] for key, field in fields.items()}}
        headers = {'Authorization': 'Bearer $TOKEN$', 'Content-Type': 'application/json', **settings.get('headers', {})}
        call = {'auth_index': account['auth_index'], 'method': 'POST' if payload is not None else 'GET',
                'url': settings['url'], 'header': headers, 'data': json.dumps(payload) if payload is not None else ''}
        result = bridge.call('POST', 'api-call', json=call, version=0)
        if result['status_code'] == 401:
            bridge.refresh(account)
            result = bridge.call('POST', 'api-call', json=call, version=0)
        if not 200 <= result['status_code'] < 300:
            raise urllib.error.HTTPError(settings['url'], result['status_code'], '', None, None)
        return json.loads(result['body'])


def query(settings, token, directory):
    try:
        mapper = load_mapper(settings, directory)
        if settings.get('bridge'):
            payload = fetch_through_bridge(settings, directory, mapper)
        else:
            body = settings.get('request_body')
            if credential_file := settings.get('credential_file'):
                credential = json.loads((Path(directory) / Path(credential_file).expanduser()).read_text())
                if credential.get('disabled'):
                    raise ValueError('此账号已停用')
                token = credential[settings.get('token_field', 'access_token')]
                fields = settings.get('body_credentials', getattr(mapper, 'body_credentials', {}))
                if fields:
                    body = {**(body or {}), **{key: credential[field] for key, field in fields.items()}}
            payload = fetch(settings, token, body)
        result = mapper.normalize(settings, payload, percent) if mapper else payload
    except urllib.error.HTTPError as error:
        hint = {401: '上游拒绝此账号的授权，请检查登录状态', 403: '此授权无权查询配额', 429: '配额查询受到限流，请稍后刷新'}.get(error.code, '上游配额服务暂时不可用')
        result = {'groups': [], 'error': f'{hint}（HTTP {error.code}）'}
    except (urllib.error.URLError, TimeoutError):
        result = {'groups': [], 'error': '配额服务连接失败，请检查网络或代理配置'}
    except (KeyError, TypeError, json.JSONDecodeError):
        result = {'groups': [], 'error': '凭据或配额响应字段与接口不符'}
    except OSError:
        result = {'groups': [], 'error': '配额凭据文件读取失败'}
    except ValueError as error:
        result = {'groups': [], 'error': str(error)}
    return result


if __name__ == '__main__':
    request = json.load(sys.stdin)
    print(json.dumps(query(request['settings'], request.get('token'), request.get('config_directory', '.')), ensure_ascii=False, allow_nan=False))
