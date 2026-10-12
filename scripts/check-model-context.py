import argparse
import copy
import importlib.util
import json
from pathlib import Path

import requests

parser = argparse.ArgumentParser(description='检查实际命令配置的逐模型上下文预算')
parser.add_argument('config', type=Path)
args = parser.parse_args()
path = args.config.resolve()
config = json.loads(path.read_text())
manager = config['management']
token_path = Path(manager['token_file'])
if not token_path.is_absolute():
    token_path = path.parent / token_path
session = requests.Session()
session.trust_env = False
session.headers['Authorization'] = 'Bearer ' + token_path.read_text().strip()
base = 'http://' + manager['listen'] + '/api/'
response = session.get(base + 'state', timeout=15)
response.raise_for_status()
state = response.json()
assert state['config'] == config
module = importlib.util.spec_from_file_location('claude_catalog', Path(__file__).with_name('claude-model-catalog.py'))
adapter = importlib.util.module_from_spec(module)
module.loader.exec_module(adapter)
checks = []
for name, entry in manager['commands'].items():
    template = manager['templates'][entry['template']]
    limits = template.get('context_window_range')
    if not limits or len(entry.get('model_bindings', [])) < 2:
        continue
    original = copy.deepcopy(entry)
    for budget in [limits['min'], limits['max'], limits['min']-1, limits['max']+1]:
        candidate = copy.deepcopy(config)
        candidate['management']['commands'][name]['model_bindings'][0]['context_window'] = budget
        response = session.post(base + 'preview', json={'revision':state['revision'], 'config':candidate}, timeout=30)
        assert response.status_code == (200 if limits['min'] <= budget <= limits['max'] else 400), (name, budget, response.text)
        if response.ok:
            planned = response.json()['config']
            assert planned['routes'] == config['routes']
            assert planned['profiles'] == config['profiles']
    candidate = copy.deepcopy(config)
    bindings = candidate['management']['commands'][name]['model_bindings']
    bindings[0]['context_window'] = limits['min']
    bindings[1]['context_window'] = limits['max']
    prepared = adapter.prepare({'config':candidate, 'config_directory':str(path.parent), 'command_name':name, 'args':['--settings', json.dumps({'env':{'CLAUDE_CODE_MAX_CONTEXT_TOKENS':str(limits['min']), 'CLAUDE_CODE_AUTO_COMPACT_WINDOW':str(limits['min'])}})]})
    settings = json.loads(prepared['args'][1])
    for binding in bindings[:2]:
        key = binding['model'].replace('[1m]', '')
        assert settings['modelSettings'][key]['autoCompactWindow'] == binding['context_window']
    assert prepared['env']['CLAUDE_CODE_AUTO_COMPACT_WINDOW'] == ''
    assert int(prepared['env']['CLAUDE_CODE_MAX_CONTEXT_TOKENS']) == limits['max']
    assert config['management']['commands'][name] == original
    checks.append({'command':name, 'independent_models':True, 'min_max_validated':True, 'routes_unchanged':True})
assert checks
if manager.get('context_discovery'):
    for name in [row['command'] for row in checks]:
        response = session.post(base + 'models/context', json={'config':config, 'command_name':name}, timeout=90)
        response.raise_for_status()
        entries = response.json()['commands'][name]
        assert entries
        assert all(not row.get('error') for entry in entries.values() for row in entry['destinations']), entries
        for binding in manager['commands'][name]['model_bindings']:
            maximum = entries[binding['id']]['max_tokens']
            if maximum is None:
                continue
            candidate = copy.deepcopy(config)
            command = candidate['management']['commands'][name]
            candidate['management']['templates'][command['template']]['context_window_range']['max'] = maximum + 1
            next(item for item in command['model_bindings'] if item['id'] == binding['id'])['context_window'] = maximum + 1
            response = session.post(base + 'preview', json={'revision':state['revision'], 'config':candidate}, timeout=90)
            assert response.status_code == 400 and '后端上限' in response.text, (name,binding['id'],response.text)
    lower = copy.deepcopy(config)
    choice = next((name,binding) for name,command in lower['management']['commands'].items() for binding in command.get('model_bindings',[]) if lower['profiles'][binding['profile']]['standard'].get('provider') == 'novita')
    name, binding = choice
    lower['profiles'][binding['profile']]['standard']['provider'] = 'cloudflare'
    response = session.post(base + 'models/context', json={'config':lower,'command_name':name}, timeout=90)
    response.raise_for_status()
    maximum = response.json()['commands'][name][binding['id']]['max_tokens']
    assert maximum == 262144, response.text
    binding['context_window'] = maximum + 1
    response = session.post(base + 'preview', json={'revision':state['revision'],'config':lower}, timeout=90)
    assert response.status_code == 400 and '后端上限' in response.text, response.text
    response = session.post(base + 'apply', json={'revision':state['revision'],'config':lower}, timeout=90)
    assert response.status_code == 400 and '后端上限' in response.text, response.text
    binding['context_window'] = None
    prepared = adapter.prepare({'config':lower,'config_directory':str(path.parent),'command_name':name,'args':['--settings',json.dumps({'env':{'CLAUDE_CODE_MAX_CONTEXT_TOKENS':'500000','CLAUDE_CODE_AUTO_COMPACT_WINDOW':'500000'}})]})
    settings = json.loads(prepared['args'][1])
    assert settings['modelSettings'][binding['model'].replace('[1m]','')]['autoCompactWindow'] == maximum
    checks.append({'provider_cap':maximum,'backend_validation':True,'default_budget_capped':True,'fast_checked':True})
    overridden = copy.deepcopy(config)
    name,binding = next((name,binding) for name,command in overridden['management']['commands'].items() for binding in command.get('model_bindings',[]) if overridden['profiles'][binding['profile']].get('provider_override'))
    override = overridden['profiles'][binding['profile']]['provider_override']
    template = overridden['management']['templates'][overridden['management']['commands'][name]['template']]
    settings = {'env':{'CLAUDE_CODE_MAX_CONTEXT_TOKENS':'500000','CLAUDE_CODE_AUTO_COMPACT_WINDOW':'500000',template['header_env']:f"{override['header']}: 1; {override['mode_key']}={override['mode_value']}; {override['value_key']}=inference-net/fast"}}
    prepared = adapter.prepare({'config':overridden,'config_directory':str(path.parent),'command_name':name,'args':['--settings',json.dumps(settings)]})
    assert json.loads(prepared['args'][1])['modelSettings'][binding['model'].replace('[1m]','')]['autoCompactWindow'] == 250000
    checks.append({'shell_provider_override_capped':True,'provider_cap':250000})
response = session.get(base + 'state', timeout=15)
response.raise_for_status()
assert response.json()['revision'] == state['revision']
print(json.dumps({'ok':True, 'checks':checks}))
