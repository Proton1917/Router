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
    prepared = adapter.prepare({'config':candidate, 'command_name':name, 'args':['--settings', json.dumps({'env':{'CLAUDE_CODE_MAX_CONTEXT_TOKENS':str(limits['min']), 'CLAUDE_CODE_AUTO_COMPACT_WINDOW':str(limits['min'])}})]})
    settings = json.loads(prepared['args'][1])
    for binding in bindings[:2]:
        key = binding['model'].replace('[1m]', '')
        assert settings['modelSettings'][key]['autoCompactWindow'] == binding['context_window']
    assert prepared['env']['CLAUDE_CODE_AUTO_COMPACT_WINDOW'] == ''
    assert int(prepared['env']['CLAUDE_CODE_MAX_CONTEXT_TOKENS']) == limits['max']
    assert config['management']['commands'][name] == original
    checks.append({'command':name, 'independent_models':True, 'min_max_validated':True, 'routes_unchanged':True})
assert checks
response = session.get(base + 'state', timeout=15)
response.raise_for_status()
assert response.json()['revision'] == state['revision']
print(json.dumps({'ok':True, 'checks':checks}))
