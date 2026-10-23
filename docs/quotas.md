# 账号配额

在 Router 控制台打开 **服务与额度 → 账号配额**，查看 Antigravity 和 Grok 的剩余额度、重置时间及最近更新时间。桌面应用和 `router web` 使用同一个页面。

页面打开时读取配额，可见期间按来源配置的间隔刷新；离开页面或隐藏窗口后停止定时请求。点击“刷新配额”立即查询。后台在内存中缓存结果，并合并同一来源的并发查询。查询失败时保留上次成功的数据并显示错误和更新时间；没有历史结果时显示查询失败。配额查询与模型转发分别执行。

Antigravity 的分组由上游返回，当前包括 Gemini、Claude / GPT、Gemini 4 Argon。每个分组中的模型共享对应额度，面板分别展示每周和五小时窗口。Grok 的统一订阅接口返回共享周期已用比例；Router 用 `100 − 已用比例` 显示剩余比例，并展示 Build、Chat 等产品的用量和上游提供的额外用量余额。所有重置时间按浏览器所在系统的时区显示。未提供的数值显示“未提供”。

## 配置配额来源

来源保存在 `management.quota_sources`。每个来源包含：

| 字段 | 用途 |
| --- | --- |
| `label` | 页面显示名称 |
| `backends` | 共享该账号配额的后端 ID，仅用于说明关联 |
| `auth_backend` | 可选；复用指定后端的凭据，OAuth 由 Router 检查有效期并刷新 |
| `adapter` | 适配程序及参数，支持 `config_directory`、`config_path` 模板变量 |
| `ttl_seconds` | 缓存与页面自动刷新间隔，15–3600 秒 |
| `timeout_seconds` | 授权读取、适配程序各自的超时上限，1–120 秒 |
| `settings` | 传给适配程序的接口地址、请求头及供应商参数 |

参照 [配额配置示例](../router-rs/examples/quotas.json)，将来源对象合并进已有管理配置。示例中的凭据路径和后端 ID 需要换成实际值。复制 `scripts/provider-quota.py` 到部署目录的 `scripts/`，配置 Python 3 路径。修改来源参数后由下一次查询读取。

Antigravity 适配程序读取现有桥接服务管理的凭据文件，令牌续期由该桥接服务负责。`credential_file` 指向仓库之外的现用凭据。Grok 使用 `auth_backend` 引用 Router 的 OAuth 后端，标准和 Fast 后端共享同一授权时只配置一份配额来源。

`settings.proxy_url` 可指定 HTTP 代理；空字符串表示直连，省略时采用 Python 的系统或环境代理。后台服务应配置能够访问供应商接口的网络出口。Antigravity 桥接服务的 OAuth 刷新与模型请求使用其自身 `requests.proxy-url` 设置。

## 适配接口

管理接口为 `POST /api/quotas/read`，请求正文是 `{"id":"来源 ID","refresh":false}`，使用控制台现有认证。返回 `data`、`updated_at`（Unix 秒）、`error`、`stale`、`ttl_seconds`。`refresh:true` 忽略已完成的缓存；同时发起的同源刷新共享结果。

适配程序从标准输入读取 `settings`、可选 `token` 和 `config_directory`，通过标准输出返回 JSON。令牌只传入子进程的标准输入。适配程序输出最多 256 KiB；只将以下结构化字段提供给前端：

```json
{
  "groups": [
    {
      "label": "账号分组",
      "windows": [
        {
          "label": "每周额度",
          "remaining_percent": null,
          "reset_at": null,
          "disabled": false
        }
      ]
    }
  ],
  "details": [],
  "error": null
}
```

`remaining_percent` 为 0–100 或 `null`，`reset_at` 为 RFC 3339 时间或 `null`；`details` 每项包含 `label` 和 `value` 字符串。查询失败时返回可读的 `error`。原始令牌和上游响应正文不得放入输出、错误信息或日志。

配额接口依据：Antigravity 使用 CLIProxyAPI 管理界面采用的 [`retrieveUserQuotaSummary`](https://github.com/router-for-me/Cli-Proxy-API-Management-Center/blob/main/src/features/quota/providers/antigravity/data.ts)；Grok 使用官方 Grok Build 的 [`billing?format=credits`](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/extensions/billing.rs)。供应商字段转换保存在 Python 适配程序中，接口地址、请求头、显示名称和刷新间隔由 JSON 配置提供。
