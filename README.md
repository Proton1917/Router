# router

使用 Rust 实现、通过 JSON 配置的 HTTP 模型路由服务。支持 Claude Code 使用的 Messages 接口、Codex 使用的 Responses 接口，以及 Chat Completions 和其他配置的 HTTP 接口。

请求在配置的客户端规则、模型规则和后端之间分发。Responses 请求及其 SSE 响应按原生协议转发，工具调用、推理内容、用量和错误响应保留上游格式。Messages 可按配置启用字段处理、搜索工具适配、模型目录和流式响应兼容处理。

## 安装桌面控制台

桌面版使用 Tauri 2 和 macOS 自带的 WebKit。当前提供 macOS 14 及以上版本的安装流程，已验证 Apple Silicon。应用包含 router 程序、网页和首次安装所需的配置示例，使用安装包时无需安装 Rust 或 Node.js。

取得源码后，可按下文“从源码构建桌面安装包”生成安装镜像，再执行以下安装步骤。

1. 打开 `Router_0.2.0_aarch64.dmg`，将 **Router.app** 拖入“应用程序”。
2. 打开 Router。首次使用可以选择“创建新配置”，也可以通过“选择已有配置文件”接入已有 `router.json`。已有配置需包含 `management`，网页资源路径需存在。
3. 创建配置时确认配置目录、终端入口目录和两个监听地址，点击“安装并打开控制台”。默认分别为 `~/.config/router`、`~/.local/bin`、`127.0.0.1:8080` 和 `127.0.0.1:8081`。已有文件或端口冲突会显示具体错误。
4. 保持“自动配置 Zsh 终端入口”选中，会在 `~/.zshrc` 添加初始化语句并备份原文件。重新打开终端后，`router` 及以后在界面新增的命令即可使用。

应用记住所选配置，后续启动直接进入控制台。菜单 **Router → 选择配置…** 可以切换配置，**Router → 打开控制台** 可以重新显示窗口。关闭窗口或退出控制台后，已启动的转发服务继续运行。转发、管理 API 和网页资源由同一个 router 服务进程提供；桌面窗口使用独立的 Tauri 界面进程。

首次创建配置会在该目录部署 `application/bin/router`、`application/web` 和 `application/scripts`，生成 `service.plist`，通过当前用户的 launchd 会话管理转发服务。再次打开控制台会确保服务运行。初始安装的服务在登录后首次打开控制台时启动。原生程序、网页及路由配置都可单独替换；普通模型、API 和命令配置通过控制台修改即可。

桌面应用只保存所选配置文件路径。管理令牌由 router 自动创建；上游 API Key 在控制台中填写，保存在配置目录下的 `credentials/`，目录必须位于 Git 工作区之外。应用内的配置页面仅允许访问所选本机管理地址；桌面安装权限仅授予随应用打包的初始配置页面。

桌面窗口在 macOS 26+ 使用原生 Liquid Glass，macOS 27+ 同时启用其交互效果；macOS 14–15 使用系统侧栏材质。标题栏与侧栏显示系统玻璃材质，编辑面板和操作栏使用透明层次，正文保持阅读对比度。窗口尺寸和材质定义位于 `desktop/appearance.json`；首次创建配置会将其复制为配置目录中的 `desktop.json`，修改后重新打开控制台生效。已有配置可放置同名文件来自定义外观。

本地构建的 macOS 安装包使用 ad-hoc 签名。对外分发前需使用自己的 Apple Developer ID 签名并完成公证，以满足下载后首次运行的 Gatekeeper 检查。

## 第一次配置 API 与命令

打开桌面控制台或执行 `router web` 后，按以下顺序操作：

1. **API 后端**：编辑示例 `messages` 或 `responses`，填写服务商的 API 基础地址。选择“独立凭据文件”，粘贴 API Key，点击“保存密钥文件”。按服务商要求设置认证请求头模板，保存编辑并应用修改。示例分别使用 `x-api-key: {token}` 和 `Authorization: Bearer {token}`。
2. **模型配置**：选择后端并填写其支持的模型 ID。需要固定模型时填写标准目标的模型字段；需要 Fast 时配置该模型自己的 Fast 目标或服务等级。未设置固定模型的示例 profile 会保留客户端请求的模型 ID。
3. **启动命令**：新增名称，选择启动方式、模型和模型配置，保存并应用。Claude Code API 使用 `messages-client`，Codex API 使用 `responses-client`，Codex 登录使用 `account-client`。命令名称可以自行填写，例如 `cc`、`cdx`、`cx`。Claude Code、Codex 客户端需自行安装，其程序名或绝对路径可在“启动方式”中调整。
4. 在新终端输入创建的命令。之后新增、重命名或修改命令均在控制台完成，应用修改后自动同步终端入口。

