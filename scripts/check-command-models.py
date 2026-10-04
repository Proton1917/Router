import argparse
import copy
import json
from pathlib import Path

import requests


def main():
    parser = argparse.ArgumentParser(description="通过运行中的管理接口验证真实命令模型配置，不保存修改")
    parser.add_argument("config",type=Path)
    args = parser.parse_args()
    path = args.config.resolve()
    config = json.loads(path.read_text())
    manager = config["management"]
    token_file = Path(manager["token_file"])
    if not token_file.is_absolute():
        token_file = path.parent / token_file
    session = requests.Session()
    session.trust_env = False
    session.headers["Authorization"] = "Bearer " + token_file.read_text().strip()
    base = "http://" + manager["listen"]
    state = session.get(base + "/api/state",timeout=15)
    state.raise_for_status()
    state = state.json()
    if state["config"] != config:
        raise RuntimeError("指定配置与当前管理服务不一致")

    def preview(candidate, valid=True):
        result = session.post(base + "/api/preview",json={"revision":state["revision"],"config":candidate},timeout=30)
        if valid:
            result.raise_for_status()
        elif result.status_code != 400:
            raise RuntimeError("无效模型配置没有被拒绝")
        return result.json()

    assert preview(config)["config"] == config, "重复应用改变了配置"
    commands = [(name,command) for name,command in manager["commands"].items() if len(command.get("model_bindings",[])) > 1]
    if not commands:
        raise ValueError("当前配置没有包含多个模型的命令")
    checked = []
    for name,command in commands:
        candidate = copy.deepcopy(config)
        bindings = candidate["management"]["commands"][name]["model_bindings"]
        bindings[1]["model"] = bindings[0]["model"]
        preview(candidate,valid=False)
        candidate = copy.deepcopy(config)
        hidden = candidate["management"]["commands"][name]["model_bindings"][0]
        hidden["visible"] = False
        candidate["management"]["commands"][name]["model"] = hidden["model"]
        preview(candidate,valid=False)
        candidate = copy.deepcopy(config)
        candidate["management"]["commands"][name]["model_bindings"].reverse()
        result = preview(candidate)["config"]
        assert result["management"]["integrations"] == config["management"]["integrations"]
        expected = {case["name"]:case["expected"] for case in config["validation_cases"]}
        assert all(expected[case["name"]] == case["expected"] for case in result["validation_cases"])
        checked.append({"command":name,"models":len(command["model_bindings"]),"duplicate_rejected":True,"hidden_default_rejected":True,"reorder_preserves_routes":True})
    current = session.get(base + "/api/state",timeout=15)
    current.raise_for_status()
    assert current.json()["revision"] == state["revision"], "验证期间运行配置发生变化"
    print(json.dumps({"ok":True,"commands":checked},ensure_ascii=False))


if __name__ == "__main__":
    main()
