import argparse
import atexit
from contextlib import ExitStack
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, stream, code, message, headers, url):
        raise RuntimeError('Router 管理接口返回了重定向')


def main():
    parser = argparse.ArgumentParser(prog=os.environ.get('ROUTER_COMMAND_NAME', 'spt'), description='通过 Router 后台转写音频或识别图片，在原文件旁生成 Markdown。')
    parser.add_argument('input', nargs='?', help='音频路径，或 ocr / config / models')
    parser.add_argument('image', nargs='?', help='ocr 的图片路径')
    parser.add_argument('--raw', action='store_true', help='保留原始转写，输出 .raw.md')
    parser.add_argument('--force', action='store_true', help='完整成功后原子替换已有文字稿')
    parser.add_argument('--model', help='选择已配置的模型并保存为默认值；单独使用时列出可选模型', nargs='?', const='')
    args = parser.parse_args()
    if args.input == 'help':
        parser.print_help()
        return 0
    path = Path(os.environ['ROUTER_CONFIG']).resolve(strict=True)
    config = json.loads(path.read_text())
    manager = config['management']
    token_path = Path(manager['token_file'])
    if not token_path.is_absolute():
        token_path = path.parent / token_path
    base = 'http://' + manager['listen'] + '/api/'
    parsed = urllib.parse.urlsplit(base)
    if parsed.hostname not in ('127.0.0.1', 'localhost', '::1'):
        raise RuntimeError('SPT 管理入口必须是本机回环地址')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def request(endpoint, value=None, data=None, headers=None):
        authorization = 'Bearer ' + token_path.read_text().strip()
        body = json.dumps(value).encode() if value is not None else data
        fields = {'Authorization': authorization, **(headers or {})}
        if value is not None:
            fields['Content-Type'] = 'application/json'
        operation = urllib.request.Request(base + endpoint, data=body, headers=fields)
        try:
            with opener.open(operation, timeout=120) as response:
                content = response.read()
                if response.headers.get_content_type() == 'application/json':
                    return json.loads(content)
                return content
        except urllib.error.HTTPError as error:
            detail = error.read(65536)
            try:
                message = json.loads(detail).get('error', detail.decode(errors='replace'))
            except json.JSONDecodeError:
                message = detail.decode(errors='replace')
            raise RuntimeError(message) from error

    try:
        state = request('state')
    except (urllib.error.URLError, FileNotFoundError):
        subprocess.run([os.environ['ROUTER_EXECUTABLE'], '--config', str(path), 'web', '--no-open'], check=True, stdout=subprocess.DEVNULL)
        state = request('state')
    settings = state['config']['management'].get('transcription')
    if not settings:
        raise RuntimeError('请先在 Router 的语音转写页面配置引擎和模型')
    models = settings['models']
    if args.input == 'config':
        print(json.dumps(settings, ensure_ascii=False, indent=2))
        return 0
    if args.input == 'models' or args.model == '':
        for name, model in models.items():
            destination = state['config']['profiles'][model['profile']]['standard']
            print(f'{name}\t{destination["model"]}\t{destination["target"]}\t{destination.get("provider", "")}')
        return 0
    selected = args.model or settings['default_model']
    if selected not in models:
        matches = [name for name, model in models.items() if selected in (model['profile'], state['config']['profiles'][model['profile']]['standard'].get('model'))]
        if len(matches) != 1:
            raise RuntimeError('模型名称不存在或有多个渠道，请用 spt models 查看可选名称')
        selected = matches[0]
    if args.model:
        settings['default_model'] = selected
        preview = request('preview', {'revision': state['revision'], 'config': state['config']})
        request('apply', {'revision': state['revision'], 'config': preview['config']})
        if args.input is None:
            print(f'默认模型已保存：{selected}')
            return 0
    if args.input is None:
        parser.print_help()
        return 0
    kind = 'ocr' if args.input == 'ocr' else 'audio'
    if kind == 'ocr' and not args.image:
        parser.error('ocr 需要图片路径')
    if kind == 'audio' and args.image:
        parser.error('一次处理一个音频文件')
    source = Path(args.image if kind == 'ocr' else args.input).absolute()
    if source.is_symlink() or not source.is_file():
        raise RuntimeError('输入必须是普通本地文件')
    suffix = '.ocr.md' if kind == 'ocr' else '.raw.md' if args.raw else '.md'
    output = source.with_suffix(suffix)
    lifetime = ExitStack()
    atexit.register(lifetime.close)
    lock = os.open(output.parent, os.O_RDONLY | os.O_DIRECTORY)
    lifetime.callback(os.close, lock)
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    if output.is_symlink() or (output.exists() and not output.is_file()):
        raise RuntimeError('输出目标必须是普通文件')
    if output.exists() and not args.force:
        raise RuntimeError(f'输出已存在：{output}；使用 --force 可在成功后替换')
    temporary = lifetime.enter_context(tempfile.NamedTemporaryFile(dir=output.parent, prefix='.spt-', delete=False))
    staged = Path(temporary.name)
    lifetime.callback(staged.unlink, missing_ok=True)
    interrupted = False

    def cancel_signal(number, frame):
        nonlocal interrupted
        interrupted = True

    signal.signal(signal.SIGINT, cancel_signal)
    signal.signal(signal.SIGTERM, cancel_signal)
    query = urllib.parse.urlencode({'name': source.name, 'model': selected, 'mode': 'raw' if args.raw else 'quality', 'kind': kind})
    with os.fdopen(os.open(source, os.O_RDONLY | os.O_NOFOLLOW), 'rb') as audio:
        size = os.fstat(audio.fileno()).st_size
        if size > settings['max_upload_bytes']:
            raise RuntimeError('输入文件超过 Router 配置的上传上限')
        job = request('transcription/jobs?' + query, data=audio, headers={'Content-Type':'application/octet-stream', 'Content-Length':str(size)})
    print(f'任务已提交：{job["id"]} · {selected}', file=sys.stderr)
    previous = None
    cancellation_sent = False
    while job['status'] not in ('succeeded', 'failed', 'cancelled', 'interrupted'):
        if interrupted and not cancellation_sent:
            request('transcription/jobs/' + job['id'], {'action':'cancel'})
            cancellation_sent = True
        current = (job['status'], job['stage'], job['completed_ms'], job['message'])
        if current != previous:
            print(f'{job["status"]} · {job["stage"]} · {job["completed_ms"] / 1000:g}/{job["total_ms"] / 1000:g} 秒 · {job["message"]}', file=sys.stderr)
            previous = current
        time.sleep(0.5)
        jobs = request('transcription/jobs')['jobs']
        job = next(item for item in jobs if item['id'] == job['id'])
    if job['status'] in ('cancelled', 'interrupted'):
        print(job['message'], file=sys.stderr)
        return 130 if job['status'] == 'cancelled' else 125
    if job['status'] != 'succeeded':
        raise RuntimeError(job['message'])
    content = request('transcription/jobs/' + job['id'] + '/markdown')
    temporary.write(content)
    temporary.flush()
    os.fsync(temporary.fileno())
    if output.is_symlink() or (output.exists() and not output.is_file()):
        raise RuntimeError('输出目标已变为非普通文件')
    if args.force:
        os.replace(staged, output)
    else:
        os.link(staged, output)
    print(output)
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'spt: {error}', file=sys.stderr)
        raise SystemExit(1)
