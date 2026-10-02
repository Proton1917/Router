use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::control::{self, ProcessSpec};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModConfig {
    pub adapter: ProcessSpec,
    pub timeout_ms: u64,
    pub settings: Value,
    #[serde(default)]
    pub catalog: BTreeMap<String, ModEntry>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ModProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModEntry {
    pub label: String,
    pub plugin_id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub availability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModProfile {
    pub label: String,
    #[serde(default)]
    pub plugins: BTreeMap<String, bool>,
}

impl ModConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.adapter.program.is_empty() && self.timeout_ms > 0,
            "Mods 适配程序与超时配置无效"
        );
        let mut plugins = std::collections::BTreeSet::new();
        for (id, entry) in &self.catalog {
            control::validate_name(id)?;
            ensure!(
                !entry.label.trim().is_empty() && !entry.plugin_id.trim().is_empty(),
                "Mod 名称和插件标识不能为空"
            );
            ensure!(plugins.insert(&entry.plugin_id), "Mod 插件标识重复");
            ensure!(
                entry
                    .path
                    .as_ref()
                    .is_none_or(|path| !path.trim().is_empty()),
                "Mod 路径不能为空"
            );
        }
        for (id, profile) in &self.profiles {
            control::validate_name(id)?;
            ensure!(!profile.label.trim().is_empty(), "Mod 组合名称不能为空");
            for plugin in profile.plugins.keys() {
                ensure!(
                    self.catalog.contains_key(plugin),
                    "Mod 组合引用了不存在的条目: {plugin}"
                );
            }
        }
        Ok(())
    }
}

pub fn wrap(
    path: &Path,
    name: &str,
    program: String,
    args: Vec<String>,
) -> Result<(String, Vec<String>)> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let entry = manager.commands.get(name).context("启动命令不存在")?;
    if entry.mod_profile.is_none() {
        return Ok((program, args));
    }
    let mods = manager.mods.context("未配置 Mods 管理")?;
    let context = json!({"config_path":path,"config_directory":path.parent(),"command_name":name});
    let wrapper = control::render(&mods.adapter.program, &context)?;
    let mut wrapped = mods
        .adapter
        .args
        .iter()
        .map(|arg| control::render(arg, &context))
        .collect::<Result<Vec<_>>>()?;
    wrapped.extend([
        "launch".to_owned(),
        path.to_string_lossy().into_owned(),
        name.to_owned(),
        "--".to_owned(),
        program,
    ]);
    wrapped.extend(args);
    Ok((wrapper, wrapped))
}

pub async fn perform(path: &Path, action: &str, entry: Option<&str>) -> Result<Value> {
    ensure!(["inspect", "validate"].contains(&action), "未知 Mods 操作");
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let mods = manager.mods.context("尚未配置 Mods 适配程序")?;
    let context = json!({"config_path":path,"config_directory":path.parent()});
    let mut process =
        tokio::process::Command::new(control::render(&mods.adapter.program, &context)?);
    for arg in &mods.adapter.args {
        process.arg(control::render(arg, &context)?);
    }
    process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = process.spawn().context("无法启动 Mods 适配程序")?;
    let input = json!({"action":action,"entry":entry,"mods":mods});
    let operation = async move {
        child
            .stdin
            .take()
            .context("Mods 输入不可用")?
            .write_all(&serde_json::to_vec(&input)?)
            .await?;
        child.wait_with_output().await.map_err(anyhow::Error::from)
    };
    let output = tokio::time::timeout(Duration::from_millis(mods.timeout_ms), operation)
        .await
        .context("Mods 操作超时")??;
    ensure!(output.status.success(), "Mods 适配程序执行失败");
    let result: Value = serde_json::from_slice(&output.stdout).context("Mods 返回格式无效")?;
    ensure!(
        result["ok"] == true,
        "{}",
        result["error"].as_str().unwrap_or("Mods 操作失败")
    );
    Ok(result)
}
