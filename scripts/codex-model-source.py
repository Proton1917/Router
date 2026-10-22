import copy
import json
import os
import subprocess
import sys
import urllib.request
from pathlib import Path

path = Path(sys.argv[1]).resolve()
config = json.loads(path.read_text())
command = config['management']['commands'][sys.argv[2]]
native = json.loads(Path(sys.argv[3]).read_text())
native_models = native['models']
if not native_models:
    raise ValueError('Codex 本机模型格式为空')
default = min(native_models, key=lambda row: row.get('priority', 0))
bindings = command.get('model_bindings') or [{'profile': command['profile']}]
targets = {}
for binding in bindings:
    if binding.get('visible', True):
        destination = config['profiles'][binding['profile']]['standard']
        targets.setdefault(destination['target'], set()).add(destination['model'])
models = {}
for target, wanted in targets.items():
    backend = config['runtime']['backends'][target]
    auth = backend.get('auth', {})
    headers = {}
    if auth:
        if auth.get('token_file'):
            token_path = Path(auth['token_file'])
            token = (token_path if token_path.is_absolute() else path.parent / token_path).read_text().strip()
        elif auth.get('token_env'):
            token = os.environ[auth['token_env']]
        elif auth.get('token_command'):
            token = subprocess.run(auth['token_command'], check=True, capture_output=True, text=True).stdout.strip()
        elif auth.get('oauth'):
            settings = dict(auth['oauth'])
            for key in ('credential_file', 'client_secret_file'):
                if settings.get(key) and not Path(settings[key]).is_absolute():
                    settings[key] = str(path.parent / settings[key])
            token = subprocess.run([os.environ['ROUTER_EXECUTABLE'], 'oauth-token', '--settings', json.dumps(settings)], check=True, capture_output=True, text=True, timeout=75).stdout.strip()
        else:
            raise ValueError('目录后端缺少凭据来源：' + target)
        if not token:
            raise ValueError('目录后端凭据为空：' + target)
        headers = {name: value.format(token=token) for name, value in auth['headers'].items()}
    url = backend['base_url'].rstrip('/') + os.environ.get('ROUTER_CODEX_CATALOG_PATH', '/v1/models')
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=30) as response:
        catalog = json.load(response)
    if not isinstance(catalog.get('data'), list):
        raise ValueError('后端目录需要包含 data 数组：' + target)
    found = {}
    for row in catalog['data']:
        if row.get('id') in wanted:
            if row['id'] in found:
                raise ValueError('后端目录模型标识重复：' + row['id'])
            found[row['id']] = row
    missing = wanted - found.keys()
    if missing:
        raise ValueError('后端目录缺少配置模型：' + ', '.join(sorted(missing)))
    for identifier, row in found.items():
        if identifier in models:
            raise ValueError('不同后端使用相同目录模型标识：' + identifier)
        preset = next((item for item in native_models if item['slug'] in (identifier, identifier.rsplit('/', 1)[-1])), default)
        model = copy.deepcopy(preset)
        model['slug'] = identifier
        model['display_name'] = row.get('name') or identifier
        model['description'] = row.get('description') or ''
        context = row.get('context_length')
        if not isinstance(context, int) or context <= 0:
            raise ValueError('后端目录缺少有效上下文长度：' + identifier)
        model['context_window'] = context
        model['max_context_window'] = context
        model['visibility'] = 'list'
        model['supported_in_api'] = True
        modalities = row.get('architecture', {}).get('input_modalities')
        if not isinstance(modalities, list) or not modalities:
            raise ValueError('后端目录缺少输入类型：' + identifier)
        model['input_modalities'] = [item for item in modalities if item in ('text', 'image')]
        if not model['input_modalities']:
            raise ValueError('此模型没有 Codex 支持的输入类型：' + identifier)
        parameters = row.get('supported_parameters')
        if not isinstance(parameters, list):
            raise ValueError('后端目录缺少支持参数：' + identifier)
        if not any(item in parameters for item in ('reasoning', 'reasoning_effort')):
            model['supported_reasoning_levels'] = []
            model['default_reasoning_level'] = None
        model['service_tiers'] = []
        model['additional_speed_tiers'] = []
        model['upgrade'] = None
        model['availability_nux'] = None
        models[identifier] = model
print(json.dumps({'models': list(models.values())}, ensure_ascii=False))
