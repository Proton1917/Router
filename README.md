# router

使用 Rust 实现、通过 JSON 配置的 HTTP 模型路由服务。支持 Claude Code 使用的 Messages 接口、Codex 使用的 Responses 接口，以及 Chat Completions 和其他配置的 HTTP 接口。

请求在配置的客户端规则、模型规则和后端之间分发。Responses 请求及其 SSE 响应按原生协议转发，工具调用、推理内容、用量和错误响应保留上游格式。Messages 可按配置启用字段处理、搜索工具适配、模型目录和流式响应兼容处理。

## 构建与运行

```sh
cargo build --release --manifest-path router-rs/Cargo.toml
./router-rs/target/release/router --config router-rs/examples/router.json --check
./router-rs/target/release/router --config /path/to/router.json
```

`--config` 为必填参数，也可以通过 `ROUTER_CONFIG` 设置。`--listen` 或 `ROUTER_ADDR` 可以覆盖配置中的监听地址。`--check` 校验配置及其全部路由断言，然后退出。

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

路由、客户端、模型、供应商、凭据文件、接口映射和处理策略在每次请求时重新读取。监听地址、健康检查路径及 HTTP 连接／请求超时的变更需要重新启动服务。以上配置变更均不需要重新编译。

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

## 验证

```sh
cargo fmt --manifest-path router-rs/Cargo.toml -- --check
cargo test --manifest-path router-rs/Cargo.toml
cargo clippy --manifest-path router-rs/Cargo.toml --all-targets -- -D warnings
cargo build --release --manifest-path router-rs/Cargo.toml
```

测试默认读取随源码提供的配置示例。设置 `ROUTER_TEST_CONFIG=/absolute/path/router.json` 可以验证实际部署配置。诊断请求头及允许值由 `runtime.server` 定义，诊断结果展示目标与变换后的字段，不执行上游调用。

## 本地文件与 Git

仓库纳入源码、Cargo 清单与锁文件、配置示例和文档。根目录采用明确的纳入清单；本机部署配置、客户端设置、凭据、日志、备份、运行二进制和中间文件均不在提交范围内。Rust 构建目录及源码备份由附加规则排除。
