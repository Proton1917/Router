# router

使用 Rust 实现、通过 JSON 配置的 HTTP 模型路由服务。支持 Claude Code 使用的 Messages 接口、Codex 使用的 Responses 接口，以及 Chat Completions 和其他配置的 HTTP 接口。

请求在配置的客户端规则、模型规则和后端之间分发。Responses 请求及其 SSE 响应按原生协议转发，工具调用、推理内容、用量和错误响应保留上游格式。Messages 可按配置启用字段处理、搜索工具适配、模型目录和流式响应兼容处理。

## 安装桌面控制台

桌面版使用 Tauri 2 和 macOS 自带的 WebKit。当前提供 macOS 14 及以上版本的安装流程，已验证 Apple Silicon。应用包含 router 程序、网页和首次安装所需的配置示例，使用安装包时无需安装 Rust 或 Node.js。

使用 Homebrew 安装：

```sh
brew install --cask Proton1917/tap/router
open -a Router
```

Homebrew 会自动添加项目的 Tap。升级桌面应用使用 `brew upgrade --cask Proton1917/tap/router`，卸载使用 `brew uninstall --cask Proton1917/tap/router`。已部署的后台程序、网页资源、配置和凭据保存在所选配置目录中，关闭或卸载桌面应用后会保留；后台程序和网页资源的升级步骤见下文“升级与文件位置”。

