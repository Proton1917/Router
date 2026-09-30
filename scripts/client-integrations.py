import fcntl
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse


def read_json(path):
    return json.loads(Path(path).read_text())


def atomic_json(path, value, backup_directory, expected):
    path = Path(path)
    content = json.dumps(value, ensure_ascii=False, indent=2) + "\n"
    current = read_json(path)
    if current != expected:
        raise RuntimeError("客户端配置已被其他程序修改，请重新应用")
    if current == value:
        return False
    backup_directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(backup_directory, 0o700)
    if path.exists():
        digest = hashlib.sha256(path.read_bytes()).hexdigest()[:16]
        backup = backup_directory / (path.name + "." + digest)
        if not backup.exists():
            shutil.copyfile(path, backup)
            os.chmod(backup, 0o600)
    descriptor, temporary = tempfile.mkstemp(prefix=".router-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return True


def gateway_check(spec, settings):
    parsed = urlparse(spec["gateway_url"])
    if parsed.scheme not in ("http", "https") or not parsed.hostname:
        raise ValueError("接入地址必须是完整的 HTTP 或 HTTPS URL")
    if spec.get("require_https") and parsed.scheme != "https":
        raise ValueError("此客户端必须使用 HTTPS 网关")
    result = subprocess.run([
        settings["http_program"], "--fail", "--silent", "--show-error", "--max-time", "10",
        spec["gateway_url"].rstrip("/") + spec["health_path"]
    ], capture_output=True, text=True, timeout=15)
    if result.returncode:
        raise RuntimeError("网关连接或证书验证失败，请检查地址、HTTPS 服务及系统信任")
    checks = [{"label":"HTTPS 与证书" if parsed.scheme == "https" else "HTTP 网关", "ok":True}]
    if settings.get("catalog_path"):
        result = subprocess.run([
            settings["http_program"], "--fail", "--silent", "--show-error", "--max-time", "10",
            spec["gateway_url"].rstrip("/") + settings["catalog_path"]
        ], capture_output=True, text=True, timeout=15)
        if result.returncode:
            raise RuntimeError("网关模型目录不可用")
        catalog = json.loads(result.stdout)
        actual = {row["id"] for row in catalog["data"]}
        expected = {slot["model"] for slot in spec["slots"] if slot["visible"]}
        if not expected.issubset(actual):
            raise RuntimeError("网关模型目录与客户端所选入口不一致")
        checks.append({"label":"模型目录", "ok":True, "count":len(expected)})
    return checks


def rendered(values, spec, settings):
    variables = {"gateway_url":spec["gateway_url"], "local_token":settings["local_token"]}
    return {key: value.format(**variables) if isinstance(value, str) else value for key, value in values.items()}


def active_catalog(settings):
    directory = Path(settings["catalog_directory"])
    metadata = read_json(directory / "_meta.json")
    active = metadata["appliedId"]
    if not isinstance(active, str) or not active or Path(active).name != active:
        raise ValueError("活动客户端配置标识无效")
    path = directory / (active + ".json")
    return path, read_json(path)


def app_action(request):
    spec = request["integration"]
    settings = spec["settings"]
    catalog_path, catalog = active_catalog(settings)
    action = request["action"]
    changes = []
    if action == "sync":
        gateway_check(spec, settings)
        with Path(settings["sync_lock"]).open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            catalog_path, catalog = active_catalog(settings)
            original_catalog = json.loads(json.dumps(catalog))
            current = {row["name"]: row for row in catalog["inferenceModels"]}
            configured = {row["name"]: row for row in settings["model_rows"]}
            managed = {slot["model"] for slot in spec["slots"]}
            rows = [row for row in catalog["inferenceModels"] if row["name"] not in managed]
            for slot in spec["slots"]:
                if slot["visible"]:
                    if slot["model"] not in configured:
                        raise ValueError("所选模型入口缺少客户端兼容信息")
                    rows.append(current.get(slot["model"], configured[slot["model"]]).copy())
            if not rows:
                raise ValueError("至少需要显示一个客户端模型")
            catalog["inferenceModels"] = rows
            catalog.update(rendered(settings["gateway_values"], spec, settings))
            sync_path = Path(settings["sync_config"])
            sync_config = read_json(sync_path)
            original_sync = json.loads(json.dumps(sync_config))
            previous = {row["name"]:row for row in sync_config["bindings"]}
            bindings = [row for row in sync_config["bindings"] if row["name"] not in managed]
            for slot in spec["slots"]:
                label = slot.get("label", "")
                format_text = label.replace("{", "{{").replace("}", "}}") if label else previous.get(slot["model"], {}).get("format", "{model} · {target}")
                bindings.append({"name":slot["model"], "route":f'integration-{request["id"]}-{slot["id"]}', "format":format_text})
            sync_config["bindings"] = bindings
            if read_json(request["config_path"]) != request["config"]:
                raise RuntimeError("路由配置已变化，请刷新后重新应用接入")
            backup = Path(settings["backup_directory"])
            if atomic_json(sync_path, sync_config, backup, original_sync):
                changes.append("模型目录同步规则")
            if atomic_json(catalog_path, catalog, backup, original_catalog):
                changes.append("客户端地址与模型入口")
        result = subprocess.run(settings["sync_command"], capture_output=True, timeout=20)
        if result.returncode:
            raise RuntimeError("客户端模型名称同步失败，请检查同步程序状态")
        catalog_path, catalog = active_catalog(settings)
    actual_models = {row["name"] for row in catalog["inferenceModels"]}
    expected_models = {slot["model"] for slot in spec["slots"] if slot["visible"]}
    configured = all(catalog.get(key) == value for key, value in rendered(settings["gateway_values"], spec, settings).items()) and expected_models.issubset(actual_models)
    result = {
        "ok":True, "configured":configured, "gateway_url":catalog.get("inferenceGatewayBaseUrl"),
        "message":"客户端配置已保存；重新打开客户端后载入目录。" if action == "sync" else "已检测到客户端配置。",
        "models":[{"id":row["name"],"label":row.get("labelOverride",row["name"])} for row in catalog["inferenceModels"]],
        "changes":changes
    }
    if action == "check":
        result["checks"] = gateway_check(spec, settings)
    return result


def decode_storage(value):
    text = value.decode("utf-16-le") if isinstance(value, bytes) else value
    parsed = json.loads(text)
    if not isinstance(parsed, dict):
        raise ValueError("插件接入配置格式无效")
    return parsed


def office_stores(settings):
    stores = []
    for application in settings["applications"]:
        found = []
        for path in Path(application["storage_root"]).rglob("localstorage.sqlite3"):
            with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as connection:
                tables = connection.execute("select name from sqlite_master where type='table'").fetchall()
                if ("ItemTable",) not in tables:
                    continue
                row = connection.execute("select value from ItemTable where key=?", (settings["profile_keys"][0],)).fetchone()
                if row is not None:
                    found.append({"path":path,"profile":decode_storage(row[0])})
        stores.append({"label":application["label"],"stores":found})
    return stores


def office_action(request):
    spec = request["integration"]
    settings = spec["settings"]
    stores = office_stores(settings)
    action = request["action"]
    checks = gateway_check(spec, settings) if action in ("sync", "check") else []
    changes = []
    if action == "sync":
        if any(not application["stores"] for application in stores):
            raise RuntimeError("未检测到部分 Office 插件，请先在对应 Office 应用中打开 Claude 插件一次")
        backup_directory = Path(settings["backup_directory"])
        backup_directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(backup_directory, 0o700)
        for application in stores:
            for store in application["stores"]:
                path = store["path"]
                with sqlite3.connect(path, timeout=10) as connection:
                    original = dict(connection.execute("select key,value from ItemTable where key in (" + ",".join("?" for _ in settings["profile_keys"] + settings["customer_keys"]) + ")", settings["profile_keys"] + settings["customer_keys"]).fetchall())
                    updates = {}
                    for keys, fields in ((settings["profile_keys"], settings["profile_values"]), (settings["customer_keys"], settings["customer_values"])):
                        for key in keys:
                            if key not in original:
                                raise ValueError("插件接入配置不完整，请先在插件中完成首次配置")
                            value = decode_storage(original[key])
                            revised = {**value, **rendered(fields, spec, settings)}
                            if revised != value:
                                text = json.dumps(revised, ensure_ascii=False, separators=(",", ":"))
                                updates[key] = text.encode("utf-16-le") if isinstance(original[key], bytes) else text
                    if not updates:
                        continue
                    if read_json(request["config_path"]) != request["config"]:
                        raise RuntimeError("路由配置已变化，请刷新后重新应用接入")
                    backup_path = backup_directory / (application["label"] + "-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%f") + ".sqlite3")
                    with sqlite3.connect(backup_path) as backup:
                        connection.backup(backup)
                    os.chmod(backup_path, 0o600)
                    connection.execute("BEGIN IMMEDIATE")
                    for key, value in updates.items():
                        current = connection.execute("select value from ItemTable where key=?", (key,)).fetchone()
                        if current is None or current[0] != original[key]:
                            raise RuntimeError("Office 插件配置已变化，请重新应用")
                        connection.execute("update ItemTable set value=? where key=?", (value, key))
                    connection.commit()
                    changes.append(application["label"])
        stores = office_stores(settings)
    required = rendered(settings["profile_values"], spec, settings)
    applications = [{"label":app["label"],"detected":bool(app["stores"]),"configured":bool(app["stores"]) and all(all(store["profile"].get(key) == value for key, value in required.items()) for store in app["stores"])} for app in stores]
    return {"ok":True,"configured":all(app["configured"] for app in applications),"applications":applications,"gateway_url":spec["gateway_url"],"checks":checks,"changes":changes,"message":"插件接入配置已保存；重新打开插件后载入。" if action == "sync" else "已读取 Office 插件接入状态。"}


def main():
    request = json.load(sys.stdin)
    spec = request["integration"]
    work_directory = Path(spec["settings"]["work_directory"])
    work_directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (work_directory / "integrations.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        kind = spec["settings"]["kind"]
        if kind == "app_catalog":
            return app_action(request)
        if kind == "office_storage":
            return office_action(request)
        raise ValueError("未知客户端接入配置类型")


try:
    result = main()
except (ValueError, RuntimeError) as error:
    result = {"ok":False,"error":str(error)}
except (OSError, KeyError, sqlite3.Error, subprocess.SubprocessError) as error:
    result = {"ok":False,"error":"客户端接入配置处理失败：" + type(error).__name__}
print(json.dumps(result, ensure_ascii=False))
