import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from urllib.parse import urlsplit

import requests


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("config", type=Path)
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    token_file = Path(config["token_file"])
    if token_file.stat().st_mode & 0o077:
        raise ValueError("凭据文件权限必须为 0600")
    token = token_file.read_text().strip()
    if not token:
        raise ValueError("服务凭据为空")
    endpoint = urlsplit(config["health_url"])
    if endpoint.scheme != "http" or endpoint.hostname not in ("127.0.0.1", "::1", "localhost"):
        raise ValueError("本机服务检查地址必须使用回环 HTTP 地址")
    if endpoint.username or endpoint.password:
        raise ValueError("服务检查地址不能包含凭据")
    headers = {name:value.replace("{token}", token) for name,value in config["health_headers"].items()}
    timeout = config["ready_timeout_seconds"]
    interval = config["poll_interval_seconds"]
    probe_timeout = config["probe_timeout_seconds"]
    if min(timeout, interval, probe_timeout) <= 0:
        raise ValueError("服务检查时间参数必须大于零")
    lock_file = Path(config["lock_file"])
    lock_file.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with requests.Session() as session, lock_file.open("a") as lock:
        os.fchmod(lock.fileno(), 0o600)
        session.trust_env = False

        def ready():
            try:
                response = session.get(config["health_url"], headers=headers, timeout=probe_timeout, allow_redirects=False)
            except requests.ConnectionError:
                return False
            if response.status_code != config["health_status"]:
                raise RuntimeError("本机服务认证检查失败，HTTP " + str(response.status_code))
            return True

        fcntl.flock(lock, fcntl.LOCK_EX)
        if not ready():
            command = config["start_command"]
            if not command or not all(isinstance(part, str) and part for part in command):
                raise ValueError("服务启动命令无效")
            subprocess.run(command, check=True, stdout=subprocess.DEVNULL, timeout=timeout)
            deadline = time.monotonic() + timeout
            while not ready():
                if time.monotonic() >= deadline:
                    raise RuntimeError("本机适配服务启动超时")
                time.sleep(interval)
        print(token)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, KeyError, requests.RequestException, subprocess.SubprocessError) as error:
        print("服务凭据读取失败：" + str(error), file=sys.stderr)
        raise SystemExit(1)