在 [Releases](https://github.com/Proton1917/Router/releases) 下载 `Router_0.3.0_aarch64.dmg` 和 `SHA256SUMS.txt`。将两个文件放在同一目录，执行 `shasum -a 256 -c SHA256SUMS.txt` 校验下载完整性，再执行以下安装步骤。也可以按下文“从源码构建桌面安装包”自行构建。

1. 打开 `Router_0.3.0_aarch64.dmg`，将 **Router.app** 拖入“应用程序”。
2. 打开 Router。首次使用可以选择“创建新配置”，也可以通过“选择已有配置文件”接入已有 `router.json`。已有配置需包含 `management`，网页资源路径需存在。
3. 创建配置时确认配置目录、终端入口目录和两个监听地址，点击“安装并打开控制台”。默认分别为 `~/.config/router`、`~/.local/bin`、`127.0.0.1:8080` 和 `127.0.0.1:8081`。已有文件或端口冲突会显示具体错误。
4. 保持“自动配置 Zsh 终端入口”选中，会在 `~/.zshrc` 添加初始化语句并备份原文件。重新打开终端后，`router` 及以后在界面新增的命令即可使用。

应用记住所选配置，后续启动直接进入控制台。菜单 **Router → 选择配置…** 可以切换配置，**Router → 打开控制台** 可以重新显示窗口。关闭窗口或退出控制台后，已启动的转发服务继续运行。转发、管理 API 和网页资源由同一个 router 服务进程提供；桌面窗口使用独立的 Tauri 界面进程。

首次创建配置会在该目录部署 `application/bin/router`、`application/web` 和 `application/scripts`，生成 `service.plist`，通过当前用户的 launchd 会话管理转发服务。再次打开控制台会确保服务运行。初始安装的服务在登录后首次打开控制台时启动。原生程序、网页及路由配置都可单独替换；普通模型、API 和命令配置通过控制台修改即可。

桌面应用只保存所选配置文件路径。管理令牌由 router 自动创建；上游 API Key 在控制台中填写，保存在配置目录下的 `credentials/`，目录必须位于 Git 工作区之外。应用内的配置页面仅允许访问所选本机管理地址；桌面安装权限仅授予随应用打包的初始配置页面。

桌面窗口在 macOS 26+ 使用原生 Liquid Glass，macOS 27+ 同时启用其交互效果；macOS 14–15 使用系统侧栏材质。标题栏与侧栏显示系统玻璃材质，编辑面板和操作栏使用透明层次，正文保持阅读对比度。窗口尺寸和材质定义位于 `desktop/appearance.json`；首次创建配置会将其复制为配置目录中的 `desktop.json`，修改后重新打开控制台生效。已有配置可放置同名文件来自定义外观。

当前 macOS 安装包使用 ad-hoc 签名，尚未经过 Apple 公证。首次打开可能受到 Gatekeeper 阻止。确认下载来源和校验值后，可按 [Apple 官方说明](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac)，在“系统设置 → 隐私与安全性”中手动允许打开。

## 第一次配置 API 与命令

打开桌面控制台或执行 `router web` 后，按以下顺序操作：

1. **API 后端**：编辑示例 `messages` 或 `responses`，填写服务商的 API 基础地址。选择“独立凭据文件”，粘贴 API Key，点击“保存密钥文件”。按服务商要求设置认证请求头模板，保存编辑并应用修改。示例分别使用 `x-api-key: {token}` 和 `Authorization: Bearer {token}`。
2. **模型配置**：选择后端并填写其支持的模型 ID。需要固定模型时填写标准目标的模型字段；需要 Fast 时配置该模型自己的 Fast 目标或服务等级。未设置固定模型的示例 profile 会保留客户端请求的模型 ID。
3. **启动命令**：新增名称，选择启动方式、模型和模型配置，保存并应用。Claude Code API 使用 `messages-client`；Codex 的 `responses-client` 可选择 API 或独立账号登录，`account-client` 使用客户端现有登录。命令名称可以自行填写，例如 `cc`、`cdx`、`cx`。Claude Code、Codex 客户端需自行安装，其程序名或绝对路径可在“启动方式”中调整。
4. 在新终端输入创建的命令。之后新增、重命名或修改命令均在控制台完成，应用修改后自动同步终端入口。

创建示例配置后，真实上游调用需要自己的有效 API Key 和有权限使用的模型。新增后端时，在编辑页面的“支持的接口协议”中勾选其实际支持的接口；模型配置、命令绑定与路由校验会使用该声明。

“客户端接入”展示 `management.integrations` 中配置的条目。首次安装示例没有预置个人 Claude App 或 Office 数据库路径。需要这些接入时，先安装并打开对应客户端，再按下文“客户端接入”说明配置其实际路径、网关和模型入口。

## 从源码构建桌面安装包

取得源码后，在项目根目录执行。需要 macOS 14+、Xcode Command Line Tools、Rust stable（Tauri 2.12 要求至少 Rust 1.90）、Node.js 20.19+ 或 22.12+、npm 和 Pixi。依赖由 Cargo、npm 和 Pixi 锁文件固定。

```sh
git clone https://github.com/Proton1917/Router.git
cd Router
npm ci --prefix desktop
npm run build --prefix desktop
```

构建过程会编译 router、构建网页、生成应用图标并打包。应用位于 `desktop/src-tauri/target/release/bundle/macos/Router.app`，安装镜像位于 `desktop/src-tauri/target/release/bundle/dmg/`。安装窗口由 `desktop/installer.svg` 和 `desktop/installer.json` 定义，使用 Retina 背景、固定图标位置和中文拖放引导；`dmgbuild` 在独立 Pixi 环境中生成 Finder 布局。构建依赖仅用于开发，安装后的运行程序不依赖 Node.js。

开发时运行 `npm run dev --prefix desktop`。先执行 `npm run prepare:bundle --prefix desktop` 可以准备直接运行 Cargo 检查所需的资源。

发行构建会将源码、Cargo 依赖及 Rust 工具链路径映射为通用路径。`desktop/resources.json` 明确列出安装包资源。安装包包含项目许可证和 `THIRD_PARTY_NOTICES.txt`；构建前会核对许可声明对应的依赖锁文件。

修改依赖锁文件后，使用 cargo-about 更新许可声明：

```sh
cargo install --locked --features cli cargo-about
mkdir -p .work/licenses
cargo about generate --locked --fail --format json --config desktop/about.toml --manifest-path router-rs/Cargo.toml --output-file .work/licenses/router.json
cargo about generate --locked --fail --format json --config desktop/about.toml --manifest-path desktop/src-tauri/Cargo.toml --output-file .work/licenses/desktop.json
npm ci --prefix web
node desktop/notices.mjs .work/licenses/router.json .work/licenses/desktop.json
```

## 安装命令行与网页

这组步骤适用于只需要命令行和浏览器的安装。在项目根目录执行，需要 Rust stable、上述 Node.js/npm，以及 macOS Zsh：

```sh
cargo build --release --locked --manifest-path router-rs/Cargo.toml
npm ci --prefix web
npm run build --prefix web

router_config="$HOME/.config/router/router.json"
router_install="$HOME/.local/share/router"
mkdir -p "$router_install/bin" "$router_install/web" "$HOME/.config/router" "$HOME/.local/bin"
install -m 755 router-rs/target/release/router "$router_install/bin/router"
codesign --force --sign - "$router_install/bin/router"
cp -R web/dist/. "$router_install/web/"
cp -R scripts "$router_install/"

# 首次安装时复制；已有配置需自行选择导入管理部分。
test ! -e "$router_config" && cp router-rs/examples/router.json "$router_config"
chmod 600 "$router_config"
"$router_install/bin/router" --config "$router_config" config management --file router-rs/examples/management.json
"$router_install/bin/router" --config "$router_config" commands install
```

首次在 `~/.zshrc` 加入下列两行，再重新打开终端。终端入口目录中的 `router` 是包装命令，原生可执行文件保存在上面的 `bin/router`。

```sh
export PATH="$HOME/.local/bin:$PATH"
source "$HOME/.config/router/commands.init.zsh"
```

```sh
router config check
router web
```

`router web` 启动服务并按配置打开浏览器，默认使用 Safari。示例的转发入口是 `http://127.0.0.1:8080`，网页入口是 `http://127.0.0.1:8081`。命令行安装使用的管理示例未设置服务启动或重启命令；需要由 launchd 等服务管理器管理时，在 `management.service_start` 和 `management.service_restart` 中配置对应命令。桌面首次安装会自动生成这两项配置及所需服务文件。

## 构建与直接运行

```sh
cargo build --release --manifest-path router-rs/Cargo.toml
./router-rs/target/release/router --config router-rs/examples/router.json --check
./router-rs/target/release/router --config /path/to/router.json serve
```

执行服务与管理操作时，通过 `--config` 或 `ROUTER_CONFIG` 指定配置文件；帮助与版本命令可直接运行。`--listen` 或 `ROUTER_ADDR` 可以覆盖配置中的监听地址。`--check` 校验配置及其全部路由断言，然后退出。

直接运行 `router` 或 `router --help` 查看命令列表。`router serve` 在同一个进程中提供转发、管理接口和网页；`router web` 确保该进程运行并打开页面。

## CLI 与本地管理界面

```sh
router web
router web --restart
router status
router commands list
router commands add my-client --template responses-client --model '<model-id>' --profile responses
router commands install
router run my-client -- --version
router run my-client --dry-run
router backends list
router models list
router routes list
router templates list
router config check
```

命令名称来自 `management.commands`。每个命令可以选择启动方式、默认模型、可选模型列表、附加参数和环境变量。启动方式定义程序、参数、模型参数标记、API 请求标记及可选的启动器集成。模板使用 MiniJinja，提供 `config`、`command_name`、`command`、`model`、`profile`、`gateway_url`、`command_header` 和 `headers_text` 等变量。

`router web` 按照 `management.browser` 打开本机配置页面，示例使用 Safari。转发入口与网页入口由同一个 Rust 进程提供，关闭浏览器后继续运行。网页包括启动命令、API 后端、模型配置、路由规则、启动方式、服务设置、客户端接入和 Mods 管理。常用参数使用表单编辑，完整 JSON 编辑保留全部配置能力。

页面打开时读取配置，并通过经过认证的事件流接收配置变更通知。文件变更由操作系统通知触发，适用于网页、CLI 和外部编辑器保存的配置。存在未保存草稿或打开的编辑窗口时，页面保留当前编辑并提示有新配置。正常页面不显示服务连接过程或在线状态；读取、保存及主动检查发生错误时显示具体原因。

新增命令在网页中填写名称、选择启动方式和模型后，应用修改会自动同步终端入口。模型的标准与 Fast 目标可通过“服务等级”选择上游标准、Priority 或 Flex 服务。启动方式的 `options` 可以声明选项名称、默认值和各选项对应的参数数组，网页自动生成下拉框；每个命令在自身 `options` 中保存所选值。例如 Codex 的启动服务等级通过该机制生成 `-c service_tier="fast"` 参数，运行时发送 `service_tier: "priority"`。

`runtime.selection.speed_pointer` 和 `fast_value` 定义主要速度字段。`speed_sources` 按顺序声明其他来源，每项包含 `pointer`、`fast_values` 和可选的客户端 `contexts`。主要字段中的显式值优先；主要字段缺失时，读取第一个适用于当前客户端且包含字符串值的来源。该值在 `fast_values` 中时选择此模型的 Fast 目标，其他字符串值选择标准目标；全部来源缺失时沿用该路由的设置规则。示例把 Responses 客户端的 `/service_tier` 中 `priority`、`fast` 和 `ultrafast` 映射为 Fast，字段与取值均由配置定义。

运行 `python3 scripts/check-speed-sources.py /absolute/path/to/router.json <command>` 可向当前命令的真实接口发送路由预览请求，检查配置来源、标准选择和主要速度字段的优先顺序。该检查依赖 `requests`，不发送上游生成请求。

Fast 是选择快速目标的客户端开关。快速目标可以指定独立 API 模型、后端、供应商和上游服务等级；命令编辑器的摘要显示其实际供应商。`validation_cases[].request_fields` 可按 JSON Pointer 设置额外请求字段，用于校验这些速度来源及其优先顺序。

Claude Code API 启动方式通过 `options.router-speed` 提供“Fast 控制方式”。默认选择“客户端 /fast”，在会话内使用原生 `/fast` 开关；每个模型使用自身配置的标准或快速目标。也可选择固定标准或固定 Fast，固定 Fast 的客户端模型名称带有 `Router Fast`。应用启动配置后重新启动命令；恢复已有会话可使用 `<命令> --continue`。

`claude-model-catalog.py` 在客户端控制模式下启用原生 Fast 能力，由客户端发送 `speed`；固定模式将所选等级合并到 `CLAUDE_CODE_EXTRA_BODY.speed` 并关闭原生开关。两种模式均保留其他附加请求字段。自动登录启动方式通过 `api_gate` 将此适配限制在 API 模式。

客户端控制模式为全部绑定模型及兼容名称声明 Fast 能力，能力标识按客户端规范去掉 `[1m]` 后缀。开启 `/fast` 保留当前模型；模型未配置快速目标时沿用标准目标。

Claude Code 的后台小模型、子任务及工作流子任务由启动方式的 `api_env` 指定；可配置 `ANTHROPIC_DEFAULT_HAIKU_MODEL`、`ANTHROPIC_SMALL_FAST_MODEL`、`CLAUDE_CODE_SUBAGENT_MODEL` 和 `CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL` 等客户端角色变量。环境值支持引用模型配置，例如 `{{ config.profiles["auxiliary"].standard.model }}`，辅助调用因此可以独立使用 OAuth 后端。压缩请求通过相应路由选择同一模型配置。旧设置文件中的同名角色变量应删除，避免覆盖启动方式的配置。

`scripts/claude-context-status.py` 读取状态栏的真实 JSON，并核对当前主模型会话中最近的有效输入 usage，包含缓存读取与写入 tokens。读取范围最多为会话文件末尾 256 KiB；辅助消息及失败消息不参与主模型上下文统计。状态栏遇到临时较小的 usage 时保持最近已确认的主模型用量，压缩边界和新会话按客户端当前数据重新计算。脚本输出上下文剩余百分比，可由现有状态栏命令调用。

`runtime.clients[].response_errors` 提供按请求头限定的错误状态转换。示例仅对携带 `x-router-fast-control: client` 的客户端将上游 429 返回为 503，并通过 `x-router-upstream-status: 429` 保留原始状态；错误正文和 `Retry-After` 等响应头原样传递，日志同时记录上下游状态。Claude Code 因此执行普通错误重试，保留当前 Fast 线路。真实上游限额仍然生效，重试达到客户端上限后报错。原生 `/fast` 中的 Opus 价格文案仍由客户端提供，实际模型和计费以所选后端为准。错误映射只能将 400～599 转为另一个错误状态，不能转换为成功。

已有 Claude Code Fast 别名恢复规则的部署，可在此启动方式的 `request_headers` 添加独立标记，并用恢复规则的 `match.absent_headers` 区分请求。独立模式按实际模型标识路由，状态栏读取 `ROUTER_MODEL_LABELS`，无需通过共享设置恢复选中模型；该内部标记应加入 `runtime.strip_request_headers`。

运行 `python3 scripts/check-claude-speed.py /absolute/path/to/router.json <command>` 可使用实际配置检查全部绑定模型在标准和 Fast 下的路由。额外传入 `--live-model <客户端模型 ID>` 会通过同目录 `bin/router` 的启动预览调用官方客户端，分别发送两次真实请求，将验证记录保存在配置目录的 `.work/independent-speed/`。

启动命令卡片列出全部已绑定的模型及服务来源。在“编辑命令与模型”中可以添加、删除、排序或隐藏条目，为每项选择 API 后端、模型配置和显示名称，并从可见列表中选择默认启动模型。“自定义标准与 Fast 目标”直接更新此条目独享的模型配置；配置被其他条目、客户端或手工路由使用时，为当前条目创建独立副本。更换或删除绑定后，页面清理该命令已脱离全部引用的模型配置。“模型配置”列表按应用后的命令、客户端和手工路由显示关联数量。显示名称留空时，按目标 API 模型读取 `management.labels["model:<模型 ID>"]`，未设置名称时显示 API 模型 ID。

多模型列表保存在命令的 `model_bindings` 数组，每项包含 `id`、`model`、`profile`、可选的 `label`、`description`、`visible`、`aliases`、`route_ids` 和 `context_window`。`aliases` 用于启动器将已有名称规范化为当前客户端模型标识；`route_ids` 将指定的现有兼容路由关联到该模型配置，要求这些路由仅属于同一客户端上下文。Router 为各条目生成同时匹配客户端、命令请求头和模型标识的独立路由。压缩和子任务等特殊规则应位于启动方式的 `route_before` 之前。单模型 `profile` 与 `model_bindings` 不能同时设置。

Claude Code 启动方式声明 `context_window_range: {"min":100000,"max":1000000}` 后，每个模型条目会显示“上下文预算（tokens）”。`context_window` 填写整数，允许的最大值取启动方式上限和已识别后端上限中的较小值。上下文预算通过 Claude Code 2.1.296 的 `modelSettings.<model>.autoCompactWindow` 生效，控制模型使用的自动压缩窗口。切换模型时，客户端使用对应条目的预算；修改配置后重新启动命令。该预算包含客户端预留的输出和压缩空间，实际触发压缩的输入 tokens 会低于窗口数值。

`management.context_discovery` 配置目录适配程序、缓存目录、缓存有效期及各后端的目录字段。打开命令编辑窗口或切换目标时，界面自动查询真实 API 模型和供应商的上限，标准与 Fast 共用一个预算时取两者较小值；“刷新模型上限”重新请求目录。管理接口预览、保存和客户端启动均校验该上限，留空的启动器基线也会受已识别上限限制。目录结果只缓存模型和上限等元数据，凭据继续读取独立来源。网络错误明确显示；没有公布数值的目标显示上限未知，其物理最大窗口需要后端提供。目录查询发生在配置操作和客户端启动时，转发请求不执行目录查询。

目录字段使用 JSON 数组表示属性路径，例如 `rows:["data"]`、`id:"id"`、`context:["context_length"]`。目录与推理接口使用不同地址时，在该后端的目录来源中设置独立的 `base_url`；`auth_headers` 可设置目录专用认证头模板，通过 `{token}` 引用同一后端的独立凭据。例如 DeepSeek 的推理地址可使用 `/anthropic`，目录来源使用 `https://api.deepseek.com`、`path:"/models"`、`context:["context_window"]` 和 `Authorization: Bearer {token}`。可通过 `endpoints` 声明供应商目录路径、数组位置、供应商标识及上下文字段；`presets` 可声明预设当前版本的模型与供应商位置。API 路径、字段、缓存时间和具体模型均由配置定义，Rust 调用通用适配接口。

适配程序将逐模型预算写入当前启动的临时设置，并清除该进程中具有更高优先级的统一自动压缩环境值。未单独填写的条目使用既有启动器基线，`[1m]` 条目使用长上下文基线，随后按已识别后端上限限制；需要更大预算时，提高通用客户端容量以容纳配置值。模型标识、供应商、Fast 目标和子任务路由保持由各自配置控制。修改后重新启动命令，已启动的会话保留其启动时的设置。自动登录入口的官方账号路径继续采用客户端原生上下文行为。

Claude Code 启动方式通过 `model_adapter` 调用 `scripts/claude-model-catalog.py`，依赖 Python 3 标准库。该程序接收包含配置、命令名称和启动参数的 JSON，输出参数及环境变量调整，保留已有 `--settings` 内容，并生成以完整客户端模型 ID 为键的 `modelPicker`。Claude Code 的菜单、切换提示及状态名称由同一条目的 `label` 提供，避免选择别名后重新显示客户端内置名称。自定义显示名称和模型变更在下一次启动时读取；已运行客户端的历史输出保持原样。自动登录判断的启动方式只有进入 API 模式才使用适配程序。

`model_adapter` 的程序与参数使用 MiniJinja，还可引用 `config_path` 和 `config_directory`。安装脚本时，将 `claude-model-catalog.py` 与其他适配程序一同放入配置目录下的 `application/scripts/`，或在启动方式中填写实际路径。Rust 只处理通用模型绑定、作用范围校验和适配程序接口；模型名称、后端、别名和显示规则保存在配置与脚本中。

运行 `python3 scripts/check-command-models.py /absolute/path/to/router.json` 可通过管理接口检查实际配置的重复应用、模型重复、隐藏默认模型和调整顺序等行为。该检查依赖 `requests`，只发送配置预览，不保存修改。

运行 `node scripts/check-model-editing.mjs /absolute/path/to/router.json` 可通过同一管理接口检查独享配置编辑、共享配置副本、绑定删除与替换，以及应用后的关联路由数量。检查使用指定的实际配置，只发送预览，不保存修改。

`scripts/codex-api.zsh` 调用 `codex-model-source.py`，按命令绑定从后端直接读取模型目录。该程序使用配置中的凭据来源查询后端，再用当前 Codex 的本机模型格式生成目录。后端提供真实模型 ID、名称、上下文长度和输入类型；原生模型保留对应的客户端基础说明，其他模型使用客户端的默认基础说明。后端目录须包含 `data` 数组、`context_length`、`architecture.input_modalities` 和 `supported_parameters`；OpenRouter 提供此格式。目录缺少配置模型或必要能力字段时，启动立即失败。

启动器合并启动方式的默认服务等级和 Fast 目标中的服务等级。环境变量 `ROUTER_CODEX_DEFAULT_SERVICE_TIERS` 接受包含 `id`、`name`、`description` 的 JSON 对象数组；OpenRouter 入口可将 `priority` 声明为目录模型的默认 Fast 服务等级。`ROUTER_CODEX_CATALOG_PATH` 设置目录路径，默认为 `/v1/models`；`ROUTER_CODEX_CATALOG_PYTHON` 设置 Python 3 程序，默认为 `python3`。该流程依赖 Python 3 标准库、Zsh（含 `zsh/system`）、jq 与 shasum。启动脚本的参数依次为 router 配置文件、目录保存位置、Codex 程序、provider 参数、`--` 和客户端参数。目录应位于仓库外；会话使用的临时文件在客户端退出时删除。API 客户端的会话内快捷命令由客户端自身决定。

每个启动命令保存一份已校验的精简模型目录，默认有效期为 300 秒。启动方式的环境变量 `ROUTER_CODEX_CATALOG_CACHE_SECONDS` 可调整有效期，设为 `0` 时每次启动都重新获取。配置、provider 参数、Codex 版本或目录适配脚本变化会立即使缓存失效。并发启动通过文件锁协调更新；失效目录必须重新获取并通过校验，失败时停止启动。缓存文件权限为 `0600`，只保存精简目录和校验信息；所有 Codex 调用均继承调用者原有的 `umask`。

`router run` 向启动程序提供 `ROUTER_COMMAND_NAME`。设置 `model_bindings` 时，Codex 目录只保留该列表中的可见条目；其余命令使用单模型绑定和带该命令标记的专属路由。名单来自配置，保留上游返回的能力信息。直接调用适配脚本时需设置同名环境变量。修改命令绑定或专属路由后，重新启动客户端即可读取新的模型列表。

`ROUTER_CODEX_CATALOG_MODEL_PATTERN` 可声明后端模型名称需要匹配的正则表达式。OpenRouter 入口可使用 `/` 检查带命名空间的模型 ID；目录不符合要求或缺少已配置模型时启动立即失败。Codex 的 Fast 开关发送 `priority`，Router 按速度来源配置选择模型的 Fast 目标，并应用其中的供应商和字段策略。实际上游服务等级以响应为准。

修改先进入浏览器草稿，应用前校验配置结构、引用关系和路由用例，并展示受影响的用例。保存使用文件锁、版本摘要和原子替换，过期版本会被拒绝。命令绑定生成带有独立请求头的路由；启动模型名称可以按协议配置在 `management.profile_models` 中，后端协议声明由 `management.backend_protocols` 保存。

终端入口通过 `router commands install` 同步。`router` 主命令安装到 `launcher_directory`，客户端命令生成为 Zsh 函数，避免影响外部程序查找同名系统可执行文件。将返回的 `shell_init` 文件在 `.zshrc` 中加载一次，后续新增和重命名在终端提示符或命令执行前同步。已有终端首次接入时需要重新加载此初始化文件。

`backends`、`models`、`routes`、`templates` 支持 `list`、`get <id>`、`put <id> --file <json>` 和 `remove <id>`。命令支持 `add`、`put`、`remove`、`install`。`router config apply --file <json>` 应用完整配置；`router config management --file <json>` 只导入管理部分并保留现有后端和路由。`router credentials set <id> --file <key-file>` 把密钥写入独立受保护文件，不打印密钥值。

管理接口仅监听 loopback 地址，使用独立随机令牌，检查 Host 和 Origin；API 响应禁止缓存，事件流同样校验令牌。密钥输入只写入 `credential_directory`，网页和配置中保存文件路径。该目录必须位于 Git 工作区之外。环境变量凭据需由转发进程继承，命令凭据需向标准输出提供令牌。

使用随附的 [管理配置示例](router-rs/examples/management.json) 时，可把运行配置放在 `~/.config/router/router.json`，将已安装的原生程序和前端资源分别放在 `~/.local/share/router/bin/router` 与 `~/.local/share/router/web`，再导入管理配置。示例中的相对目录按该布局解析。自有路由配置应在启动方式中调整 `route_before`。

```sh
npm ci --prefix web
npm run build --prefix web
```

前端构建结果位于 `web/dist`。`management.assets_directory` 指向已部署的资源目录，运行时由 Rust 提供静态文件服务。`service_start`、`service_restart` 可声明外部服务管理命令；监听地址、超时等参数变化时，界面通过配置的重启命令重启整个 router，并确认新进程启动。网页监听或资源配置变化后，使用 `router web --restart` 重新加载并打开当前地址。

## 升级与文件位置

替换 Router.app 会更新桌面程序和其随附资源；已有配置仍指向原来部署的服务程序及网页。需要升级转发服务时，先用 `router status` 确认配置路径，将新包中 `Router.app/Contents/Resources/resources/router` 和 `web/` 分别复制至该配置实际使用的原生程序路径与 `management.assets_directory`，然后执行 `router web --restart`。配置文件、凭据和自定义脚本应保留。安装了新协议适配能力时，服务程序与配套网页应一起升级。

使用 `codex-api.zsh` 的启动方式还需更新该脚本，并将同一安装包中的 `scripts/codex-catalog.jq` 放在它的同一目录。脚本位置以启动方式的 `args` 为准；更新后重新启动 Codex，以加载当前命令的模型目录。

| 内容 | 桌面首次安装位置（相对于所选配置目录） |
| --- | --- |
| 路由、命令及管理配置 | `router.json` |
| 原生服务与网页 | `application/bin/router`、`application/web/` |
| 项目与第三方许可声明 | `application/LICENSE`、`application/THIRD_PARTY_NOTICES.txt` |
| API 凭据 | `credentials/`，文件权限为 `0600` |
| 管理令牌 | `control-token` |
| Zsh 命令与初始化文件 | `commands.zsh`、`commands.init.zsh` |
| launchd 配置 | `service.plist` |
| 桌面窗口尺寸与材质 | `desktop.json` |
| 运行日志 | `logs/gateway.log` |

网页资源、桌面窗口和服务进程使用同一份 JSON 配置。保存一般路由参数后，后续请求读取新配置，打开的网页与桌面控制台自动更新。有未保存编辑时会保留草稿并提示配置有变化。

安装包和运行程序保存好之后，可执行 `cargo clean --manifest-path router-rs/Cargo.toml` 与 `cargo clean --manifest-path desktop/src-tauri/Cargo.toml` 清理构建缓存。后者会删除 `target` 中的安装包，因此必须先将要使用的 `.app` 和 `.dmg` 复制到其他目录。

## 客户端接入

网页的“客户端接入”按 `management.integrations` 展示客户端。终端入口可以直接进入命令编辑；桌面应用和 Office 插件可以逐个选择模型入口的 API 后端、模型配置及显示名称，调整模型目录可见性，并检查网关连接。需要 HTTPS 的客户端会拒绝 HTTP 地址。

每个客户端声明协议、网关地址、检查路径、请求头和模型入口。模型入口保留客户端兼容 ID，并生成独立的上下文路由及验证用例。目录入口隐藏后，已有会话使用的路由继续保留。地址和目录同步在保存路由配置后执行；同步失败会显示错误，可以重新“应用已保存配置”。

Rust 负责校验、路由生成、管理接口和有超时限制的适配程序调用。客户端目录格式和部署路径由外部配置及 `scripts/client-integrations.py` 管理。适配程序通过标准输入接收 JSON，通过标准输出返回包含 `ok` 的 JSON 结果，支持 `inspect`、`sync`、`check`。配置中的路径和命令可按部署环境调整。

提供的适配程序支持活动 JSON 模型目录，以及 Office WebKit 中已有的插件接入存储。JSON 修改保留其他字段，Office 修改通过 SQLite 事务更新接入项，写入前在受保护目录保存备份。真实上游凭据仍由 router 单独读取，客户端使用本地占位令牌。Office 的首次安装或首次加载由宿主应用完成；同步后重新打开客户端或插件载入地址和目录，路由调整对后续请求生效。连接检查验证网关、HTTPS 证书及目录，不代表上游账号已获得所有模型权限。

应用配置时，同步发生修改的客户端接入项；启用 Office 名称同步后，关联模型的默认名称或后端名称变化也会更新对应加载项清单。模型和路由配置由后续请求直接读取。Office 接入页分别报告各宿主应用的配置状态；macOS 拒绝读取应用容器时会显示权限提示，需为实际运行后台服务的程序配置系统访问权限后才能自动同步。

Office 的 Router 入口通过加载项启动参数 `available_models` 同步模型 ID 与显示名称。保存并应用接入设置时，适配程序读取当前模型目录，为 Word、Excel 生成独立的自定义加载项清单。名称通过 URL fragment 传给官方插件页面；上游凭据由 router 管理。加载项清单变化时自动递增版本，已有任务窗格在重新加载后读取新名称。首次安装后，从 Office 的“加载项 → 开发人员加载项”选择配置中的入口名称。

名称同步由 Office 接入项的 `settings.model_labels` 启用。该对象声明 `display_name`、`page_origin`、`page_paths`、`model_parameter`、`parameters` 和 `backup_directory`。`parameters` 支持 `{gateway_url}` 与 `{local_token}`，其中 `local_token` 必须是本机网关的非秘密占位值。每个 `settings.applications` 条目同时声明 `manifest_root`、`manifest_pattern`、`source_manifest_id`、`manifest_id`、`manifest_file`。源清单保存在配置指定的模板目录中，首次配置时可从已安装的官方插件缓存复制。目标清单使用独立 UUID；macOS 的目标目录为相应 Office 容器中的 `Data/Documents/wef/`。适配程序需要 Python 3、`lxml`、`jsonpointer` 和 `packaging`，通过 `adapter.program` 指定已安装这些依赖的 Python。

## Mods 管理

“Mods 管理”提供命名组合、逐项“继承 / 启用 / 停用”，以及启动命令绑定。先创建组合并设置各 Mod，再为支持 Mods 的命令选择组合，点击“应用修改”。新启动的会话读取选择。Mod 的所属插件及其其他组件会一同启停；组织管理策略继续由客户端执行。

可以添加已安装插件的完整 ID，或者添加本地插件目录 / ZIP 路径。本地目录使用清单中的 `name@inline` 标识，“校验”调用客户端的插件校验命令。已有会话保留当前加载状态；客户端原生的插件管理与重载操作继续可用。内置条目的使用条件在卡片中显示，例如辅助提醒功能可能需要官方账号、遥测和组织资格。

配置结构位于 `management.mods`：`catalog` 保存条目，`profiles` 保存组合，`adapter` 指定外部适配程序，`settings` 保存客户端程序与临时设置目录。启动方式通过 `supports_mods` 声明支持；命令的 `mod_profile` 引用组合。普通组合和条目修改通过 JSON 生效。Rust 提供通用校验、调用和启动转交，客户端专属设置由 `scripts/claude-mods.py` 处理。

Claude Code 适配程序要求 2.1.287 及以上版本、Python 3 和 `packaging`。在“客户端与运行环境”中填写这台机器的 Claude Code、Python 和适配脚本路径。桌面首次安装的脚本在配置目录的 `application/scripts/claude-mods.py`；命令行安装应将该路径调整为自己的脚本安装位置。

启动时，适配程序保留已有 `--settings` 中的模型、上下文及其他字段，将所选组合合并至 `enabledPlugins`，通过权限为 `0600` 的临时设置文件启动客户端。调用者的文件权限设置继续传递给客户端；退出后移除临时文件。未绑定组合的命令直接使用原有启动配置。

[配置示例](router-rs/examples/router.json) 分别声明 Messages 和 Responses 后端。部署时填写自己的地址和凭据来源。示例通过 `MESSAGES_API_KEY` 和 `RESPONSES_API_KEY` 环境变量读取凭据。模型由客户端选择，示例路由保留客户端传入的模型 ID。

## 配置边界

| 配置位置 | 控制内容 |
| --- | --- |
| `runtime.server` | 监听地址、健康检查路径、请求大小、连接与请求超时、诊断请求头、接口路径与协议 |
| `runtime.backends` | 上游地址、路径重写、凭据来源、供应商字段模板、缓存与协议适配选项 |
| `runtime.clients` | 按请求路径和请求头识别客户端，可选设置文件及 Messages 兼容处理 |
| `runtime.selection` | 模型、速度字段的 JSON Pointer，以及客户端模型后缀 |
| `runtime.reasoning_policies` | 推理字段读取、映射和写入规则 |
| `profiles` | 每个逻辑模型的标准与 Fast 目标、请求字段与请求头处理规则 |
| `routes` | 按顺序执行的客户端、模型、请求头和提示词匹配规则 |
| `catalog` | 可选模型目录、模型元数据及 Messages 兼容参数 |
| `validation_cases` | 配置加载时执行的路由断言 |

路由、客户端、模型、供应商、凭据文件、接口映射和处理策略在每次请求时重新读取。监听地址、健康检查路径、HTTP 超时及正文处理资源配置的变更需要重新启动服务。以上配置变更均不需要重新编译。

Rust 实现配置校验、规则执行、HTTP 传输和协议编解码。增加现有配置能够表达的后端或客户端，只需修改 JSON；增加新的协议编解码能力时需要修改对应实现。

`runtime.server.endpoints` 将路径绑定到 `messages`、`responses`、`chat_completions`、`models` 或 `passthrough`。路径支持末尾 `*`，精确匹配优先，其次选择最长前缀；客户端的 `paths` 使用同一规则。Responses 的创建、压缩、读取和删除等路径可以分别配置。`passthrough` 保留原始请求体字节。`runtime.backends.<id>.path_rewrites` 将入站路径映射到上游路径，并保留查询参数。

配置文件中的凭据文件、凭据命令和客户端设置文件允许绝对路径或相对于配置文件所在目录的路径。凭据支持 `token_env`、`token_file`、`token_command` 三种来源，每个后端选择其中一种。凭据值应存放在仓库外。

后端的 `auth.error_hint` 可填写凭据读取失败时的处理说明，例如重新登录所用的命令。该说明会随错误返回客户端，因此只应包含可公开的操作指引。凭据程序的标准错误输出不会传给客户端。

需要 OAuth 协议适配的服务可以通过独立的本机适配程序接入。适配程序负责登录、刷新 OAuth 凭据和提供兼容接口；Router 后端指向它的回环地址，并声明已验证的接口协议。适配程序的二进制、运行目录和 OAuth 凭据分别管理，实际资源占用包含该进程。

`scripts/service-token.py` 支持在 macOS/Linux 上按需启动本机适配服务。它依赖 Python 3 和 `requests`，作为后端的 `auth.token_command` 使用，参数为下列 JSON 配置文件的路径。健康检查通过后，程序才向标准输出提供本机服务令牌。令牌文件须仅允许当前用户访问；健康检查仅接受回环 HTTP 地址，禁止重定向，并且不使用环境中的代理设置。

```json
{
  "token_file": "/absolute/private/credentials/adapter-key",
  "health_url": "http://127.0.0.1:43185/v1/models",
  "health_headers": {"Authorization": "Bearer {token}"},
  "health_status": 200,
  "ready_timeout_seconds": 15,
  "probe_timeout_seconds": 2,
  "poll_interval_seconds": 0.2,
  "lock_file": "/absolute/runtime/adapter/service.lock",
  "start_command": ["/absolute/runtime/adapter/start-service"]
}
```

`start_command` 必须启动后台服务并在期限内返回。程序使用文件锁协调并发启动；认证失败、非预期 HTTP 状态和启动失败会直接返回错误。服务端应只监听回环地址，OAuth 凭据与本机服务令牌均保存于仓库外的受保护目录。健康检查证明本机适配服务可用，上游模型权限需要通过真实请求验证。

供应商参数由 `runtime.backends.<id>.provider_fields` 声明。例如某个后端要求 `provider.only` 时，可配置：

```json
{
  "provider_fields": {
    "/provider": {"only": ["{provider}"], "allow_fallbacks": false}
  }
}
```

`{provider}` 取自所选 profile 或该 profile 允许的请求头覆盖。`field_policy` 支持字段保留、删除、重命名、赋值、空值处理和指定文本行替换；`header_policy` 支持请求头过滤及固定字段。接入新后端时按其实际协议配置这些规则。

## Claude Code

将 Claude Code 的 API 地址指向 router，并选择后端实际支持的模型：

```sh
ANTHROPIC_BASE_URL=http://127.0.0.1:8080 \
ANTHROPIC_API_KEY=local-router \
claude --model '<model-id>'
```

示例配置会移除客户端传入的认证请求头，随后注入 router 自己读取的上游凭据。

## 项目初始化

通过 Claude Code API 命令运行 `/init` 时，Router 根据用户消息中的完整命令标记和初始化模板识别该操作，并将项目指令文件目标设置为 `AGENTS.md`。已有文件由初始化流程读取并提出完善内容；用户级个人指令路径保留客户端原有约定。普通对话、工具结果和其他客户端请求沿用原内容。

规则位于 `runtime.request_text_rewrites`。每条规则声明客户端 `contexts`、必须同时命中的 `markers`、目标文本块的 `prefixes` 和有序 `replacements`。只有同一条用户消息同时包含命令标记和指定模板前缀时，才处理其中对应的文本块。客户端在后续请求中重发该初始化模板时，Router 使用相同映射，因此工具轮次和交互式回答都保持初始化目标一致。客户端保存的原始会话记录保留原样。替换按列表顺序执行，其他文本块和系统提示均保留。初始化标记、模板前缀和文件名通过 JSON 修改后生效。

Claude Code 2.1.285 及以上版本可在启动设置中配置 `pluginConfigs.cc-plugin-agents-md@builtin.options.instructionFiles` 为 `claude-md-and-agents-md`，同时读取项目中的 `CLAUDE.md` 和 `AGENTS.md`。新建配置的 Claude Code API 模板携带此设置；已有安装可在客户端设置文件中配置同一字段。使用 `/memory` 检查当前项目已加载的指令文件。

## 日志保留

macOS 后台服务启动时会配置独立的日志维护任务。默认每 5 分钟检查一次 `management.log_directory` 下的 `.log` 文件：单文件超过 4 MiB 时保留末尾约 1 MiB，总量超过 16 MiB 时优先缩减较大的文件，直至总量回到限制内。检查之间仍可继续写入，下一次检查会重新限制大小。

维护任务直接缩减已有文件，保留文件身份和最新完整日志行，后台 Router 继续运行，不生成额外归档。读取与写入量受保留尾部大小限制；未达到清理条件时只检查文件信息。

策略保存在配置目录的 `log-retention.json`。`check_interval_seconds` 设置检查间隔，`max_file_bytes`、`retain_tail_bytes`、`max_total_bytes` 分别设置单文件阈值、保留尾部和总量阈值，单位为字节。调整检查间隔后重新执行服务启动操作，使维护任务读取新的时间间隔；大小设置在下一次检查时生效。

已有安装可执行以下命令启用维护任务，`--service-plist` 指向当前 Router 的 launchd 服务文件：

```sh
python3 scripts/log-maintenance.py install --service-plist /path/to/service.plist
python3 scripts/log-maintenance.py run --config /path/to/router.json --report
```

维护任务名称为原服务名称加 `.logs`，使用当前用户的 launchd 会话调度。维护错误写入同一目录的 `log-maintenance.log`，该文件也受同一策略限制。

## Codex

控制台的“启动命令 → 编辑命令与模型”中，支持切换登录方式的 Codex 模板提供“默认启动模式”和“账号配置与凭据目录”。默认使用 API；账号模式使用该目录下独立的配置、凭据和会话，模型及 Fast 由 Codex 原生菜单选择。切换模式会保留 API 模型列表。

以 `cdx` 为例，可在终端临时选择模式并完成第二个账号登录：

```sh
cdx --router-mode account login
cdx --router-mode account login status
cdx --router-mode account
cdx --router-mode api
```

首次登录在浏览器选择所需的另一个 ChatGPT 账号。账号模式使用其订阅额度；API 模式使用 Router 各后端的凭据与计费。独立账号目录通过 `CODEX_HOME` 传给官方 CLI，并使用 `cli_auth_credentials_store="file"` 保存凭据，目录权限为 `0700`。凭据目录应放在 Git 工作区之外；现有默认 `~/.codex` 账号和其他入口保留原配置。模式选择与目录修改在下次启动时生效，正在运行的会话保持原模式。

此入口使用 `scripts/codex-launch.py` 选择启动路径，复用 `router __exec` 启动 API 客户端。启动模板以 `options.auth-mode` 声明模式，以 `ROUTER_CODEX_ACCOUNT_HOME` 和 JSON 数组 `ROUTER_CODEX_ACCOUNT_ARGS` 配置账号目录及原生参数。服务等级映射保存在 `ROUTER_CODEX_SERVICE_TIERS`；`ROUTER_CODEX_USE_CATALOG=0` 直接启动兼容 Responses 的客户端，其他值沿用下述目录适配流程。API 模式的默认模型由脚本从当前命令配置读取，模板的 `model_flags` 留空，避免将 API 模型传入账号模式。启动脚本需要 Python 3。

在 Codex 用户级配置中添加自定义 provider，并选择上游 Responses 接口支持的模型：

```toml
model_provider = "router"
model = "<model-id>"

[model_providers.router]
name = "router"
base_url = "http://127.0.0.1:8080/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
```

配置字段依据 [OpenAI 官方配置文档](https://learn.chatgpt.com/docs/config-file/config-reference)。上游应原生支持 Responses；router 将 HTTP 请求与 SSE 流转发到配置的后端。客户端是否支持某个模型和功能，还取决于该后端实现。

## 正文处理与内存

仅依赖路径和请求头、且不修改正文的路由直接流式转发请求和响应。需要读取模型、检查最后一条用户消息或执行字段策略时，请求写入指定目录中的匿名临时文件；JSON 由流式解析库验证并建立字段位置索引。改写时复制未变化的原始片段，按需读取控制字段和目标子树。日志使用已经提取的路由信息。

`runtime.server.body_processing` 配置如下：

| 字段 | 作用 |
| --- | --- |
| `spool_directory` | 请求暂存目录，可相对于配置文件；Unix 目录权限为 `0700`，临时文件创建后立即解除目录链接 |
| `io_buffer_bytes` | 文件读取、文本处理和上传缓冲大小，示例为 64 KiB |
| `metadata_limit_bytes` | 单次载入的路由控制字段大小上限，示例为 4 MiB；上下文正文按块处理 |
| `max_concurrent_requests` | 同时接收及处理正文的请求数量，示例为 8；额外请求等待处理名额 |

原样转发的内存主要由传输缓冲构成。字段改写的内存主要由缓冲、控制字段及当前对象的字段索引构成；正文暂存磁盘空间随请求大小增长，文件描述符关闭后释放。取消请求时同样释放临时文件。内容检查需要遍历输入；字段策略按声明顺序执行，处理时间与输入大小及策略数量有关。Messages 的非流式响应兼容和搜索适配仍按各自的响应协议处理。

## 验证

```sh
cargo fmt --manifest-path router-rs/Cargo.toml -- --check
cargo test --manifest-path router-rs/Cargo.toml
cargo clippy --manifest-path router-rs/Cargo.toml --all-targets -- -D warnings
cargo build --release --manifest-path router-rs/Cargo.toml
```

测试默认读取随源码提供的配置示例。设置 `ROUTER_TEST_CONFIG=/absolute/path/router.json` 可以验证实际部署配置；同时设置 `ROUTER_TEST_REQUEST=/absolute/path/request.json`，可以用真实请求逐项重放配置中的路由用例，核对文件处理与 JSON 语义参考实现的结果。请求材料应保存在 Git 忽略的目录中。诊断请求头及允许值由 `runtime.server` 定义，诊断结果展示目标与变换后的字段，不执行上游调用。

## 本地文件与 Git

仓库纳入源码、Cargo 清单与锁文件、配置示例和文档。根目录采用明确的纳入清单；本机部署配置、客户端设置、凭据、日志、备份、运行二进制和中间文件均不在提交范围内。Rust 构建目录及源码备份由附加规则排除。

## 许可证

本项目采用 [MIT License](LICENSE)，允许商业使用、修改、再分发及闭源使用。分发代码或其重要部分时，须保留版权声明和许可证全文。第三方依赖遵循各自的许可证。
