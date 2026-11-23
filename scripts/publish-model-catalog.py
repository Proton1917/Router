from http.client import HTTPConnection
from itertools import count
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit


def catalog_entries(models, variants):
    entries = {}
    families = {}
    suffixes = sorted((variants or {}).get('suffixes', {}).items(), key=lambda item: -len(item[1]))
    for row in models:
        model = row['id']
        match = next(((effort, suffix) for effort, suffix in suffixes if suffix and model.endswith(suffix)), None)
        if match:
            effort, suffix = match
            family = model.removesuffix(suffix)
            families.setdefault(family, {})[effort] = row
        else:
            entries[model] = (row['name'], {'model': model})

    for family, rows in families.items():
        default = variants['default_effort']
        if default not in rows:
            default = next(iter(rows))
        primary = rows[default]
        name = family if primary['name'] == primary['id'] else primary['name']
        name = name.removesuffix(variants['name_suffixes'].get(default, ''))
        aliases = {alias: effort for alias, effort in variants.get('aliases', {}).items()
                   if effort in rows and alias not in rows}
        entries[family] = (name, {
            'model': primary['id'],
            'reasoning_models': {
                'label': name, 'models': {effort: row['id'] for effort, row in rows.items()},
                'default_effort': default, 'follow_request': True, 'aliases': aliases,
                'labels': variants.get('labels', {}), 'remove': variants.get('remove', []),
            },
        })
    return entries


def register(config, backend, models, rules):
    management = config['management']
    source = management['model_catalogs'][backend]
    entries = catalog_entries(models, source.get('reasoning_variants'))
    suffixes = [suffix.lower() for suffix in config['runtime']['selection'].get('ignored_model_suffixes', [])]

    def normalized(model):
        value = model.lower()
        suffix = next((suffix for suffix in suffixes if value.endswith(suffix)), '')
        return value.removesuffix(suffix)

    added = []
    for rule in rules:
        command = rule['command']
        entry = management['commands'][command]
        template = management['templates'][entry['template']]
        protocol = config['runtime']['server']['endpoints'][template['request_path']]
        protocols = management.get('backend_protocols', {}).get(backend)
        if protocols is not None and protocol not in protocols:
            raise ValueError(f'命令 {command} 的接口协议与后端 {backend} 不兼容')
        if entry.get('profile') or entry.get('match_all_models'):
            raise ValueError(f'命令 {command} 使用单一模型路由，请为其配置多模型列表')

        bindings = entry.setdefault('model_bindings', [])
        occupied_ids = {binding['id'] for binding in bindings} | set(config['profiles'])
        existing = {normalized(model) for binding in bindings
                    for model in (binding['model'], *binding.get('aliases', []))}
        include = re.compile(rule['include'])
        required = set(rule.get('required_efforts', []))
        for model, (name, destination) in sorted(entries.items()):
            efforts = destination.get('reasoning_models', {}).get('models', {})
            if normalized(model) in existing or not include.fullmatch(model) or not required.issubset(efforts):
                continue
            base = re.sub(r'[^a-z0-9_-]+', '-', f'catalog-{backend}-{model}'.lower())[:56]
            identifier = base
            for number in count(2):
                if identifier not in occupied_ids:
                    break
                identifier = f'{base}-{number}'
            config['profiles'][identifier] = {'standard': {**(source.get('destination') or {}), **destination, 'target': backend}}
            bindings.append({'id': identifier, 'model': model, 'profile': identifier, 'label': name, 'visible': True})
            occupied_ids.add(identifier)
            existing.add(normalized(model))
            added.append(f'{command}/{model}')
    return added


def control(config_path, settings, backend, models):
    manager = json.loads(config_path.read_text())['management']
    address = urlsplit('http://' + manager['listen'])
    if address.hostname not in ('127.0.0.1', 'localhost', '::1') or address.netloc != manager['listen']:
        raise ValueError('模型登记使用本机回环管理地址')
    token_path = config_path.parent / Path(manager['token_file'])
    headers = {'Authorization': 'Bearer ' + token_path.read_text().strip(), 'Content-Type': 'application/json'}
    connection = HTTPConnection(address.hostname, address.port, timeout=20)

    def request(method, endpoint, body=None):
        payload = json.dumps(body).encode() if body is not None else None
        connection.request(method, '/api/' + endpoint, payload, headers)
        response = connection.getresponse()
        data = json.load(response)
        if response.status >= 300:
            raise RuntimeError(f'Router {endpoint} 返回 HTTP {response.status}：{data.get("error", "")}')
        return data

    try:
        state = request('GET', 'state')
        config = state['config']
        rules = config['management']['model_catalogs'][backend]['update_settings'].get('register', [])
        if rules != settings['register']:
            return
        if added := register(config, backend, models, rules):
            request('POST', 'apply', {'revision': state['revision'], 'config': config})
            print(json.dumps({'added': added}, ensure_ascii=False))
    finally:
        connection.close()


def main():
    config_path, backend = sys.argv[1:]
    payload = json.load(sys.stdin)
    settings = payload['settings']
    if bridge := settings.get('bridge'):
        subprocess.run([sys.executable, str(Path(__file__).with_name('sync-bridge-catalog.py'))],
                       input=json.dumps({'models': payload['models'], 'settings': bridge}), text=True, check=True)
    if settings.get('register'):
        control(Path(config_path), settings, backend, payload['models'])


if __name__ == '__main__':
    main()
