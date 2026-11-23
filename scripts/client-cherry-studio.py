from contextlib import closing
import json
from pathlib import Path
import re
import sqlite3
import time
import uuid

from model_catalog import ModelCatalog, binding_metadata


def model_values(config, catalog, binding, existing):
    metadata = binding_metadata(config, binding, catalog.load)
    modalities = metadata['capabilities'] & {'text', 'image', 'audio', 'video'}
    capabilities = set(json.loads(existing.get('capabilities') or '[]'))
    capabilities.update({'image-recognition', 'audio-recognition', 'video-recognition'} &
                        {value + '-recognition' for value in modalities})
    if metadata['reasoning']:
        capabilities.add('reasoning')
    if {'tools', 'function-call'} & metadata['capabilities']:
        capabilities.add('function-call')
    policy = config['profiles'][binding['profile']]['standard'].get('reasoning_models')
    reasoning = existing.get('reasoning')
    if policy and policy['follow_request']:
        efforts = [level for level in policy['models'] if level not in ('low', 'medium', 'high', 'xhigh')]
        efforts[:0] = [level for level in ('low', 'medium', 'high', 'xhigh') if level in policy['models']]
        default = policy['default_effort']
        reasoning = json.dumps({'controls': [{'kind': 'effort', 'values': efforts, 'default': default}],
                                'selectableEfforts': efforts, 'defaultEffort': default}, separators=(',', ':'))
    recorded_modalities = set(json.loads(existing.get('input_modalities') or '[]'))
    inputs = sorted(recorded_modalities | modalities | {'text'}, key=lambda value: (value != 'text', value))
    return {
        'name': existing.get('name') or metadata['name'],
        'capabilities': json.dumps(sorted(capabilities), separators=(',', ':')),
        'input_modalities': json.dumps(inputs, separators=(',', ':')),
        'input_modalities_explicit': 1,
        'context_window': metadata['context_window'] or existing.get('context_window'),
        'supports_streaming': existing.get('supports_streaming') if existing.get('supports_streaming') is not None else 1,
        'reasoning': reasoning,
    }


def run(request, gateway_check):
    spec = request['integration']
    settings = spec['settings']
    sync = request['action'] == 'sync'
    checks = gateway_check(spec, settings) if request['action'] in ('check', 'sync') else []
    config = request['config']
    path = Path(request['config_path'])
    catalog = ModelCatalog(config, path.parent)
    database = Path(settings['database']).expanduser().resolve()
    include = re.compile(settings['include'])
    changes = []
    missing = []
    inspected = 0
    with closing(sqlite3.connect(database.as_uri() + ('?mode=rw' if sync else '?mode=ro'), uri=True, timeout=10)) as db, db:
        db.row_factory = sqlite3.Row
        if sync:
            db.execute('BEGIN IMMEDIATE')
        provider_id = settings['provider_id']
        provider = db.execute('SELECT endpoint_configs FROM user_provider WHERE provider_id = ?', (provider_id,)).fetchone()
        if provider is None:
            raise ValueError('Cherry Studio 缺少指定的服务设置')
        endpoint = json.loads(provider['endpoint_configs']).get(settings['endpoint'], {})
        if endpoint.get('baseUrl', '').rstrip('/') != spec['gateway_url'].rstrip('/'):
            raise ValueError('Cherry Studio 的服务地址与 Router 接入地址不同')
        existing = {row['model_id']: dict(row) for row in db.execute('SELECT * FROM user_model WHERE provider_id = ?', (provider_id,))}
        last_order = max((row['order_key'] for row in existing.values()), default='a0')
        for name in spec['commands']:
            for binding in config['management']['commands'][name].get('model_bindings', []):
                model_id = f'{name}/{binding["model"]}'
                if not binding.get('visible', True) or not include.fullmatch(model_id):
                    continue
                inspected += 1
                aliases = (f'{name}/{alias}' for alias in binding.get('aliases', []))
                row = next((existing[key] for key in (model_id, *aliases) if key in existing), {})
                values = model_values(config, catalog, binding, row)
                if row and all(row.get(field) == value for field, value in values.items()):
                    continue
                if not sync:
                    missing.append(model_id)
                    continue
                now = time.time_ns() // 1_000_000
                values['updated_at'] = now
                if row:
                    assignments = ', '.join(f'{field} = ?' for field in values)
                    db.execute(f'UPDATE user_model SET {assignments} WHERE id = ?', (*values.values(), row['id']))
                else:
                    last_order += 'V'
                    values.update(id=str(uuid.uuid4()), provider_id=provider_id, model_id=model_id, order_key=last_order, created_at=now)
                    placeholders = ', '.join('?' for _ in values)
                    db.execute(f'INSERT INTO user_model ({", ".join(values)}) VALUES ({placeholders})', tuple(values.values()))
                changes.append(model_id)
    if inspected:
        checks.append({'label': '模型登记', 'count': inspected - len(missing)})
    message = f'已同步 {len(changes)} 个模型。' if sync else (f'{len(missing)} 个模型待同步。' if missing else '模型登记与 Router 目录一致。')
    return {'ok': True, 'configured': not missing and inspected > 0, 'checks': checks, 'changes': changes, 'message': message}
