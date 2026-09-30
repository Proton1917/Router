use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::control::{self, ProcessSpec};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Integration {
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub protocol: String,
    pub gateway_url: String,
    pub health_path: String,
    pub request_path: String,
    #[serde(default)]
    pub require_https: bool,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub slots: Vec<Slot>,
    #[serde(default)]
    pub catalog_pointer: Option<String>,
    #[serde(default)]
    pub adapter: Option<ProcessSpec>,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub reload_hint: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub model: String,
    pub context: String,
    pub profile: String,
    #[serde(default)]
    pub label: String,
    pub visible: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Integration {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.label.is_empty(), "客户端名称不能为空");
        let url = reqwest::Url::parse(&self.gateway_url)?;
        ensure!(
            matches!(url.scheme(), "http" | "https"),
            "接入地址需要使用 HTTP 或 HTTPS"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "接入地址不能包含凭据"
        );
        ensure!(
            !self.require_https || url.scheme() == "https",
            "此客户端需要 HTTPS 接入地址"
        );
        ensure!(self.timeout_ms > 0, "客户端操作超时必须大于零");
        ensure!(
            self.health_path.starts_with('/') && self.request_path.starts_with('/'),
            "客户端接口路径必须以 / 开始"
        );
        for (name, value) in &self.headers {
            name.parse::<http::HeaderName>()?;
            value.parse::<http::HeaderValue>()?;
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut models = std::collections::BTreeSet::new();
        for slot in &self.slots {
            control::validate_name(&slot.id)?;
            ensure!(
                ids.insert(&slot.id) && models.insert(&slot.model),
                "客户端模型入口重复"
            );
            ensure!(
                !slot.model.is_empty() && !slot.context.is_empty(),
                "客户端模型和上下文不能为空"
            );
        }
        ensure!(
            self.slots.is_empty() || self.slots.iter().any(|slot| slot.visible),
            "至少需要显示一个客户端模型"
        );
        if let Some(pointer) = &self.catalog_pointer {
            ensure!(
                pointer.starts_with('/'),
                "模型目录位置需要使用 JSON Pointer"
            );
        }
        Ok(())
    }
}

pub fn slot_label(value: &Value, slot: &Slot) -> String {
    if !slot.label.is_empty() {
        return slot.label.clone();
    }
    let destination = &value["profiles"][&slot.profile]["standard"];
    let model = destination["model"].as_str().unwrap_or(&slot.model);
    let target = destination["target"].as_str().unwrap_or("");
    let labels = &value["management"]["labels"];
    let model_label = labels[format!("model:{model}")].as_str().unwrap_or(model);
    let target_label = labels[format!("backend:{target}")]
        .as_str()
        .unwrap_or(target);
    format!("{model_label} · {target_label}")
}

pub fn materialize(value: &mut Value, managed: &mut Vec<String>) -> Result<()> {
    let manager = control::management(value)?;
    for (name, integration) in &manager.integrations {
        integration.validate()?;
        for command in &integration.commands {
            ensure!(
                manager.commands.contains_key(command),
                "客户端 {name} 引用了不存在的命令 {command}"
            );
        }
        let mut catalog = Vec::new();
        for slot in &integration.slots {
            let profile = value["profiles"]
                .get(&slot.profile)
                .context("客户端模型配置不存在")?;
            let target = profile["standard"]["target"]
                .as_str()
                .context("模型后端不存在")?;
            let target_value = json!(target);
            ensure!(
                profile["standard"]["model"]
                    .as_str()
                    .is_some_and(|model| !model.is_empty()),
                "客户端入口需要明确的后端模型"
            );
            if let Some(protocols) = manager.backend_protocols.get(target) {
                ensure!(
                    protocols.contains(&integration.protocol),
                    "客户端 {name} 的后端不支持 {}",
                    integration.protocol
                );
            }
            let id = format!("integration-{name}-{}", slot.id);
            let route = json!({"id":id,"match":{"contexts":[slot.context],"models":[slot.model]},"profile":slot.profile});
            let routes = value["routes"]
                .as_array_mut()
                .context("routes 必须为数组")?;
            ensure!(
                !routes.iter().any(|route| route["id"] == id),
                "客户端路由名称冲突: {id}"
            );
            routes.insert(0, route);
            value["validation_cases"].as_array_mut().context("validation_cases 必须为数组")?.push(json!({
                "name":id,"context":slot.context,"model":slot.model,"headers":integration.headers,
                "expected":{"rule_id":id,"target":target_value,"model":slot.model}
            }));
            managed.push(id);
            if slot.visible {
                let mut row = serde_json::to_value(&slot.metadata)?;
                row["id"] = json!(slot.model);
                row["name"] = json!(slot_label(value, slot));
                catalog.push(row);
            }
        }
        if let Some(pointer) = &integration.catalog_pointer {
            ensure!(!catalog.is_empty(), "客户端 {name} 至少需要一个显示的模型");
            *value
                .pointer_mut(pointer)
                .context("客户端模型目录位置不存在")? = json!(catalog);
        }
    }
    Ok(())
}

pub async fn perform(path: &Path, id: &str, action: &str) -> Result<Value> {
    ensure!(
        ["inspect", "sync", "check"].contains(&action),
        "未知客户端接入操作"
    );
    let config = control::load_value(path)?;
    let manager = control::management(&config)?;
    let integration = manager.integrations.get(id).context("客户端接入不存在")?;
    integration.validate()?;
    if let Some(adapter) = &integration.adapter {
        let context = json!({"config_path":path,"id":id,"action":action,"executable":std::env::current_exe()?});
        let mut command =
            tokio::process::Command::new(control::render(&adapter.program, &context)?);
        for arg in &adapter.args {
            command.arg(control::render(arg, &context)?);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("无法启动客户端接入程序")?;
        let input = json!({"action":action,"id":id,"config_path":path,"config":config,"integration":integration});
        let operation = async move {
            child
                .stdin
                .take()
                .context("客户端接入输入不可用")?
                .write_all(&serde_json::to_vec(&input)?)
                .await?;
            child.wait_with_output().await.map_err(anyhow::Error::from)
        };
        let output = tokio::time::timeout(Duration::from_millis(integration.timeout_ms), operation)
            .await
            .context("客户端接入操作超时")??;
        ensure!(
            output.status.success(),
            "客户端接入程序执行失败，请检查其配置"
        );
        let result: Value =
            serde_json::from_slice(&output.stdout).context("客户端接入程序返回了无效 JSON")?;
        ensure!(
            result["ok"].as_bool() == Some(true),
            "{}",
            result["error"].as_str().unwrap_or("客户端接入操作失败")
        );
        return Ok(result);
    }
    if action == "sync" {
        crate::cli::install_commands(path, None)?;
    }
    let url = format!(
        "{}{}",
        integration.gateway_url.trim_end_matches('/'),
        integration.health_path
    );
    let response = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_millis(integration.timeout_ms))
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "客户端网关健康检查失败: {}",
        response.status()
    );
    Ok(
        json!({"ok":true,"configured":true,"gateway_url":integration.gateway_url,"message":"终端入口已接入；启动设置在下次运行命令时读取。","commands":integration.commands}),
    )
}
