import argparse
import json
import os
import sys
from pathlib import Path


config_path = Path(os.environ['ROUTER_CONFIG'] if sys.argv[1] == '-' else sys.argv[1]).resolve()
catalog_directory, client_program = sys.argv[2:4]
arguments = sys.argv[4:]
separator = arguments.index('--')
provider_arguments = arguments[:separator]
config = json.loads(config_path.read_text())
name = os.environ['ROUTER_COMMAND_NAME']
command = config['management']['commands'][name]
template = config['management']['templates'][command['template']]
parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
parser.add_argument('--router-mode', choices=['api', 'account'],
                    default=command.get('options', {}).get('auth-mode', template['options']['auth-mode']['default']))
parser.add_argument('--router-preview', action='store_true')
parser.add_argument('--router-service-tier', choices=['standard', 'fast'],
                    default=command.get('options', {}).get('service-tier', template['options']['service-tier']['default']))
options, client_arguments = parser.parse_known_args(arguments[separator + 1:])
environment = dict(os.environ)
removed = ['OPENAI_API_KEY', 'OPENAI_BASE_URL']
for key in removed:
    environment.pop(key, None)
preview = options.router_preview or bool(environment.get('ROUTER_LAUNCH_PREVIEW'))

if options.router_mode == 'account':
    if not environment['ROUTER_CODEX_ACCOUNT_HOME'].strip():
        raise ValueError('请填写独立账号目录')
    account_home = Path(environment['ROUTER_CODEX_ACCOUNT_HOME']).expanduser()
    if not account_home.is_absolute():
        account_home = config_path.parent / account_home
    account_home = account_home.resolve()
    if account_home == (Path.home() / '.codex').resolve():
        raise ValueError('独立账号目录须与现有 ~/.codex 目录分开')
    account_arguments = json.loads(environment['ROUTER_CODEX_ACCOUNT_ARGS'])
    if not isinstance(account_arguments, list) or not all(isinstance(item, str) for item in account_arguments):
        raise ValueError('ROUTER_CODEX_ACCOUNT_ARGS 必须为字符串数组')
    program = client_program
    launch_arguments = account_arguments + client_arguments
    environment['CODEX_HOME'] = str(account_home)
    if not preview:
        account_home.mkdir(parents=True, exist_ok=True, mode=0o700)
        account_home.chmod(0o700)
else:
    if client_arguments and client_arguments[0] in ('login', 'logout'):
        parser.error('账号登录请使用 --router-mode account login；API 模式使用 Router 后端凭据')
    model = command.get('model')
    if not model and command.get('profile'):
        profile = command['profile']
        model = config['management'].get('profile_models', {}).get(profile, {}).get('responses')
        model = model or config['profiles'][profile]['standard'].get('model')
    explicit_model = any(item in ('-m', '--model') or item.startswith(('--model=', '-m='))
                         for item in client_arguments[:client_arguments.index('--') if '--' in client_arguments else len(client_arguments)])
    if model and not explicit_model:
        client_arguments = ['--model', model] + client_arguments
    service_tiers = json.loads(environment['ROUTER_CODEX_SERVICE_TIERS'])
    service_arguments = ['-c', 'service_tier=' + json.dumps(service_tiers[options.router_service_tier])]
    program = '/bin/zsh'
    launch_arguments = [str(Path(__file__).with_name('codex-api.zsh')), str(config_path),
                        catalog_directory, client_program, *provider_arguments, '--',
                        *service_arguments, *client_arguments]
    if environment.get('ROUTER_CODEX_USE_CATALOG') == '0':
        program = client_program
        launch_arguments = [*provider_arguments, *service_arguments, *client_arguments]

if preview:
    print(json.dumps({'program': program, 'args': launch_arguments, 'mode': options.router_mode,
                      'env': {'CODEX_HOME': environment['CODEX_HOME']} if options.router_mode == 'account' else {},
                      'unset_env': removed}, ensure_ascii=False))
elif options.router_mode == 'api':
    executable = environment['ROUTER_EXECUTABLE']
    # 复用 Router 的最终启动阶段，确保 API 网关运行并应用命令环境。
    os.execvpe(executable, [executable, '--config', str(config_path), '__exec', program, *launch_arguments], environment)
else:
    for key in ('ROUTER_EXECUTABLE', 'ROUTER_CONFIG', 'ROUTER_COMMAND_NAME', 'ROUTER_LAUNCH_PREVIEW'):
        environment.pop(key, None)
    os.execvpe(program, [program, *launch_arguments], environment)
