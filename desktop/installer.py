import json
import os
import subprocess
import tempfile
from pathlib import Path

import dmgbuild

desktop = Path(__file__).resolve().parent
work = desktop / ".work/installer"
work.mkdir(parents=True, exist_ok=True)
tempfile.tempdir = str(work)
configuration = json.loads((desktop / "src-tauri/tauri.conf.json").read_text())
application = desktop / "src-tauri/target/release/bundle/macos/Router.app"
if not application.is_dir():
    raise RuntimeError("请先构建 Router.app")
subprocess.run(["codesign", "--verify", "--deep", "--strict", str(application)], check=True)
renderer = os.environ.get("ROUTER_SVG_RENDERER", "rsvg-convert")
for scale, name in [(1, "background.png"), (2, "background@2x.png")]:
    subprocess.run([renderer, "--zoom", str(scale), "--output", str(work / name), str(desktop / "installer.svg")], check=True)
background = work / "background.tiff"
subprocess.run(["/usr/bin/tiffutil", "-cathidpicheck", str(work / "background.png"), str(work / "background@2x.png"), "-out", str(background)], check=True)
settings = json.loads((desktop / "installer.json").read_text())
volume = settings.pop("volume_name")
settings.update({"files": [str(application)], "background": str(background), "icon": None, "badge_icon": None})
output = desktop / f'src-tauri/target/release/bundle/dmg/Router_{configuration["version"]}_aarch64.dmg'
output.parent.mkdir(parents=True, exist_ok=True)
dmgbuild.build_dmg(str(output), volume, settings=settings)
print(f"安装镜像已生成：{output}")
