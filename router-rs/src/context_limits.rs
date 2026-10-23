use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control::{self, ProcessSpec};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextDiscovery {
    pub adapter: ProcessSpec,
    pub settings: Value,
}

pub fn discover(path: &Path, config: &Value, name: Option<&str>, refresh: bool) -> Result<Value> {
    let manager = control::management(config)?;
    let Some(discovery) = manager.context_discovery else {
        return Ok(json!({"commands":{}}));
    };
    let context = json!({"config_path":path,"config_directory":path.parent()});
    let mut child = Command::new(control::render(&discovery.adapter.program, &context)?)
        .args(
            discovery
                .adapter
                .args
                .iter()
                .map(|arg| control::render(arg, &context))
                .collect::<Result<Vec<_>>>()?,
        )
        .stdin(Stdio::piped())
        .env("ROUTER_EXECUTABLE", std::env::current_exe()?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动上下文目录适配程序")?;
    let input = json!({"config":config,"config_directory":path.parent(),"command_name":name,"refresh":refresh});
    child
        .stdin
        .take()
        .context("上下文目录输入不可用")?
        .write_all(&serde_json::to_vec(&input)?)?;
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "上下文上限识别失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).context("上下文目录返回无效数据")?;
    ensure!(result["commands"].is_object(), "上下文目录缺少命令结果");
    Ok(result)
}

pub fn validate(path: &Path, config: &Value) -> Result<()> {
    let manager = control::management(config)?;
    if manager.context_discovery.is_none()
        || !manager.commands.values().any(|command| {
            command
                .model_bindings
                .iter()
                .any(|item| item.context_window.is_some() || item.max_context)
        })
    {
        return Ok(());
    }
    let result = discover(path, config, None, false)?;
    for (name, command) in &manager.commands {
        for binding in &command.model_bindings {
            if binding.context_window.is_none() && !binding.max_context {
                continue;
            }
            let entry = &result["commands"][name][&binding.id];
            let window = if binding.max_context {
                ensure!(
                    entry["complete"].as_bool() == Some(true),
                    "模型 {} 的目录尚未公布全部上下文上限",
                    binding.id
                );
                entry["max_tokens"]
                    .as_u64()
                    .context("模型上下文上限未知")?
                    .min(
                        manager.templates[&command.template]
                            .context_window_range
                            .context("启动方式缺少上下文范围")?
                            .max,
                    )
            } else {
                binding.context_window.context("缺少固定上下文")?
            };
            let destinations = entry["destinations"]
                .as_array()
                .context("上下文目录缺少模型结果")?;
            for destination in destinations {
                ensure!(
                    destination.get("error").is_none(),
                    "命令 {name} 的模型 {}：{}",
                    binding.id,
                    destination["error"]
                );
            }
            if let Some(maximum) = entry["max_tokens"].as_u64() {
                ensure!(
                    window <= maximum,
                    "命令 {name} 的模型 {} 上下文预算不能超过后端上限 {maximum} tokens",
                    binding.id
                );
            }
        }
    }
    Ok(())
}
