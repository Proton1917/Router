import json
import os
import signal
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

from packaging.version import Version


def read_json(path):
    return json.loads(Path(path).read_text())


def client_info(mods):
    program = mods["settings"]["program"]
    version = subprocess.check_output([program, "--version"], text=True, timeout=10).strip()
    if Version(version.split()[0]) < Version(mods["settings"]["minimum_version"]):
        raise ValueError("当前客户端版本尚不支持 Mods，请先更新 Claude Code")
    result = subprocess.run([program, "plugin", "list", "--json"], capture_output=True, text=True, timeout=30, check=True)
    installed = json.loads(result.stdout)
    if not isinstance(installed, list):
        raise ValueError("客户端插件清单格式不受支持")
    return program, version, installed


def validate_entry(program, entry):
    if entry.get("path"):
        path = local_path(entry)
        result = subprocess.run([program, "plugin", "validate", str(path)], capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise ValueError("Mod 校验失败：" + result.stdout.strip() + result.stderr.strip())
        if path.is_dir():
            manifest = read_json(path / ".claude-plugin/plugin.json")
        else:
            with zipfile.ZipFile(path) as archive:
                manifest = json.loads(archive.read(".claude-plugin/plugin.json"))
        if entry["plugin_id"] != manifest["name"] + "@inline":
            raise ValueError("本地 Mod 的插件标识应为清单中的 name 加 @inline")
        return result.stdout.strip()
    return "内置或已安装插件的可用条件由 Claude Code 在启动时检查。"


def local_path(entry):
    path = Path(entry["path"]).expanduser()
    if not path.is_absolute():
        raise ValueError("本地 Mod 路径必须是绝对路径，可以使用 ~ 表示用户目录")
    return path.resolve(strict=True)


def merge(base, extra):
    for key, value in extra.items():
        if isinstance(value, dict) and isinstance(base.get(key), dict):
            merge(base[key], value)
        else:
            base[key] = value
    return base


def prepare(mods, profile_id, args):
    profile = mods["profiles"][profile_id]
    settings = {}
    remaining = []
    iterator = iter(args)
    for arg in iterator:
        if arg == "--settings" or arg.startswith("--settings="):
            source = next(iterator, None) if arg == "--settings" else arg.split("=", 1)[1]
            if source is None:
                raise ValueError("--settings 缺少参数")
            value = json.loads(source) if source.lstrip().startswith("{") else read_json(Path(source).expanduser())
            if not isinstance(value, dict):
                raise ValueError("客户端设置必须是 JSON 对象")
            merge(settings, value)
        else:
            remaining.append(arg)
    overrides = {}
    directories = []
    for key, enabled in profile.get("plugins", {}).items():
        entry = mods["catalog"][key]
        overrides[entry["plugin_id"]] = enabled
        if enabled and entry.get("path"):
            validate_entry(mods["settings"]["program"], entry)
            directories.extend(["--plugin-dir", str(local_path(entry))])
    merge(settings, {"enabledPlugins": overrides})
    return settings, directories + remaining


def launch():
    config_path, command_name, separator, program, *args = sys.argv[2:]
    if separator != "--":
        raise ValueError("缺少客户端参数分隔符")
    config = read_json(config_path)
    mods = config["management"]["mods"]
    profile = config["management"]["commands"][command_name]["mod_profile"]
    client_info(mods)
    settings, args = prepare(mods, profile, args)
    directory = Path(mods["settings"]["launch_directory"]).expanduser()
    if not directory.is_absolute():
        directory = Path(config_path).resolve().parent / directory
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory.chmod(0o700)
    descriptor, temporary = tempfile.mkstemp(prefix="settings-", suffix=".json", dir=directory)
    child = None
    pending_signal = None
    try:
        with os.fdopen(descriptor, "w") as stream:
            json.dump(settings, stream, ensure_ascii=False)
        signal.signal(signal.SIGINT, lambda *_: None)
        def forward(signum, _frame):
            nonlocal pending_signal
            pending_signal = signum
            if child is not None:
                child.send_signal(signum)
        signal.signal(signal.SIGTERM, forward)
        signal.signal(signal.SIGHUP, forward)
        child = subprocess.Popen([program, "--settings", temporary, *args])
        if pending_signal is not None:
            child.send_signal(pending_signal)
        code = child.wait()
        return code if code >= 0 else 128 - code
    finally:
        Path(temporary).unlink(missing_ok=True)


def action(request):
    mods = request["mods"]
    program, version, installed = client_info(mods)
    if request["action"] == "inspect":
        return {"ok": True, "version": version, "installed": installed, "catalog": mods.get("catalog", {})}
    if request["action"] == "validate":
        entry = mods["catalog"][request["entry"]]
        return {"ok": True, "message": validate_entry(program, entry)}
    raise ValueError("未知 Mods 操作")


if len(sys.argv) > 1 and sys.argv[1] == "launch":
    try:
        sys.exit(launch())
    except (ValueError, KeyError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        print("router: Mods 启动失败：" + str(error), file=sys.stderr)
        sys.exit(1)
else:
    try:
        response = action(json.load(sys.stdin))
    except (ValueError, KeyError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        response = {"ok": False, "error": "Mods 操作失败：" + str(error)}
    print(json.dumps(response, ensure_ascii=False))
