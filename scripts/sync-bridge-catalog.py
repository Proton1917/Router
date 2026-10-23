import json
import os
from pathlib import Path
import sys
import tempfile
import yaml


def atomic(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as temporary:
        temporary.write(content)
        temporary.flush()
        os.fsync(temporary.fileno())
        name = temporary.name
    os.replace(name, path)


def main():
    request = json.load(sys.stdin)
    settings = request['settings']
    config_path = Path(settings['config_file'])
    config = yaml.safe_load(config_path.read_text())
    rows = []
    for model in request['models']:
        row = {'id': model['id'], 'name': model['id'], 'display_name': model['name'], 'object': 'model',
               'type': settings['provider'], 'owned_by': settings['provider']}
        if model.get('context_window'):
            row['context_length'] = model['context_window']
        rows.append(row)
    catalog = {settings['provider']: rows}
    current_path = config.get('models', {}).get('catalog')
    if current_path and Path(current_path).is_file() and json.loads(Path(current_path).read_text()) == catalog:
        return
    directory = Path(settings['output_directory'])
    slot = 'models-1.json' if current_path == str(directory/'models-0.json') else 'models-0.json'
    output = directory/slot
    atomic(output, json.dumps(catalog, ensure_ascii=False, indent=2) + '\n')
    config.setdefault('models', {})['catalog'] = str(output)
    atomic(config_path, yaml.safe_dump(config, allow_unicode=True, sort_keys=False))


if __name__ == '__main__':
    main()
