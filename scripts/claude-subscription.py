import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import secrets
import selectors
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import termios
import time
import tty
import uuid


AUTH_ENV = (
    'ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_BASE_URL',
    'ANTHROPIC_CUSTOM_HEADERS', 'CLAUDE_CODE_OAUTH_TOKEN', 'ANTHROPIC_PROFILE',
    'ANTHROPIC_FEDERATION_RULE_ID', 'ANTHROPIC_ORGANIZATION_ID',
    'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY',
    'CLAUDE_CONFIG_DIR',
)


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        json.dump(value, output, ensure_ascii=False)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)


def read(path, default):
    return json.loads(path.read_text()) if path.exists() else default


def environment(directory):
    removed = (*AUTH_ENV, 'ROUTER_EXECUTABLE', 'ROUTER_COMMAND_NAME', 'ROUTER_LAUNCH_PREVIEW', 'ROUTER_DEFAULT_API_MODEL')
    result = {key: value for key, value in os.environ.items() if key not in removed}
    if Path(directory).resolve() != (Path.home() / '.claude').resolve():
        result['CLAUDE_CONFIG_DIR'] = str(directory)
    return result


def account_status(program, account):
    result = subprocess.run(
        [program, '--setting-sources', 'user', 'auth', 'status', '--json'],
        env=environment(account['directory']), capture_output=True, text=True, timeout=15,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(f"{account['label']}：官方客户端状态查询失败（{result.returncode}）")
    data = json.loads(result.stdout)
    return {
        'id': account['id'], 'label': account['label'], 'directory': account['directory'],
        'authenticated': data.get('loggedIn') is True and data.get('authMethod') == 'claude.ai',
        'email': data.get('email'), 'subscription': data.get('subscriptionType'),
        'organization': data.get('orgId'),
        'expires_at': data.get('loginExpiresAt'),
    }


def settings(config, name, validate_accounts=True):
    command = config['management']['commands'][name]
    template = config['management']['templates'][command['template']]
    configured = {**template.get('env', {}), **command.get('env', {})}
    if not validate_accounts:
        return configured, []
    accounts = json.loads(configured.get('ROUTER_SUBSCRIPTION_ACCOUNTS', '[]'))
    if not isinstance(accounts, list):
        raise ValueError('账号配置应为列表')
    seen = set()
    directories = set()
    for account in accounts:
        identifier = account['id']
        if not identifier or not all(character.isascii() and (character.isalnum() or character in '-_') for character in identifier) or identifier in seen:
            raise ValueError('账号标识须唯一，并由英文字母、数字、短横线或下划线组成')
        directory = Path(account['directory']).expanduser()
        if not directory.is_absolute():
            raise ValueError('账号配置目录请填写绝对路径')
        directory = directory.resolve()
        if directory in directories or any((parent / '.git').exists() for parent in (directory, *directory.parents)):
            raise ValueError('各账号使用独立目录，并放在 Git 工作区之外')
        account['directory'] = str(directory)
        account['label'] = account.get('label') or identifier
        seen.add(identifier)
        directories.add(directory)
    return configured, accounts


class NativeOption(argparse.Action):
    def __call__(self, parser, namespace, values, option_string=None):
        parts = values if isinstance(values, list) else [] if values is None else [values]
        namespace.records.append([option_string, *parts])


def native_arguments(arguments):
    specification = read(Path(__file__).with_name('claude-cli-options.json'), None)
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.set_defaults(records=[])
    for group, count in [('zero', 0), ('one', None), ('optional', '?'), ('many', '+')]:
        for names in specification[group]:
            parser.add_argument(*names, nargs=count, action=NativeOption)
    parser.add_argument('prompt', nargs='?')
    parsed = parser.parse_args(arguments)
    initial = [value for row in parsed.records for value in row]
    resumed = [value for row in parsed.records if row[0] not in specification['resume_remove'] for value in row]
    return initial, resumed, parsed.prompt


def manage(program, config_path, name, configured, accounts, action, selected, session=None):
    state_directory = config_path.parent / 'client-state' / 'subscriptions' / name
    if action == 'status':
        sessions = []
        if state_directory.exists():
            for path in state_directory.glob('session-*.json'):
                state = read(path, {})
                pid = state.get('supervisor_pid')
                if state.get('state') in ('running', 'switching', 'exhausted') and pid:
                    try:
                        os.kill(pid, 0)
                    except ProcessLookupError:
                        state['state'] = 'interrupted'
                sessions.append(state)
        sessions.sort(key=lambda item: item.get('started_at',0), reverse=True)
        return {'accounts': [account_status(program, account) for account in accounts], 'sessions': sessions}
    account = next((item for item in accounts if item['id'] == selected), None)
    if account is None:
        raise ValueError('请选择账号')
    if action == 'switch':
        if not session:
            raise ValueError('请选择正在运行的会话')
        run_id = str(uuid.UUID(session))
        current = read(state_directory / f'session-{run_id}.json', {})
        control = read(Path(current['control_file']), None) if current.get('control_file') else None
        if control is None:
            raise ValueError('此会话已结束，请刷新会话状态')
        with socket.create_connection(('127.0.0.1', control['port']), timeout=5) as connection:
            connection.settimeout(20)
            connection.sendall(json.dumps({'nonce':control['nonce'],'action':'switch','account':selected}).encode() + b'\n')
            with connection.makefile('rb') as source:
                result = json.loads(source.readline(65536))
        if result.get('error'):
            raise ValueError(result['error'])
        return result
    if action == 'logout':
        result = subprocess.run([program, 'auth', 'logout'], env=environment(account['directory']), capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError('官方客户端退出登录失败')
        return {'logged_out': True}
    if action != 'login':
        raise ValueError('账号操作无效')
    directory = Path(account['directory'])
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory.chmod(0o700)
    state_directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    launcher = state_directory / f"login-{account['id']}.command"
    arguments = ['/usr/bin/env', *[item for key in AUTH_ENV for item in ('-u', key)]]
    selected_environment = environment(directory)
    if 'CLAUDE_CONFIG_DIR' in selected_environment:
        arguments.append(f'CLAUDE_CONFIG_DIR={directory}')
    arguments.extend([program, 'auth', 'login', '--claudeai'])
    launcher.write_text('#!/bin/zsh\nexec ' + shlex.join(arguments) + '\n')
    launcher.chmod(0o700)
    subprocess.run(['/usr/bin/open', '-a', 'Terminal', str(launcher)], check=True)
    return {'login_started': True, 'account': selected, 'message': '在官方客户端打开的浏览器中完成登录，然后刷新状态。'}


def emit_event(mode):
    payload = json.load(sys.stdin)
    message = {key: payload[key] for key in ('session_id', 'transcript_path', 'cwd', 'hook_event_name', 'error', 'agent_id') if key in payload}
    if mode == 'statusline':
        message['hook_event_name'] = 'StatusLine'
        message['rate_limits'] = payload.get('rate_limits') or {}
    message['nonce'] = os.environ['ROUTER_SUBSCRIPTION_NONCE']
    with socket.create_connection(('127.0.0.1', int(os.environ['ROUTER_SUBSCRIPTION_PORT'])), timeout=2) as connection:
        connection.sendall(json.dumps(message).encode() + b'\n')
    if mode == 'statusline':
        previous = os.environ.get('ROUTER_SUBSCRIPTION_STATUSLINE')
        if previous:
            output = subprocess.run(previous, shell=True, input=json.dumps(payload), text=True, capture_output=True, timeout=10)
            if output.returncode:
                raise RuntimeError('原有状态栏命令执行失败')
            print(output.stdout.rstrip(), end=' | ')
        print('账号：' + os.environ['ROUTER_SUBSCRIPTION_LABEL'])


def merge(left, right):
    for key, value in right.items():
        if isinstance(value, dict) and isinstance(left.get(key), dict):
            merge(left[key], value)
        else:
            left[key] = value
    return left


def write_terminal(descriptor, value):
    remaining = memoryview(value)
    while remaining:
        remaining = remaining[os.write(descriptor, remaining):]


def supervise(program, config_path, name, configured, accounts, arguments, selected, explicit_settings, initial_prompt, launcher):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ValueError('订阅账号自动切换入口请在交互终端中运行')
    if any(argument in ('--bare', '--safe-mode', '-p', '--print', '--no-session-persistence', '--bg', '--background', '--desktop') for argument in arguments):
        raise ValueError('自动切换使用官方客户端的本机交互会话、Hook 和会话记录，请使用常规交互启动参数')
    arguments, resume_arguments, native_prompt = native_arguments(arguments)
    if initial_prompt is None:
        initial_prompt = native_prompt
    order = list(accounts)
    if selected:
        chosen = next((item for item in accounts if item['id'] == selected), None)
        if chosen is None:
            raise ValueError('指定的账号标识不存在')
        order = [chosen, *[item for item in accounts if item is not chosen]]
    statuses = {item['id']: account_status(program, item) for item in order}
    if selected and not statuses[selected]['authenticated']:
        raise ValueError(f"请先通过官方客户端登录 {order[0]['label']}")
    available = [item for item in order if statuses[item['id']]['authenticated']]
    if not available:
        raise ValueError('请先通过官方客户端登录主账号或备用账号')
    account = available[0]
    attempted = {account['id']}
    identities = {(statuses[account['id']]['email'], statuses[account['id']]['organization'])}
    state_directory = config_path.parent / 'client-state' / 'subscriptions' / name
    state_directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    run_id = str(uuid.uuid4())
    state_path = state_directory / f'session-{run_id}.json'
    credential_directory = Path(read(config_path, None)['management']['credential_directory']).expanduser()
    if not credential_directory.is_absolute():
        credential_directory = config_path.parent / credential_directory
    credential_directory = credential_directory.resolve()
    if any((parent / '.git').exists() for parent in (credential_directory, *credential_directory.parents)):
        raise ValueError('会话控制凭据目录应位于 Git 工作区之外')
    control_path = credential_directory / 'session-control' / name / f'{run_id}.json'
    state = {'run_id':run_id, 'control_file':str(control_path), 'state': 'running', 'account': account['id'], 'supervisor_pid': os.getpid(), 'started_at': int(time.time()), 'switches': []}
    save(state_path, state)
    snapshot = read(Path(account['directory']) / 'settings.json', {})
    for value in explicit_settings:
        merge(snapshot, json.loads(value) if value.lstrip().startswith('{') else read(Path(value).expanduser(), {}))
    snapshot['forceLoginMethod'] = 'claudeai'
    snapshot.pop('apiKeyHelper', None)
    for key in AUTH_ENV:
        snapshot.get('env', {}).pop(key, None)
    if snapshot.get('disableAllHooks'):
        raise ValueError('当前账号设置关闭了 Hook，请启用 Hook 后使用自动切换')
    previous_statusline = (snapshot.get('statusLine') or {}).get('command', '')
    hook_command = shlex.join([sys.executable, str(Path(__file__).resolve()), '--event', 'hook'])
    for event, matcher in [('SessionStart', ''), ('UserPromptSubmit', ''), ('Stop', ''), ('StopFailure', '')]:
        snapshot.setdefault('hooks', {}).setdefault(event, []).append({'matcher': matcher, 'hooks': [{'type': 'command', 'command': hook_command, 'timeout': 5}]})
    snapshot['statusLine'] = {'type': 'command', 'command': shlex.join([sys.executable, str(Path(__file__).resolve()), '--event', 'statusline'])}
    settings_file = state_directory / f"launch-{os.getpid()}.json"
    save(settings_file, snapshot)
    nonce = secrets.token_urlsafe(32)
    server = socket.socket()
    server.bind(('127.0.0.1', 0))
    server.listen(4)
    server.setblocking(False)
    save(control_path, {'port':server.getsockname()[1], 'nonce':nonce})
    selector = selectors.DefaultSelector()
    selector.register(server, selectors.EVENT_READ, 'event')
    selector.register(sys.stdin.fileno(), selectors.EVENT_READ, 'input')
    original_terminal = termios.tcgetattr(sys.stdin.fileno())
    child_pid = None
    child_fd = None
    transcript = None
    resume = False
    resume_directory = None
    turn_active = False
    queued_account = None
    queued_identity = None
    child_arguments = list(arguments)
    def resize(_signum=None, _frame=None):
        if child_fd is not None:
            size = fcntl.ioctl(sys.stdin.fileno(), termios.TIOCGWINSZ, b'\0' * 8)
            fcntl.ioctl(child_fd, termios.TIOCSWINSZ, size)
    old_resize = signal.signal(signal.SIGWINCH, resize)
    def terminate(signum, _frame):
        raise SystemExit(128 + signum)
    old_termination = signal.signal(signal.SIGTERM, terminate)
    old_hangup = signal.signal(signal.SIGHUP, terminate)
    try:
        tty.setraw(sys.stdin.fileno())
        while True:
            child_environment = environment(account['directory'])
            child_environment.update({'ROUTER_SUBSCRIPTION_PORT': str(server.getsockname()[1]), 'ROUTER_SUBSCRIPTION_NONCE': nonce,
                                      'ROUTER_SUBSCRIPTION_LABEL': account['label'], 'ROUTER_SUBSCRIPTION_STATUSLINE': previous_statusline})
            launch_arguments = ['--settings', str(settings_file), *child_arguments]
            if initial_prompt is not None:
                launch_arguments.extend(['--', initial_prompt])
            if resume:
                launch_arguments = ['--settings', str(settings_file), *resume_arguments, '--resume', transcript, '--', configured['ROUTER_SUBSCRIPTION_RESUME_PROMPT']]
            child_pid, child_fd = pty.fork()
            if child_pid == 0:
                if resume_directory:
                    os.chdir(resume_directory)
                os.execvpe(launcher[0], [*launcher, *launch_arguments], child_environment)
            resize()
            selector.register(child_fd, selectors.EVENT_READ, 'output')
            next_account = None
            switch_reason = 'rate_limit'
            stop_at = None
            stopped = False
            exited = False
            result = None
            while not exited:
                timeout = min(0.5, max(0, stop_at - time.monotonic())) if stop_at is not None else 0.5
                for key, _ in selector.select(timeout):
                    if key.data == 'input':
                        incoming = os.read(sys.stdin.fileno(), 16384)
                        if incoming:
                            write_terminal(child_fd, incoming)
                    elif key.data == 'output':
                        try:
                            outgoing = os.read(child_fd, 65536)
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                            outgoing = b''
                        if outgoing:
                            write_terminal(sys.stdout.fileno(), outgoing)
                        else:
                            exited = True
                    else:
                        connection, _ = server.accept()
                        with connection:
                            connection.settimeout(2)
                            try:
                                with connection.makefile('rb') as source:
                                    event = json.loads(source.readline(65536))
                            except (ValueError, socket.timeout):
                                continue
                            if not isinstance(event, dict) or not isinstance(event.get('nonce'), str) or not secrets.compare_digest(event['nonce'], nonce):
                                continue
                            if event.get('action') == 'switch':
                                _, current_accounts = settings(read(config_path, None), name)
                                candidate = next((item for item in current_accounts if item['id'] == event.get('account')), None)
                                if not transcript:
                                    answer = {'error':'会话正在初始化，请稍后切换'}
                                elif candidate is None:
                                    answer = {'error':'请选择该命令配置的账号'}
                                elif candidate['id'] == account['id']:
                                    answer = {'accepted':True,'message':'当前已在使用此账号'}
                                elif next_account:
                                    answer = {'error':'正在切换账号，请等待完成'}
                                else:
                                    status = account_status(program, candidate)
                                    statuses[candidate['id']] = status
                                    if not status['authenticated']:
                                        answer = {'error':'请先通过官方客户端登录此账号'}
                                    else:
                                        queued_account = candidate
                                        queued_identity = (status['email'],status['organization'])
                                        order = current_accounts
                                        state['requested_account'] = candidate['id']
                                        save(state_path, state)
                                        answer = {'accepted':True,'message':'当前回复结束后切换账号' if turn_active else '正在切换账号并恢复会话'}
                                connection.sendall(json.dumps(answer, ensure_ascii=False).encode() + b'\n')
                                continue
                        if event.get('agent_id'):
                            continue
                        if event.get('hook_event_name') == 'SessionStart':
                            transcript = event['transcript_path']
                            resume_directory = event['cwd']
                            turn_active = False
                            state.update({'session_id': event['session_id'], 'transcript_path': transcript, 'cwd':event['cwd']})
                            save(state_path, state)
                        elif event.get('hook_event_name') == 'StatusLine':
                            limits = event.get('rate_limits', {})
                            state['rate_limits'] = limits
                        elif event.get('hook_event_name') == 'UserPromptSubmit':
                            turn_active = True
                        elif event.get('hook_event_name') == 'Stop':
                            turn_active = False
                            attempted = {account['id']}
                            identities = {(statuses[account['id']]['email'], statuses[account['id']]['organization'])}
                            if state['state'] == 'exhausted':
                                state['state'] = 'running'
                                save(state_path, state)
                        elif event.get('hook_event_name') == 'StopFailure' and event.get('error') == 'rate_limit' and next_account is None:
                            turn_active = False
                            transcript = event['transcript_path']
                            resume_directory = event['cwd']
                            for candidate in order:
                                if candidate['id'] not in attempted:
                                    status = account_status(program, candidate)
                                    statuses[candidate['id']] = status
                                    identity = (status['email'], status['organization'])
                                    if status['authenticated'] and identity not in identities:
                                        next_account = candidate
                                        identities.add(identity)
                                        break
                            if next_account:
                                state['state'] = 'switching'
                                save(state_path, state)
                                stop_at = time.monotonic() + 0.2
                            elif state['state'] != 'exhausted':
                                state['state'] = 'exhausted'
                                save(state_path, state)
                                os.write(sys.stderr.fileno(), '\r\nRouter：本轮可用账号已达到限额，保留当前会话等待恢复。\r\n'.encode())
                        elif event.get('hook_event_name') == 'StopFailure':
                            turn_active = False
                if queued_account and not turn_active:
                    next_account = queued_account
                    switch_reason = 'manual'
                    attempted = set()
                    identities = {queued_identity}
                    queued_account = None
                    state['state'] = 'switching'
                    save(state_path, state)
                    stop_at = time.monotonic() + 0.2
                reaped, result = os.waitpid(child_pid, os.WNOHANG)
                if reaped:
                    exited = True
                if stop_at is not None and time.monotonic() >= stop_at and not exited:
                    os.kill(child_pid, signal.SIGKILL if stopped else signal.SIGTERM)
                    stop_at = None if stopped else time.monotonic() + 10
                    stopped = True
            if not reaped:
                _, result = os.waitpid(child_pid, 0)
            child_pid = None
            selector.unregister(child_fd)
            os.close(child_fd)
            child_fd = None
            if next_account is None:
                state['state'] = 'finished' if os.waitstatus_to_exitcode(result) == 0 else 'interrupted'
                save(state_path, state)
                return os.waitstatus_to_exitcode(result)
            if not Path(transcript).is_file():
                raise RuntimeError('当前会话记录暂不可读取，已保留原账号目录')
            state['switches'].append({'from': account['id'], 'to': next_account['id'], 'at': int(time.time()), 'reason': switch_reason})
            account = next_account
            attempted.add(account['id'])
            state.update({'state': 'running', 'account': account['id']})
            state.pop('requested_account', None)
            save(state_path, state)
            reason = '当前账号达到限额，' if switch_reason == 'rate_limit' else ''
            os.write(sys.stderr.fileno(), f"\r\nRouter：{reason}正在切换至 {account['label']} 并恢复会话。\r\n".encode())
            resume = True
    finally:
        if child_pid is not None:
            try:
                os.kill(child_pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 10
            while True:
                reaped, _ = os.waitpid(child_pid, os.WNOHANG)
                if reaped:
                    break
                if time.monotonic() >= deadline:
                    os.kill(child_pid, signal.SIGKILL)
                    os.waitpid(child_pid, 0)
                    break
                time.sleep(0.05)
        termios.tcsetattr(sys.stdin.fileno(), termios.TCSADRAIN, original_terminal)
        signal.signal(signal.SIGWINCH, old_resize)
        signal.signal(signal.SIGTERM, old_termination)
        signal.signal(signal.SIGHUP, old_hangup)
        if child_fd is not None:
            os.close(child_fd)
        selector.close()
        server.close()
        settings_file.unlink(missing_ok=True)
        control_path.unlink(missing_ok=True)
        if state['state'] in ('running', 'switching', 'exhausted'):
            state['state'] = 'interrupted'
            save(state_path, state)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--event':
        emit_event(sys.argv[2])
        return 0
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument('--manage-stdin', action='store_true')
    parser.add_argument('--router-mode', choices=['api', 'account'])
    parser.add_argument('--router-account')
    parser.add_argument('--router-session')
    parser.add_argument('--router-manage', choices=['status', 'login', 'logout', 'switch'])
    parser.add_argument('--router-prompt')
    parser.add_argument('--settings', action='append', default=[])
    options, arguments = parser.parse_known_args(sys.argv[1:])
    config_path = Path(os.environ['ROUTER_CONFIG']).resolve()
    name = os.environ['ROUTER_COMMAND_NAME']
    config = read(config_path, None)
    configured, _ = settings(config, name, False)
    program = configured['ROUTER_SUBSCRIPTION_CLIENT']
    specification = read(Path(__file__).with_name('claude-cli-options.json'), None)
    settings_arguments = [item for value in options.settings for item in ('--settings', value)]
    if options.manage_stdin or options.router_manage:
        configured, accounts = settings(config, name)
        request = json.load(sys.stdin) if options.manage_stdin else {'action':options.router_manage,'account':options.router_account,'session':options.router_session}
        try:
            result = manage(program, config_path, name, configured, accounts, request['action'], request.get('account'), request.get('session'))
        except (ValueError, RuntimeError, subprocess.TimeoutExpired, OSError) as error:
            print(str(error), file=sys.stderr)
            return 1
        print(json.dumps(result, ensure_ascii=False))
        return 0
    if arguments in (['--version'], ['-v'], ['--help'], ['-h']):
        os.execvp(program, [program, *arguments])
    command = config['management']['commands'][name]
    template = config['management']['templates'][command['template']]
    default_mode = command.get('options', {}).get('auth-mode') or template.get('options', {}).get('auth-mode', {}).get('default', 'account')
    mode = options.router_mode or ('account' if options.router_account else default_mode)
    maintenance = bool(arguments and arguments[0] in specification['commands'])
    if maintenance and arguments[0] in ('auth', 'setup-token'):
        mode = 'account'
    launch_environment = dict(os.environ)
    launch_environment['ROUTER_CLAUDE_MODE'] = mode
    if mode == 'api':
        if options.router_account:
            raise ValueError('选择账号时请使用账号接入方式')
        api_arguments = [*json.loads(configured.get('ROUTER_CLAUDE_API_ARGUMENTS', '[]')), *settings_arguments, *arguments]
        if options.router_prompt is not None:
            api_arguments.extend(['--',options.router_prompt])
        default_model = launch_environment.get('ROUTER_DEFAULT_API_MODEL')
        if default_model and not any(item == '--model' or item.startswith('--model=') for item in api_arguments):
            api_arguments = ['--model',default_model,*api_arguments]
        launcher = json.loads(configured.get('ROUTER_CLAUDE_API_LAUNCHER', '[]'))
        if launcher:
            os.execvpe(launcher[0], [*launcher, *api_arguments], launch_environment)
        router = os.environ['ROUTER_EXECUTABLE']
        os.execvpe(router, [router,'--config',str(config_path),'__exec',program,*api_arguments], launch_environment)
    configured, accounts = settings(config, name)
    if not accounts:
        raise ValueError('请在此命令中添加并登录主账号或备用账号')
    chosen = next((account for account in accounts if account['id'] == options.router_account), None) if options.router_account else accounts[0]
    if chosen is None:
        raise ValueError('指定的账号标识不存在')
    if maintenance:
        os.execvpe(program, [program,*settings_arguments,*arguments], environment(chosen['directory']))
    native_args = json.loads(configured.get('ROUTER_CLAUDE_NATIVE_ARGS', '[]'))
    native_args.extend(arguments)
    direct = any(item.split('=',1)[0] in specification['direct_flags'] for item in native_args)
    if os.environ.get('ROUTER_LAUNCH_PREVIEW'):
        print(json.dumps({'program':program,'mode':'account','primary':chosen['id'],'backups':[item['id'] for item in accounts if item != chosen],
                          'args':[*settings_arguments,*native_args],'account_directories':[item['directory'] for item in accounts],
                          'switch_on':None if direct else 'StopFailure:rate_limit'}, ensure_ascii=False))
        return 0
    launcher = [program]
    if command.get('mod_profile'):
        router = os.environ['ROUTER_EXECUTABLE']
        prepared = subprocess.run([router,'--config',str(config_path),'__exec',program],
                                  env={**launch_environment,'ROUTER_LAUNCH_PREVIEW':'1'}, capture_output=True, text=True, timeout=45, check=True)
        launch = json.loads(prepared.stdout)
        launcher = [launch['program'], *launch['args']]
    if direct or not sys.stdin.isatty() or not sys.stdout.isatty():
        if options.router_prompt is not None:
            native_args.extend(['--',options.router_prompt])
        os.execvpe(launcher[0], [*launcher,*settings_arguments,*native_args], environment(chosen['directory']))
    return supervise(program, config_path, name, configured, accounts, native_args, options.router_account, options.settings, options.router_prompt, launcher)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except ValueError as error:
        print(f'Router：{error}', file=sys.stderr)
        sys.exit(1)
