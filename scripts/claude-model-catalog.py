import json
import os
import re
from pathlib import Path
import sys
from importlib.util import module_from_spec, spec_from_file_location
from email.parser import Parser


def merge(left, right):
    for key, value in right.items():
        if isinstance(value, dict) and isinstance(left.get(key), dict):
            merge(left[key], value)
        else:
            left[key] = value
    return left


def prepare(request):
    config = request["config"]
    entry = config["management"]["commands"][request["command_name"]]
    bindings = entry["model_bindings"]
    labels = config["management"].get("labels", {})
    aliases = {alias: item["model"] for item in bindings for alias in [item["model"], *item.get("aliases", [])]}
    settings = {}
    args = []
    source = iter(request["args"])
    for arg in source:
        if arg == "--settings" or arg.startswith("--settings="):
            text = next(source) if arg == "--settings" else arg.split("=", 1)[1]
            value = json.loads(text) if text.lstrip().startswith("{") else json.loads(Path(text).expanduser().read_text())
            merge(settings, value)
        elif arg == "--model":
            model = next(source)
            args.extend([arg, aliases.get(model, model)])
        elif arg.startswith("--model="):
            model = arg.split("=", 1)[1]
            args.append("--model=" + aliases.get(model, model))
        else:
            args.append(arg)
    options = []
    environment = dict(settings.get("env", {}))
    template = config["management"]["templates"][entry["template"]]
    speed_option = template.get("options", {}).get("router-speed")
    router_speed = None
    if speed_option:
        router_speed = entry.get("options", {}).get("router-speed", speed_option["default"])
        if router_speed not in ("standard", "fast"):
            raise ValueError("Router 服务等级必须为 standard 或 fast")
        extra_body = json.loads(environment.get("CLAUDE_CODE_EXTRA_BODY", os.environ.get("CLAUDE_CODE_EXTRA_BODY", "")) or "{}")
        if not isinstance(extra_body, dict):
            raise ValueError("CLAUDE_CODE_EXTRA_BODY 必须是 JSON 对象")
        extra_body["speed"] = router_speed
        environment["CLAUDE_CODE_EXTRA_BODY"] = json.dumps(extra_body, ensure_ascii=False, separators=(",", ":"))
        environment["CLAUDE_CODE_DISABLE_FAST_MODE"] = "1"
        environment["ROUTER_CLAUDE_SPEED"] = router_speed
        settings["fastMode"] = False
        settings["fastModePerSessionOptIn"] = True
    context_bindings = [item for item in bindings if item.get("context_window") is not None]
    discovered = {}
    limits = config['management']['templates'][entry['template']].get('context_window_range')
    if limits and config['management'].get('context_discovery'):
        module = spec_from_file_location('model_context', Path(__file__).with_name('model-context.py'))
        discovery = module_from_spec(module)
        module.loader.exec_module(discovery)
        header_env = config['management']['templates'][entry['template']].get('header_env')
        header_text = environment.get(header_env, os.environ.get(header_env, '')) if header_env else ''
        headers = dict(Parser().parsestr(header_text).items())
        discovered = discovery.discover({**request, 'headers':headers})['commands'][request['command_name']]
        for item in bindings:
            discovery.validate_budget(request['command_name'], item, discovered[item['id']])
    if context_bindings or any(item['max_tokens'] is not None for item in discovered.values()):
        limits = config["management"]["templates"][entry["template"]]["context_window_range"]
        for item in context_bindings:
            window = item["context_window"]
            if isinstance(window, bool) or not isinstance(window, int) or not limits["min"] <= window <= limits["max"]:
                raise ValueError("模型上下文预算超出启动方式允许的范围")
        previous_max = int(environment.get("CLAUDE_CODE_MAX_CONTEXT_TOKENS", os.environ.get("CLAUDE_CODE_MAX_CONTEXT_TOKENS", limits["min"])))
        previous_compact = environment.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW", os.environ.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW", ""))
        previous_compact = int(previous_compact) if previous_compact else previous_max
        environment["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] = str(max([previous_max, *(item["context_window"] for item in context_bindings)]))
        environment["CLAUDE_CODE_AUTO_COMPACT_WINDOW"] = ""
        settings["autoCompactWindow"] = previous_compact
        model_settings = settings.setdefault("modelSettings", {})
        for item in bindings:
            window = item.get("context_window")
            if window is None:
                window = limits["max"] if re.search(r"\[1m\]$", item["model"], re.I) else min(previous_compact, previous_max)
            maximum = discovered.get(item['id'], {}).get('max_tokens')
            if maximum is not None:
                window = min(window, maximum)
            for spelling in [item["model"], *item.get("aliases", [])]:
                key = re.sub(r"\[1m\]", "", spelling, flags=re.I)
                row = model_settings.setdefault(key, {})
                existing = row.get('autoCompactWindow', window)
                row["autoCompactWindow"] = window if item.get("context_window") is not None else min(existing, window)
    model_labels = {}
    capabilities = [environment.get("CLAUDE_CODE_MODEL_CAPABILITIES", os.environ.get("CLAUDE_CODE_MODEL_CAPABILITIES", ""))]
    for item in bindings:
        profile = config["profiles"][item["profile"]]
        destination = profile.get("fast", profile["standard"]) if router_speed == "fast" else profile["standard"]
        model = destination.get("model") or item["model"]
        backend = destination["target"]
        label = item.get("label") or labels.get("model:" + model) or model
        if router_speed == "fast" and profile.get("fast"):
            label += " · Router Fast"
        description = item.get("description") or label + " · " + labels.get("backend:" + backend, backend)
        for spelling in [item["model"], *item.get("aliases", [])]:
            model_labels[spelling] = label
        if not speed_option and profile.get("fast"):
            capabilities.append(item["model"] + "=fast_mode")
        if item.get("visible", True):
            options.append({"model": item["model"], "label": label, "description": description})
        for key in ("ANTHROPIC_DEFAULT_MODEL", "ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_FABLE_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_CUSTOM_MODEL_OPTION"):
            selected = environment.get(key, os.environ.get(key))
            if selected in [item["model"], *item.get("aliases", [])]:
                environment[key] = item["model"]
                if key.startswith("ANTHROPIC_DEFAULT_") and key.endswith("_MODEL") or key == "ANTHROPIC_CUSTOM_MODEL_OPTION":
                    environment[key + "_NAME"] = label
                    environment[key + "_DESCRIPTION"] = description
    if not options:
        raise ValueError("命令至少需要一个可见模型")
    settings["modelPicker"] = {"replaceBuiltInOptions": True, "options": options}
    environment["ROUTER_MODEL_LABELS"] = json.dumps(model_labels,ensure_ascii=False,separators=(",", ":"))
    if any(capabilities):
        environment["CLAUDE_CODE_MODEL_CAPABILITIES"] = ";".join(filter(None, capabilities))
    settings["env"] = environment
    return {"args": ["--settings", json.dumps(settings, ensure_ascii=False, separators=(",", ":")), *args], "env": environment}


if __name__ == "__main__":
    try:
        print(json.dumps(prepare(json.load(sys.stdin)), ensure_ascii=False))
    except (OSError, ValueError, KeyError, StopIteration) as error:
        print("模型菜单生成失败：" + str(error), file=sys.stderr)
        raise SystemExit(1)
