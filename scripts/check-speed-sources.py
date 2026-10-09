import argparse
import json
from pathlib import Path

import requests

parser = argparse.ArgumentParser(description='通过当前 Router 的实际请求检查速度字段选择')
parser.add_argument('config', type=Path)
parser.add_argument('command')
args = parser.parse_args()
config = json.loads(args.config.read_text())
manager = config['management']
command = manager['commands'][args.command]
template = manager['templates'][command['template']]
server = config['runtime']['server']
session = requests.Session()
session.trust_env = False
session.headers.update({manager['command_header']: args.command, server['dry_run_header']: server['dry_run_values'][0]})
url = 'http://' + server['listen'] + template['request_path']
sources = config['runtime']['selection']['speed_sources']
assert sources, '配置必须声明速度来源'
source = sources[0]
assert source['pointer'].count('/') == 1, '此检查程序需使用顶层速度字段'
field = source['pointer'][1:]
assert config['runtime']['selection']['speed_pointer'].count('/') == 1, '此检查程序需使用顶层主要速度字段'
checked = []
for binding in command['model_bindings']:
    profile = config['profiles'][binding['profile']]
    for mode, fields, fast in [(value, {field: value}, True) for value in source['fast_values']] + [
        ('standard-source', {field: 'default'}, False),
        ('explicit-standard', {field: source['fast_values'][0], config['runtime']['selection']['speed_pointer'][1:]: 'standard'}, False),
        ('explicit-fast', {field: 'default', config['runtime']['selection']['speed_pointer'][1:]: config['runtime']['selection']['fast_value']}, True),
    ]:
        destination = profile.get('fast', profile['standard']) if fast else profile['standard']
        response = session.post(url, json={'model': binding['model'], 'input': 'Route verification', **fields}, timeout=30)
        response.raise_for_status()
        result = response.json()
        assert result['target'] == destination['target'], (binding['id'], mode, result)
        assert result['outgoing_model'] == destination.get('model', binding['model']), (binding['id'], mode, result)
        provider = result['outgoing_provider']
        if destination.get('provider'):
            assert provider['only'] == [destination['provider']] and provider['allow_fallbacks'] is False, (binding['id'], mode, result)
        else:
            assert provider is None, (binding['id'], mode, result)
        checked.append({'model': binding['model'], 'mode': mode, 'provider': destination.get('provider')})
print(json.dumps({'ok': True, 'checked': checked}))
