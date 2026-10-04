import json
import os
from pathlib import Path
import sys


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
    model_labels = {}
    capabilities = [environment.get("CLAUDE_CODE_MODEL_CAPABILITIES", os.environ.get("CLAUDE_CODE_MODEL_CAPABILITIES", ""))]
    for item in bindings:
        destination = config["profiles"][item["profile"]]["standard"]
        model = destination.get("model") or item["model"]
        backend = destination["target"]
        label = item.get("label") or labels.get("model:" + model) or model
        description = item.get("description") or label + " · " + labels.get("backend:" + backend, backend)
        for spelling in [item["model"], *item.get("aliases", [])]:
            model_labels[spelling] = label
        if config["profiles"][item["profile"]].get("fast"):
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
