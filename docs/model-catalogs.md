# 模型目录同步

打开“启动命令 → 编辑命令与模型”，选择 API 后端，旁边的模型下拉框会加载该后端的完整目录。搜索框按名称和 ID 筛选，“同步模型”立即查询。选择条目后，Router 在草稿中建立对应模型绑定；“保存草稿 → 应用修改”将它保存为可用入口。

替换范围默认是“仅此命令入口”。共享配置会复制为当前入口的独立配置。选择“共享配置的全部关联入口”时，编辑器显示受影响的命令、客户端与规则。新模型使用目录来源配置的默认请求策略；已有模型可复用其保存的路由配置。供应商、上下文预算和独立 Fast 目标在同一命令窗口中设置。

左下角“设置 → 客户端接入”中的模型选择也读取同一目录。Claude App、Office 的兼容 ID 保留，绑定的目标模型可以更换。高级设置中的共享模型配置提供同样的模型选择控件。

## 思考强度

`runtime.selection.reasoning_sources` 按顺序声明请求中的强度字段，例如 `/output_config/effort`、`/reasoning/effort`、`/reasoning_effort`、`/effort`。每个目标的 `reasoning_models` 保存档位与实际模型 ID 的映射：

```json
{
  "label": "Opus 5.5",
  "models": {
    "low": "claude-opus-5-5-low",
    "medium": "claude-opus-5-5-medium",
    "high": "claude-opus-5-5-high"
  },
  "default_effort": "high",
  "follow_request": true,
  "aliases": {"minimal": "low", "xhigh": "high", "max": "high"},
  "labels": {"low": "Low · 低", "medium": "Medium · 中", "high": "High · 高"},
  "remove": ["/thinking"]
}
```

映射属于目标所在的 API 后端。`follow_request` 为 `false` 时使用 `default_effort`；为 `true` 时优先读取客户端参数。`aliases` 明确声明不同客户端档位的对应关系，未配置的强度返回错误。完成选择后移除已消费的强度字段及 `remove` 指定的字段，避免后端再次按其他参数改变档位。标准与 Fast 分别保存映射；上下文预算按所有可能目标的已知上限检查。

在 `management.model_catalogs.<backend>.reasoning_variants` 中配置目录的档位命名规则：

```json
{
  "suffixes": {"low": "-low", "medium": "-medium", "high": "-high"},
  "name_suffixes": {"low": " (Low)", "medium": " (Medium)", "high": " (High)"},
  "default_effort": "high",
  "aliases": {"minimal": "low", "xhigh": "high", "max": "high"},
  "labels": {"low": "Low · 低", "medium": "Medium · 中", "high": "High · 高"},
  "remove": ["/thinking"]
}
```

模型下拉框只合并实时目录中已经返回、且同一名称前缀下存在至少两个档位的条目。选择模型后，将当前目录中的目标写入路由配置。相同规则适用于后续新模型，规则变化时修改 JSON；Rust 不包含服务商名称、模型名称或固定档位列表。已保存的映射可以在命令、客户端接入和共享模型编辑窗口中修改。

## 后台监控

命令编辑窗口提供模型搜索与自填输入框。选择后端后，输入关键字过滤实时目录；填写完整 ID 并点击“使用此 ID”可保存目录尚未列出的模型。

供应商目录由 `management.model_catalogs.<backend>.providers` 配置，复用该后端的认证。OpenRouter 可配置：

```json
{
  "providers": {
    "path": "/v1/models/{model}/endpoints",
    "rows_pointer": "/data/endpoints",
    "id_pointers": ["/tag"],
    "name_pointers": ["/provider_name"]
  }
}
```

选择模型时，供应商输入框查询该模型的端点列表。用户可选择返回的 tag 或直接填写 tag；应用后继续使用该目标原有的供应商锁定设置。

模型选择控件中的“后台监控”与检查间隔随应用修改保存。后台 Router 运行时默认每 30 秒查询，关闭控制台后继续检查。打开模型编辑或切换后端时立即同步。首次成功结果作为比较基准，后续显示新增、移除及名称、上下文、能力字段的变化；点击“标记已查看”更新基准。后台只读取目录，模型替换由用户选择并应用。

各页面共享同一后端的缓存。读取失败时显示错误并保留已读取的模型，现有模型转发继续使用保存的路由。没有目录接口的后端可在选择控件中手动填写模型 ID。

缓存保存在路由配置文件旁的 `model-catalogs/`，文件权限为 `0600`，目录为 `0700`。每个来源只保存一份最新目录和已查看基准，进程重启后继续使用。缓存只包含模型元数据及目录查询配置。

## 来源配置

`management.model_catalogs` 使用后端 ID 作为键。未配置的后端可手动同步默认 `/v1/models`，后台监控默认关闭。

```json
{
  "model_catalogs": {
    "my-api": {
      "enabled": true,
      "interval_seconds": 30,
      "timeout_seconds": 20,
      "path": "/v1/models",
      "rows_pointer": "/data",
      "id_pointers": ["/id"],
      "name_pointers": ["/name", "/display_name"],
      "context_pointers": ["/context_length", "/context_window"],
      "capability_pointers": ["/capabilities", "/architecture/input_modalities"],
      "destination": {"field_policy": {"remove": ["/speed"]}}
    }
  }
}
```

`path` 附加在该后端的 `base_url` 后。目录认证复用后端已配置的 API Key、OAuth 或凭据命令。`headers` 设置目录专用请求头；`auth_headers` 可重定义认证头模板，其中 `{token}` 来自该后端现有凭据。列表与属性映射均使用 JSON Pointer，`filter` 可按条目字段的字符串值筛选。

独立的目录服务通过 `request_url` 指定 HTTPS 地址，并通过 `credential_file`、`token_pointer` 读取对应访问令牌。配置 `request_body` 时使用 POST；`body_credentials` 将凭据 JSON 中指定指针的值填入请求字段。`rows_pointer` 可指向数组或以模型 ID 为键的对象，`required_pointers` 用于只选取包含指定属性的条目。`destination` 声明从目录新建模型时使用的默认请求策略，前端填入选中的 `target` 和 `model`。

`on_update` 可配置目录变化后的适配程序，`update_settings` 保存其参数。程序从标准输入读取模型数组和设置；执行成功后发布新目录。Antigravity 的桥接同步使用 `scripts/sync-bridge-catalog.py`（依赖 PyYAML），将服务端模型元数据写入 CLIProxyAPI 支持的 `models.catalog` 本机文件。文件内容变化时切换两个固定文件名，触发桥接服务重新加载；相同内容复用现有文件。这样新增模型可以在桥接服务中注册，已有会话继续使用当前路由。

目录检查间隔允许 30–86400 秒，单次查询超时允许 1–120 秒，响应上限为 8 MiB。目录参数、供应商地址和模型名称均来自 JSON，通用查询、字段提取、缓存和比较由 Rust 执行。

## Cherry Studio 参考

交互参考 Cherry Studio 的 [模型同步抽屉](https://github.com/CherryHQ/cherry-studio/blob/04649fa0203056ce29090a59347205f1ea2c8d1f/src/renderer/pages/settings/ProviderSettings/ModelList/ModelListSyncDrawer.tsx)：完整目录与已添加模型分别维护，支持搜索、刷新和选择。

Router 的目录数据直接来自服务端。Grok CLI 使用 `https://cli-chat-proxy.grok.com/v1/models`，携带现有 OAuth 授权及配置的客户端协议头；Antigravity 使用 `fetchAvailableModels`，通过现有 Google 授权和项目 ID 查询。目录响应提供当前账号的模型条目、显示名称和可用元数据。
