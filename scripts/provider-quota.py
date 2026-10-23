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


def antigravity(settings):
    credential = json.loads(Path(settings['credential_file']).expanduser().read_text())
    if credential.get('disabled'):
        raise ValueError('此账号已停用')
    payload = fetch(settings, credential['access_token'], {'project': credential['project_id']})
    groups = []
    for group in payload['groups']:
        windows = []
        for bucket in group.get('buckets', []):
            windows.append({'label': settings.get('window_labels', {}).get(bucket.get('window'), bucket.get('displayName', '配额')),
                            'remaining_percent': percent(bucket.get('remainingFraction'), 100),
                            'reset_at': bucket.get('resetTime'), 'disabled': bucket.get('disabled', False)})
        groups.append({'label': settings.get('group_labels', {}).get(group['displayName'], group['displayName']), 'windows': windows})
    if not groups:
        raise ValueError('上游未返回配额分组')
    return {'groups': groups, 'details': []}


def grok(settings, token):
    payload = fetch(settings, token)
    config = payload.get('config')
    if not isinstance(config, dict):
        raise ValueError('上游未返回此账号的订阅配额')
    period = config.get('currentPeriod', {})
    used = config.get('creditUsagePercent')
    # 该接口采用 protobuf JSON，存在计费周期时省略的数值字段为零。
    if used is None and period.get('type'):
        used = 0
    remaining = None if used is None else max(0, 100 - float(used))
    title = settings.get('period_labels', {}).get(period.get('type'), '当前周期')
    result = {'groups': [{'label': '订阅共享额度' if config.get('isUnifiedBillingUser') else 'Grok Build 额度',
                         'windows': [{'label': title, 'remaining_percent': percent(remaining),
                                      'reset_at': period.get('end') or config.get('billingPeriodEnd')}]}], 'details': []}
    for item in config.get('productUsage', []):
        result['details'].append({'label': settings.get('product_labels', {}).get(item['product'], item['product']) + ' 已用',
                                  'value': str(percent(item.get('usagePercent', 0))) + '%'})
    for field, label in settings.get('balance_labels', {}).items():
        if field in config:
            result['details'].append({'label': label, 'value': f"${int(config[field].get('val', 0)) / 100:.2f} USD"})
    return result


def main():
    request = json.load(sys.stdin)
    settings = request['settings']
    try:
        if settings['kind'] == 'antigravity':
            result = antigravity(settings)
        elif settings['kind'] == 'grok':
            result = grok(settings, request['token'])
        else:
            raise ValueError('未知配额适配类型')
    except urllib.error.HTTPError as error:
        hint = {401: '授权已过期，请检查此服务的登录状态', 403: '此授权无权查询配额', 429: '配额查询受到限流，请稍后刷新'}.get(error.code, '上游配额服务暂时不可用')
        result = {'groups': [], 'error': f'{hint}（HTTP {error.code}）'}
    except (urllib.error.URLError, TimeoutError):
        result = {'groups': [], 'error': '配额服务连接失败，请检查网络或代理配置'}
    except (KeyError, TypeError, json.JSONDecodeError):
        result = {'groups': [], 'error': '凭据或配额响应字段与接口不符'}
    except OSError:
        result = {'groups': [], 'error': '配额凭据文件读取失败'}
    except ValueError as error:
        result = {'groups': [], 'error': str(error)}
    print(json.dumps(result, ensure_ascii=False, allow_nan=False))


if __name__ == '__main__':
    main()
