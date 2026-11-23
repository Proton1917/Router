import copy
import json
import os
from pathlib import Path
import re
import sys

from model_catalog import ModelCatalog, binding_metadata
from router_control import RouterControl


def command_bindings(config, name):
    manager = config['management']
    command = manager['commands'][name]
    if command.get('model_bindings'):
        return [binding for binding in command['model_bindings'] if binding.get('visible', True)]
    profile = command.get('profile')
    if profile:
        model = command.get('model') or manager.get('profile_models', {}).get(profile, {}).get('responses')
        return [{'model': model or config['profiles'][profile]['standard']['model'], 'profile': profile}]
    profiles = {route['profile'] for route in config['routes']
                if any(header['name'].lower() == manager['command_header'].lower() and name in header.get('values', [])
                       for header in route['match'].get('headers', []))}
    return [{'model': manager.get('profile_models', {}).get(profile, {}).get('responses') or config['profiles'][profile]['standard']['model'],
             'profile': profile} for profile in sorted(profiles)]


def build(path, name, native):
    config = json.loads(path.read_text())
    manager = config['management']
    catalog = ModelCatalog(config, path.parent)
    native_models = native['models']
    default = min(native_models, key=lambda row: row.get('priority', 0))
    defaults = json.loads(os.environ.get('ROUTER_CODEX_DEFAULT_SERVICE_TIERS', '[]'))
    if not isinstance(defaults, list) or any(not isinstance(row.get('id'), str) or not isinstance(row.get('name'), str) for row in defaults):
        raise ValueError('默认服务等级须提供标识与显示名称')
    labels = manager.get('labels', {})
    pattern = os.environ.get('ROUTER_CODEX_CATALOG_MODEL_PATTERN', '')
    upstream_ids = set()
    models = {}
    with RouterControl(path, config) as router:
        for binding in command_bindings(config, name):
            client_model = binding['model']
            if client_model in models:
                raise ValueError(f'客户端模型 ID 重复：{client_model}')
            profile = config['profiles'][binding['profile']]
            metadata = binding_metadata(config, binding, lambda backend: catalog.read(router, backend))
            upstream_ids.update(metadata['upstream_ids'])
            upstream = metadata['upstream']
            native_ids = (upstream, upstream.rsplit('/', 1)[-1])
            model = copy.deepcopy(next((row for row in native_models if row['slug'] in native_ids), default))
            context = metadata['context_window']
            if context is None:
                raise ValueError(f'Router 目录与命令配置均缺少模型上下文：{client_model}')
            modalities = metadata['capabilities'] & {'text', 'image'}
            if not metadata['reasoning']:
                model.update(supported_reasoning_levels=[], default_reasoning_level=None)
            tiers = {row['id']: row for row in defaults}
            standard, fast = profile['standard'], profile.get('fast')
            if fast and standard.get('model') == fast.get('model') == upstream and standard['target'] == fast['target']:
                tier = fast.get('field_policy', {}).get('set', {}).get('/service_tier')
                if tier:
                    tiers[tier] = {'id': tier, 'name': labels.get('service-tier:' + tier, tiers.get(tier, {}).get('name', tier)),
                                   'description': labels.get('service-tier-description:' + tier, tiers.get(tier, {}).get('description', ''))}
            model.update(slug=client_model, display_name=metadata['name'],
                         description=binding.get('description') or '', context_window=context, max_context_window=context,
                         visibility='list', supported_in_api=True, input_modalities=sorted(modalities or {'text'}, reverse=True),
                         service_tiers=[tiers[key] for key in sorted(tiers)], additional_speed_tiers=[],
                         upgrade=None, availability_nux=None)
            models[client_model] = model
    if not models:
        raise ValueError('当前命令需要至少一个启用的模型绑定')
    if pattern and not any(re.search(pattern, identifier) for identifier in upstream_ids):
        raise ValueError('模型目录缺少启动方式指定的模型标识')
    return {'models': list(models.values())}


if __name__ == '__main__':
    path, name, native = sys.argv[1:]
    print(json.dumps(build(Path(path).resolve(), name, json.loads(Path(native).read_text())), ensure_ascii=False))
