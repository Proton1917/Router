# OAuth 配置

每个后端的 `auth.oauth` 定义服务商授权参数。它与 `token_file`、`token_env`、`token_command` 三种凭据来源互斥；`auth.headers` 继续通过 `{token}` 引用访问令牌。服务商参数由 JSON 提供，增加符合已有协议的服务商无需重新编译 Rust。

## 设备码授权

以下为 xAI 公共设备码客户端参数。授权端点来自 [xAI 的 OpenID 配置](https://auth.x.ai/.well-known/openid-configuration)，Client ID 是公开客户端标识。凭据路径应替换为自己的绝对路径。

```json
{
  "base_url": "https://api.x.ai",
  "auth": {
    "headers": {"authorization": "Bearer {token}"},
    "oauth": {
      "provider": "xAI",
      "flow": "device_code",
      "client_id": "b1a00492-073a-47ea-816f-4c329264a828",
      "device_authorization_url": "https://auth.x.ai/oauth2/device/code",
      "token_url": "https://auth.x.ai/oauth2/token",
      "scopes": ["openid", "profile", "email", "offline_access", "grok-cli:access", "api:access"],
      "credential_file": "/absolute/path/outside-repository/xai-oauth.json",
      "login_timeout_seconds": 600
    }
  }
}
```

Router 请求设备码、打开服务端返回的授权页面，并按服务端指定的间隔等待结果；`authorization_pending` 和 `slow_down` 由 OAuth 库按协议处理。取消或超时会停止等待。

## 浏览器授权

将 `flow` 改为 `authorization_code`，填写 `authorization_url` 与 `redirect_uri`。Router 使用 PKCE S256 和随机 `state`，在 `127.0.0.1` 临时监听指定端口，收到通过状态校验的回调后换取令牌并释放端口。拒绝授权、取消、超时同样会释放监听。

`redirect_uri` 必须与服务商为 Client ID 注册的本机回调规则一致。端口 `0` 表示自动分配，适用于允许动态 loopback 端口的服务商。自定义 URI scheme、网页登录 Cookie、非标准私有登录协议需要对应适配。

## 可选参数

| 字段 | 用途 |
| --- | --- |
| `provider` | 自由填写的显示名称 |
| `client_secret_file` | Client Secret 文件路径；在界面中输入时保存为独立凭据文件 |
| `client_auth_basic` | `true` 使用 HTTP Basic；默认在请求正文传递客户端认证参数 |
| `authorization_params` | 授权请求附加参数，例如 `audience`、`resource` |
| `token_params` | 换取及刷新令牌时的附加参数 |
| `login_timeout_seconds` | 等待用户授权的时限，60～1800 秒，默认 600 秒 |

附加参数不能覆盖 `state`、PKCE、Client ID、令牌等协议字段。配置中不要保存密钥或令牌。

## 凭据与运行

Router 原子写入权限为 `0600` 的凭据文件，目录权限为 `0700`，凭据目录须位于 Git 工作区之外。每次调用时检查有效期；访问令牌距离过期不足一分钟或已经过期时，Router 用刷新令牌直接请求服务商。文件锁协调多个本机进程，避免刷新令牌轮换冲突。服务商未返回刷新令牌时，到期需要重新登录。登录状态由保存的有效期判断，实际模型权限还需通过后端调用确认。

共享凭据文件的后端必须使用同一 Client ID 和令牌地址。新登录完成后替换旧凭据；取消登录保留原凭据。“退出本机账号”删除本机文件，不会注销服务商网页会话或远程撤销授权。

管理操作使用现有本机管理接口保护：`POST /api/oauth/action`，正文为 `{"id":"后端标识","action":"login"}`。操作包括 `status`、`login`、`cancel`、`refresh`、`logout`。接口只返回状态、授权链接及设备码，不返回访问令牌、刷新令牌或 Client Secret。后台令牌刷新不产生模型调用。

上下文目录和 Codex 模型目录适配程序使用同一 OAuth 凭据读取逻辑。内部 `router oauth-token --settings JSON` 命令用于这些适配程序，其标准输出是访问令牌，应只由调用程序捕获，不应写入日志。

## 共享模型目录

给一个明确匹配的 `runtime.clients` 条目设置 `catalog_commands`，可向桌面聊天客户端提供现有命令中的可见模型。例如，在最终默认客户端之前添加：

```json
{
  "id": "desktop-chat",
  "any_headers": [
    {"name":"authorization","values":["Bearer router-local-client"]},
    {"name":"x-api-key","values":["router-local-client"]}
  ],
  "catalog_commands": ["mycc", "mycodex"]
}
```

命令名称须已存在。`router-local-client` 是本机客户端匹配标记，不是上游密钥；此配置供回环地址上的客户端使用。客户端 Base URL 填写实际转发地址，API Key 填写该标记。模型目录返回 `mycc/模型名称` 等独立 ID，并自动生成仅适用于此客户端的路由。目录名称取模型条目显示名称或目标模型名称，目标、供应商、Fast 和字段策略复用原模型配置。

客户端须按模型后端实际支持的协议发送请求。填写多个端点不会自动把任意 Messages、Responses、Chat Completions 互相转换。尚未配置独立 Fast 目标的模型继续使用其标准目标。

Cherry Studio 的普通自定义提供商主要根据模型 ID 匹配本地能力目录，可能需要在“模型设置”中补充推理、工具和输入模态。Router 输出目录时会去除配置声明的客户端专用模型后缀，原后缀仍可用于已保存的请求。

若要使用 Cherry Studio 的原生“服务等级 → 快速”，可从 OpenRouter 协议预设创建 Router 实例，再将实际端点改为本机 Router 地址。其快速选项发送 `service_tier: priority`；Router 可通过以下配置将它消费为自己的标准/Fast 路由选择，避免把仅供 Router 选择目标的字段透传到其他服务商：

```json
{
  "contexts": ["desktop-chat"],
  "pointer": "/service_tier",
  "fast_values": ["priority", "fast", "ultrafast"],
  "consume": true
}
```

将此项加入 `runtime.selection.speed_sources`，其中上下文名称须与客户端的 `id` 一致。目标自己的 `field_policy.set` 仍可设置上游所需的服务等级。Fast 的模型、后端和计费遵循模型配置中的 Fast 目标。
