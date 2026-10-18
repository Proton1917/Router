import argparse
import fcntl
import json
import os
from pathlib import Path
import plistlib
import stat
import subprocess
import sys


def load(config_path):
    config_path = Path(config_path).resolve()
    management = json.loads(config_path.read_text())['management']
    directory = Path(management['log_directory'])
    if not directory.is_absolute():
        directory = config_path.parent / directory
    policy_path = config_path.parent / 'log-retention.json'
    policy = json.loads(policy_path.read_text())
    for field in ('check_interval_seconds', 'max_file_bytes', 'retain_tail_bytes', 'max_total_bytes'):
        if type(policy[field]) is not int or policy[field] <= 0:
            raise ValueError(field + ' 必须为正整数')
    if policy['retain_tail_bytes'] >= policy['max_file_bytes'] or policy['max_file_bytes'] > policy['max_total_bytes']:
        raise ValueError('日志保留尾部需小于单文件上限，单文件上限需不超过总量上限')
    return directory.resolve(), policy


def clean(config_path, report=False):
    directory, policy = load(config_path)
    directory.mkdir(parents=True, exist_ok=True)
    changed = []
    with (directory / '.log-maintenance.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        files = [(path, path.stat().st_size) for path in directory.glob('*.log')
                 if not path.is_symlink() and path.is_file()]
        total = before = sum(size for _, size in files)
        for path, size in sorted(files, key=lambda item: item[1], reverse=True):
            if size <= policy['max_file_bytes'] and total <= policy['max_total_bytes']:
                continue
            retain = min(policy['retain_tail_bytes'], size,
                         max(0, policy['max_total_bytes'] - (total - size)))
            descriptor = os.open(path, os.O_RDWR | os.O_NOFOLLOW)
            with os.fdopen(descriptor, 'r+b') as output:
                current = os.fstat(output.fileno())
                if not stat.S_ISREG(current.st_mode):
                    raise ValueError('日志路径必须为普通文件：' + str(path))
                tail = os.pread(output.fileno(), retain, max(0, current.st_size - retain))
                if current.st_size > retain:
                    boundary = tail.find(b'\n')
                    tail = tail[boundary + 1:] if boundary >= 0 else b''
                output.seek(0)
                output.write(tail)
                output.truncate(len(tail))
            total -= size - len(tail)
            changed.append({'file': path.name, 'before_bytes': current.st_size, 'after_bytes': len(tail)})
    if report:
        print(json.dumps({'before_bytes': before, 'after_bytes': total, 'changed': changed}, ensure_ascii=False))


def install(service_path):
    service_path = Path(service_path).resolve()
    with service_path.open('rb') as source:
        service = plistlib.load(source)
    arguments = service['ProgramArguments']
    config_path = Path(arguments[arguments.index('--config') + 1]).resolve()
    policy_path = config_path.parent / 'log-retention.json'
    if not policy_path.exists():
        defaults = Path(__file__).with_name('log-retention.json').read_bytes()
        with policy_path.open('xb') as destination:
            destination.write(defaults)
        policy_path.chmod(0o600)
    directory, policy = load(config_path)
    directory.mkdir(parents=True, exist_ok=True)
    label = service['Label'] + '.logs'
    if '/' in label or not label:
        raise ValueError('日志维护服务名称无效')
    agent = Path.home() / 'Library/LaunchAgents' / (label + '.plist')
    agent.parent.mkdir(parents=True, exist_ok=True)
    definition = {'Label': label,
                  'ProgramArguments': [sys.executable, str(Path(__file__).resolve()), 'run', '--config', str(config_path)],
                  'RunAtLoad': True, 'StartInterval': policy['check_interval_seconds'],
                  'ProcessType': 'Background', 'LowPriorityIO': True, 'Nice': 10,
                  'StandardOutPath': '/dev/null', 'StandardErrorPath': str(directory / 'log-maintenance.log')}
    encoded = plistlib.dumps(definition)
    clean(config_path, report=True)
    domain = 'gui/' + str(os.getuid())
    identity = domain + '/' + label
    current = subprocess.run(['launchctl', 'print', identity], capture_output=True)
    if not agent.exists() or agent.read_bytes() != encoded:
        if current.returncode == 0:
            subprocess.run(['launchctl', 'bootout', identity], check=True)
        agent.write_bytes(encoded)
        agent.chmod(0o600)
        subprocess.run(['launchctl', 'bootstrap', domain, str(agent)], check=True)
    elif current.returncode != 0:
        subprocess.run(['launchctl', 'bootstrap', domain, str(agent)], check=True)


parser = argparse.ArgumentParser(description='按配置限制 Router 日志文件及总量')
actions = parser.add_subparsers(dest='action', required=True)
installer = actions.add_parser('install')
installer.add_argument('--service-plist', required=True)
runner = actions.add_parser('run')
runner.add_argument('--config', required=True)
runner.add_argument('--report', action='store_true')
options = parser.parse_args()
if options.action == 'install':
    install(options.service_plist)
else:
    clean(options.config, options.report)