创建示例配置后，真实上游调用需要自己的有效 API Key 和有权限使用的模型。新增后端时，在编辑页面的“支持的接口协议”中勾选其实际支持的接口；模型配置、命令绑定与路由校验会使用该声明。

“客户端接入”展示 `management.integrations` 中配置的条目。首次安装示例没有预置个人 Claude App 或 Office 数据库路径。需要这些接入时，先安装并打开对应客户端，再按下文“客户端接入”说明配置其实际路径、网关和模型入口。

## 从源码构建桌面安装包

取得源码后，在项目根目录执行。需要 macOS 14+、Xcode Command Line Tools、Rust stable（Tauri 2.12 要求至少 Rust 1.90）、Node.js 20.19+ 或 22.12+，以及 npm。依赖由 Cargo 和 npm 锁文件固定。

```sh
git clone https://github.com/Proton1917/Router.git
cd Router
npm ci --prefix desktop
npm run build --prefix desktop
```

构建过程会编译 router、构建网页、生成应用图标并打包。应用位于 `desktop/src-tauri/target/release/bundle/macos/Router.app`，安装镜像位于 `desktop/src-tauri/target/release/bundle/dmg/`。构建依赖仅用于开发，安装后的运行程序不依赖 Node.js。

开发时运行 `npm run dev --prefix desktop`。先执行 `npm run prepare:bundle --prefix desktop` 可以准备直接运行 Cargo 检查所需的资源。

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

命令名称来自 `management.commands`。每个命令可以选择启动方式、默认模型、独立模型路由、附加参数和环境变量。启动方式定义程序、参数、模型参数标记、API 请求标记及可选的启动器集成。模板使用 MiniJinja，提供 `config`、`command_name`、`command`、`model`、`profile`、`gateway_url`、`command_header` 和 `headers_text` 等变量。

`router web` 按照 `management.browser` 打开本机配置页面，示例使用 Safari。转发入口与网页入口由同一个 Rust 进程提供，关闭浏览器后继续运行。网页包括启动命令、API 后端、模型配置、路由规则、启动方式、服务设置和客户端接入。常用参数使用表单编辑，完整 JSON 编辑保留全部配置能力。

页面打开时读取配置，并通过经过认证的事件流接收配置变更通知。文件变更由操作系统通知触发，适用于网页、CLI 和外部编辑器保存的配置。存在未保存草稿或打开的编辑窗口时，页面保留当前编辑并提示有新配置。正常页面不显示服务连接过程或在线状态；读取、保存及主动检查发生错误时显示具体原因。

新增命令在网页中填写名称、选择启动方式和模型后，应用修改会自动同步终端入口。模型的标准与 Fast 目标可通过“服务等级”选择上游标准、Priority 或 Flex 服务。启动方式的 `options` 可以声明选项名称、默认值和各选项对应的参数数组，网页自动生成下拉框；每个命令在自身 `options` 中保存所选值。例如 Codex 的启动服务等级通过该机制生成 `-c service_tier="fast"` 参数，运行时发送 `service_tier: "priority"`。

`scripts/codex-api.zsh` 从后端读取 Codex 模型目录，并合并启动方式的默认服务等级、后端目录元数据和同模型、同后端 Fast 目标中的服务等级。启动方式的环境变量 `ROUTER_CODEX_DEFAULT_SERVICE_TIERS` 接受包含 `id`、`name`、`description` 的 JSON 对象数组；OpenRouter 入口可将 `priority` 声明为所有目录模型的默认 Fast 服务等级。该规则也适用于后续新增的目录模型。该脚本依赖 Zsh 与 jq，参数依次为 router 配置文件、临时目录、Codex 程序、provider 参数、`--` 和客户端参数。临时目录应位于仓库外；生成的目录文件在客户端退出时删除。API 客户端的会话内快捷命令由客户端自身决定。

`ROUTER_CODEX_CATALOG_MODEL_PATTERN` 可声明后端模型名称需要匹配的正则表达式。OpenRouter 入口可使用 `/` 检查带命名空间的模型 ID；目录不符合要求时启动立即失败。Fast 表示请求上游 `priority` 服务等级，实际使用的服务等级以 OpenRouter 响应为准。

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

| 内容 | 桌面首次安装位置（相对于所选配置目录） |
| --- | --- |
| 路由、命令及管理配置 | `router.json` |
| 原生服务与网页 | `application/bin/router`、`application/web/` |
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

## Codex

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
