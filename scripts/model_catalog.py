import json
from pathlib import Path
import time

from router_control import RouterControl


class ModelCatalog:
    def __init__(self, config, config_directory):
        self.config = config
        self.directory = Path(config_directory)
        self.models = {}

    def load(self, backend, max_age=None):
        if backend not in self.models:
            path = self.directory / 'model-catalogs' / (backend + '.json')
            snapshot = json.loads(path.read_text()) if path.is_file() else {}
            source = self.config['management'].get('model_catalogs', {}).get(backend, {})
            saved_source = snapshot.get('source', {})
            matches = snapshot.get('base_url') == self.config['runtime']['backends'][backend]['base_url']
            matches = matches and all(saved_source.get(key) == value for key, value in source.items())
            recent = max_age is None or 0 <= time.time() - (snapshot.get('updated_at') or 0) <= max_age
            self.models[backend] = {row['id']: row for row in snapshot.get('models', [])} if matches and recent else {}
        return self.models[backend]

    def read(self, control, backend, refresh=False):
        if backend not in self.models:
            payload = control.request('catalogs/read', {'backend': backend, 'refresh': refresh})
            if not payload['models'] and payload.get('error'):
                raise ValueError(f'{backend} 模型目录获取失败：{payload["error"]}')
            self.models[backend] = {row['id']: row for row in payload['models']}
        return self.models[backend]


def destination_models(destination, model):
    policy = destination.get('reasoning_models')
    if policy:
        if policy['follow_request']:
            return list(dict.fromkeys(policy['models'].values()))
        return [policy['models'][policy['default_effort']]]
    return [destination.get('model') or destination.get('model_template', '{model}').replace('{model}', model)]


def binding_metadata(config, binding, read_catalog):
    profile = config['profiles'][binding['profile']]
    rows = []
    for speed in ('standard', 'fast'):
        if speed not in profile:
            continue
        destination = profile[speed]
        backend = destination['target']
        source = read_catalog(backend)
        for identifier in destination_models(destination, binding['model']):
            if identifier not in source:
                raise ValueError(f'{backend} 的 Router 目录缺少已配置模型 {identifier}')
            rows.append(source[identifier])
    primary = rows[0]
    labels = config['management'].get('labels', {})
    limits = [row['context_window'] for row in rows if row.get('context_window')]
    if binding.get('context_window'):
        limits.append(binding['context_window'])
    capabilities = set.intersection(*(set(row.get('capabilities', [])) for row in rows))
    return {
        'name': binding.get('label') or labels.get('model:' + primary['id']) or primary['name'],
        'context_window': min(limits) if limits else None,
        'capabilities': capabilities,
        'reasoning': all({'reasoning', 'reasoning_effort'}.intersection(row.get('capabilities', [])) for row in rows),
        'upstream': primary['id'],
        'upstream_ids': [row['id'] for row in rows],
    }
