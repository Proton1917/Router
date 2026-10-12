use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, ensure};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control::{self, CommandSpec, LaunchTemplate};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelBinding {
    pub id: String,
    pub model: String,
    pub profile: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub route_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

fn visible() -> bool {
    true
}

pub fn validate(name: &str, command: &CommandSpec, template: &LaunchTemplate) -> Result<()> {
    if command.model_bindings.is_empty() {
        return Ok(());
    }
    ensure!(
        command.profile.is_none() && !command.match_all_models,
        "命令 {name} 的多模型列表不能同时绑定单一模型路由"
    );
    ensure!(
        template.request_path.is_some(),
        "命令 {name} 的多模型列表需要 API 接口路径"
    );
    ensure!(
        command.model_bindings.iter().any(|item| item.visible),
        "命令 {name} 至少需要一个可见模型"
    );
    let mut ids = BTreeSet::new();
    let mut models = BTreeSet::new();
    let mut linked = BTreeSet::new();
    for binding in &command.model_bindings {
        control::validate_name(&binding.id)?;
        if let Some(window) = binding.context_window {
            let range = template
                .context_window_range
                .context("启动方式未声明逐模型上下文设置")?;
            ensure!(
                window >= range.min && window <= range.max,
                "命令 {name} 的模型 {} 上下文预算需在 {}～{} tokens 之间",
                binding.id,
                range.min,
                range.max
            );
        }
        ensure!(ids.insert(&binding.id), "命令 {name} 的模型条目标识重复");
        for model in std::iter::once(&binding.model).chain(&binding.aliases) {
            ensure!(
                !model.trim().is_empty() && models.insert(model.to_lowercase()),
                "命令 {name} 的模型标识或兼容名称为空或重复: {model}"
            );
        }
        for id in &binding.route_ids {
            ensure!(linked.insert(id), "命令 {name} 的兼容路由绑定重复: {id}");
        }
    }
    if let Some(model) = &command.model {
        ensure!(
            command
                .model_bindings
                .iter()
                .any(|item| item.visible && &item.model == model),
            "命令 {name} 的默认模型必须在可见模型列表中"
        );
    }
    Ok(())
}

pub fn materialize(value: &mut Value, managed: &mut Vec<String>) -> Result<()> {
    let manager = control::management(value)?;
    let runtime: crate::runtime::RuntimeConfig = serde_json::from_value(value["runtime"].clone())?;
    let mut replaced_profiles = BTreeSet::new();
    let mut linked_routes = BTreeSet::new();
    for (name, command) in &manager.commands {
        let template = &manager.templates[&command.template];
        validate(name, command, template)?;
        if command.model_bindings.is_empty() {
            continue;
        }
        let path = template.request_path.as_deref().context("缺少接口路径")?;
        let protocol = value["runtime"]["server"]["endpoints"][path]
            .as_str()
            .context("接口路径未声明协议")?
            .to_owned();
        let context = control::command_context(value, name)?;
        let mut headers = HeaderMap::new();
        let mut case_headers = BTreeMap::new();
        for (key, text) in &template.request_headers {
            let text = control::render(text, &context)?;
            headers.insert(key.parse::<http::HeaderName>()?, text.parse()?);
            case_headers.insert(key.clone(), text);
        }
        headers.insert(
            manager.command_header.parse::<http::HeaderName>()?,
            name.parse()?,
        );
        case_headers.insert(manager.command_header.clone(), name.clone());
        let client = runtime.client_for(&headers, path)?;
        for binding in &command.model_bindings {
            let profile = value["profiles"]
                .get(&binding.profile)
                .context("命令模型配置不存在")?;
            for speed in ["standard", "fast"] {
                if let Some(destination) = profile.get(speed) {
                    let target = destination["target"].as_str().context("模型后端不存在")?;
                    if let Some(protocols) = manager.backend_protocols.get(target) {
                        ensure!(
                            protocols.contains(&protocol),
                            "命令 {name} 的后端 {target} 不支持 {protocol}"
                        );
                    }
                }
            }
            let target = profile["standard"]["target"].clone();
            for id in &binding.route_ids {
                ensure!(
                    linked_routes.insert(id),
                    "兼容路由不能由多个模型条目管理: {id}"
                );
                let route = value["routes"]
                    .as_array_mut()
                    .context("routes 必须为数组")?
                    .iter_mut()
                    .find(|route| route["id"] == *id)
                    .with_context(|| format!("兼容路由不存在: {id}"))?;
                let contexts = route["match"]["contexts"]
                    .as_array()
                    .context("兼容路由必须明确限定客户端上下文")?;
                ensure!(
                    contexts.len() == 1 && contexts[0] == client.id,
                    "兼容路由 {id} 跨越了命令 {name} 的客户端范围"
                );
                if let Some(old) = route["profile"].as_str() {
                    replaced_profiles.insert(old.to_owned());
                }
                route["profile"] = json!(binding.profile);
            }
            let id = format!("command-{name}-model-{}", binding.id);
            let route = json!({"id":id,"match":{"contexts":[client.id],"headers":[{"name":manager.command_header,"values":[name]}],"models":[binding.model]},"profile":binding.profile});
            let routes = value["routes"]
                .as_array_mut()
                .context("routes 必须为数组")?;
            ensure!(
                !routes.iter().any(|route| route["id"] == id),
                "命令模型路由名称冲突: {id}"
            );
            let before = command
                .route_before
                .as_ref()
                .or(template.route_before.as_ref());
            let index = if let Some(before) = before {
                routes
                    .iter()
                    .position(|route| route["id"] == *before)
                    .with_context(|| format!("路由插入位置不存在: {before}"))?
            } else {
                routes.len().saturating_sub(1)
            };
            routes.insert(index, route);
            value["validation_cases"].as_array_mut().context("缺少验证用例")?.push(json!({"name":id,"context":client.id,"model":binding.model,"headers":case_headers,"expected":{"rule_id":id,"target":target,"model":binding.model}}));
            managed.push(id);
        }
    }
    for id in replaced_profiles {
        let routed = value["routes"]
            .as_array()
            .context("缺少路由")?
            .iter()
            .any(|route| route["profile"] == id);
        let bound = manager.commands.values().any(|command| {
            command.profile.as_ref() == Some(&id)
                || command.model_bindings.iter().any(|item| item.profile == id)
        }) || manager
            .integrations
            .values()
            .any(|integration| integration.slots.iter().any(|slot| slot.profile == id));
        if !routed && !bound {
            value["profiles"]
                .as_object_mut()
                .context("缺少模型配置")?
                .remove(&id);
            value["management"]["profile_models"]
                .as_object_mut()
                .map(|models| models.remove(&id));
            value["management"]["labels"]
                .as_object_mut()
                .map(|labels| labels.remove(&format!("profile:{id}")));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdaptedLaunch {
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

pub fn adapt(
    path: &Path,
    name: &str,
    args: Vec<String>,
    environment: &mut BTreeMap<String, String>,
) -> Result<Vec<String>> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let command = &manager.commands[name];
    let template = &manager.templates[&command.template];
    let Some(adapter) = &template.model_adapter else {
        return Ok(args);
    };
    if command.model_bindings.is_empty()
        || template.api_gate.as_ref().is_some_and(|gate| {
            !environment
                .get(&gate.name)
                .cloned()
                .or_else(|| std::env::var(&gate.name).ok())
                .is_some_and(|value| value.contains(&gate.contains))
        })
    {
        return Ok(args);
    }
    let mut context = control::command_context(&value, name)?;
    context["config_path"] = json!(path);
    context["config_directory"] = json!(path.parent().context("配置目录不存在")?);
    let mut child = Command::new(control::render(&adapter.program, &context)?)
        .args(
            adapter
                .args
                .iter()
                .map(|arg| control::render(arg, &context))
                .collect::<Result<Vec<_>>>()?,
        )
        .envs(&*environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动模型菜单适配程序")?;
    let input =
        json!({"config":value,"config_directory":path.parent(),"command_name":name,"args":args});
    child
        .stdin
        .take()
        .context("模型菜单适配输入不可用")?
        .write_all(&serde_json::to_vec(&input)?)?;
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "模型菜单适配失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let adapted: AdaptedLaunch =
        serde_json::from_slice(&output.stdout).context("模型菜单适配程序返回无效数据")?;
    environment.extend(adapted.env);
    Ok(adapted.args)
}
