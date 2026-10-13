import argparse
import copy
from email.parser import Parser
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import urllib.request


parser = argparse.ArgumentParser(description="使用实际配置和网关检查 Claude Code 独立服务等级")
parser.add_argument("config", type=Path)
parser.add_argument("command")
parser.add_argument("--live-model", help="额外使用官方客户端发送标准和 Fast 请求")
parser.add_argument("--native", action="store_true", help="验证客户端原生 Fast 标记")
args = parser.parse_args()
path = args.config.resolve()
config = json.loads(path.read_text())
manager = config["management"]
entry = manager["commands"][args.command]
template = manager["templates"][entry["template"]]
assert "router-speed" in template.get("options", {}), "启动方式没有独立服务等级"
spec = importlib.util.spec_from_file_location("claude_catalog", Path(__file__).with_name("claude-model-catalog.py"))
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)
server = config["runtime"]["server"]
headers = {**template.get("request_headers", {}), manager["command_header"]: args.command,
           server["dry_run_header"]: server["dry_run_values"][0], "Content-Type": "application/json"}
url = "http://" + server["listen"] + template["request_path"]
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
checks = []
directory = path.parent / ".work" / "independent-speed"
directory.mkdir(parents=True, exist_ok=True)
for speed in ("standard", "fast"):
    candidate = copy.deepcopy(config)
    candidate["management"]["commands"][args.command].setdefault("options", {})["router-speed"] = "client" if args.native else speed
    flag_args = ["--settings", json.dumps({"fastMode": speed == "fast"})] if args.native else []
    prepared = adapter.prepare({"config": candidate, "config_directory": str(path.parent),
                                "command_name": args.command, "args": flag_args})
    settings = json.loads(prepared["args"][1])
    extra = json.loads(prepared["env"]["CLAUDE_CODE_EXTRA_BODY"])
    adapted_headers = dict(Parser().parsestr(prepared["env"][template["header_env"]]).items())
    request_headers = {key:value for key,value in headers.items() if key not in ("x-router-speed-control", "x-router-fast-control")}
    request_headers.update(adapted_headers)
    if args.native:
        assert "speed" not in extra
        assert prepared["env"]["CLAUDE_CODE_DISABLE_FAST_MODE"] == "0"
        assert settings["fastMode"] == (speed == "fast")
        assert adapted_headers["x-router-fast-control"] == "client"
        extra["speed"] = speed
    else:
        assert extra["speed"] == speed
        assert prepared["env"]["CLAUDE_CODE_DISABLE_FAST_MODE"] == "1"
        assert settings["fastMode"] is False
    for binding in entry["model_bindings"]:
        profile = config["profiles"][binding["profile"]]
        destination = profile.get("fast", profile["standard"]) if speed == "fast" else profile["standard"]
        payload = {"model": binding["model"], "messages": [{"role": "user", "content": "Reply OK"}], **extra}
        model_headers = dict(request_headers)
        if args.native:
            for client in config["runtime"]["clients"]:
                selection = client.get("settings", {}).get("model_header")
                if selection and selection["header"] in model_headers:
                    model_headers[selection["header"]] += "; " + selection["parameter"] + "=" + binding["model"]
        request = urllib.request.Request(url, data=json.dumps(payload).encode(), headers=model_headers)
        with opener.open(request, timeout=30) as response:
            result = json.load(response)
        assert result["target"] == destination["target"], result
        assert result["outgoing_model"] == destination.get("model", binding["model"]), result
        if destination.get("provider"):
            assert result["outgoing_provider"] == {"only": [destination["provider"]], "allow_fallbacks": False}, result
        label = json.loads(prepared["env"]["ROUTER_MODEL_LABELS"])[binding["model"]]
        assert ("Router Fast" in label) == (not args.native and speed == "fast" and bool(profile.get("fast")))
        checks.append({"speed": speed, "model": binding["model"], "provider": destination.get("provider"), "route": result["rule_id"]})
    if args.live_model:
        client_dir = directory / (args.command + "-" + speed)
        client_dir.mkdir(exist_ok=True)
        environment = {**os.environ, "CLAUDE_CONFIG_DIR": str(client_dir)}
        command = [str(path.parent / "bin/router"), "--config", str(path), "run", args.command, "--dry-run", "--",
                   "--model", args.live_model, "-p", "Reply exactly ROUTER_SPEED_OK", "--tools", "", "--strict-mcp-config",
                   "--system-prompt", "Follow the user's output format.", "--max-turns", "1", "--output-format", "json",
                   "--debug-file", str(client_dir / "debug.log")]
        plan = json.loads(subprocess.run(command, check=True, capture_output=True, text=True, env=environment).stdout)
        for name in plan["unset_env"]:
            environment.pop(name, None)
        environment.update(plan["env"])
        adapted = adapter.prepare({"config": candidate, "config_directory": str(path.parent),
                                   "command_name": args.command, "args": [*plan["args"], *flag_args]})
        environment.update(adapted["env"])
        environment["ANTHROPIC_AUTH_TOKEN"] = "local-router"
        environment["ANTHROPIC_API_KEY"] = ""
        launch_args = [plan["program"], *adapted["args"]]
        if args.native:
            assert entry.get("options", {}).get("router-speed", template["options"]["router-speed"]["default"]) == "client"
            launch_args = [part for part in command if part != "--dry-run"] + flag_args
        with (client_dir / "response.json").open("w") as output:
            result = subprocess.run(launch_args, env=environment, cwd=client_dir,
                                    stdout=output, timeout=150, check=True)
        response = json.loads((client_dir / "response.json").read_text())
        assert not response.get("is_error"), response
        assert "ROUTER_SPEED_OK" in response.get("result", ""), response
        debug = (client_dir / "debug.log").read_text()
        assert "Fast mode cooldown triggered" not in debug
        served = [json.loads(line)["message"] for file in (client_dir / "projects").glob("*/" + response["session_id"] + ".jsonl")
                  for line in file.read_text().splitlines() if json.loads(line).get("type") == "assistant"]
        assert served
        binding = next(item for item in entry["model_bindings"] if item["model"] == args.live_model)
        profile = config["profiles"][binding["profile"]]
        destination = profile.get("fast", profile["standard"]) if speed == "fast" else profile["standard"]
        if destination.get("model") and not destination["model"].startswith("@preset/"):
            assert served[-1]["model"] == destination["model"], served[-1]["model"]
        if destination.get("provider"):
            normalized = lambda value: re.sub(r"[^a-z0-9]", "", value.lower())
            assert normalized(served[-1].get("provider", "")) == normalized(destination["provider"].split("/")[0]), served[-1].get("provider")
        print(json.dumps({"live": True, "speed": speed, "model": served[-1].get("model"),
                          "provider": served[-1].get("provider"), "result": response["result"]}), flush=True)
print(json.dumps({"ok": True, "checks": checks}, ensure_ascii=False))
