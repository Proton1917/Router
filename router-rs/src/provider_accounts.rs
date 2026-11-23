use std::{collections::BTreeMap, process::Stdio, time::Duration};

use anyhow::{Context, Result, ensure};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::control::{self, ApiError, ControlState, ProcessSpec};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub label: String,
    pub backends: Vec<String>,
    pub adapter: Option<ProcessSpec>,
    #[serde(default)]
    pub settings: Value,
    pub quota_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<crate::oauth_accounts::Source>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Action {
    Status,
    Login,
    Cancel,
    Prefer,
    Rename,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    action: Action,
    account: Option<String>,
    label: Option<String>,
}

pub fn validate(
    sources: &BTreeMap<String, Source>,
    quotas: &BTreeMap<String, crate::quotas::Source>,
) -> Result<()> {
    for (id, source) in sources {
        control::validate_name(id)?;
        ensure!(
            source.adapter.is_some() != source.native.is_some(),
            "账号服务 {id} 须选择一种授权方式"
        );
        if let Some(adapter) = &source.adapter {
            ensure!(!adapter.program.is_empty(), "账号管理需要适配程序");
        }
        if let Some(quota) = &source.quota_source {
            ensure!(
                quotas.contains_key(quota),
                "账号服务 {id} 引用的配额来源 {quota} 缺失"
            );
        }
    }
    Ok(())
}

pub(crate) fn sources(config: &crate::routing::RouterConfig) -> BTreeMap<String, Source> {
    let Some(manager) = &config.management else {
        return BTreeMap::new();
    };
    let mut sources = manager.account_providers.clone();
    let mut groups = BTreeMap::<&str, Vec<String>>::new();
    for (id, backend) in &config.runtime.backends {
        if let Some(settings) = backend.auth.as_ref().and_then(|auth| auth.oauth.as_ref())
            && !sources.values().any(|source| source.backends.contains(id))
        {
            groups
                .entry(&settings.credential_file)
                .or_default()
                .push(id.clone());
        }
    }
    for mut backends in groups.into_values() {
        backends.sort();
        let quota = manager.quota_sources.iter().find(|(id, quota)| {
            !sources
                .values()
                .any(|source| source.quota_source.as_ref() == Some(id))
                && quota
                    .auth_backend
                    .as_ref()
                    .is_some_and(|id| backends.contains(id))
        });
        let backend = quota
            .and_then(|(_, quota)| quota.auth_backend.clone())
            .unwrap_or_else(|| backends[0].clone());
        let settings = config.runtime.backends[&backend]
            .auth
            .as_ref()
            .and_then(|auth| auth.oauth.as_ref())
            .expect("OAuth backend group");
        let label = quota
            .map(|(_, quota)| quota.label.clone())
            .unwrap_or_else(|| settings.provider.clone());
        sources.insert(
            format!("oauth:{backend}"),
            Source {
                label,
                backends,
                adapter: None,
                settings: Value::Null,
                quota_source: quota.map(|(id, _)| id.clone()),
                native: Some(crate::oauth_accounts::Source { backend }),
            },
        );
    }
    sources
}

async fn action(
    State(state): State<ControlState>,
    Json(request): Json<Request>,
) -> Result<Json<Value>, ApiError> {
    let config = crate::load_router_runtime_config(&state.path)?;
    let manager = config.management.as_ref().context("管理界面缺少配置")?;
    let sources = sources(&config);
    let source = sources
        .get(&request.id)
        .context("请先配置此服务的账号管理")?;
    let mut result = if let Some(native) = &source.native {
        let pool = crate::oauth_accounts::Pool::new(&config, native)?;
        let key = pool.session_key();
        let session = match request.action {
            Action::Login => Some(pool.login().await?),
            Action::Cancel => {
                crate::oauth::cancel_login(&key).await;
                None
            }
            Action::Rename | Action::Prefer => {
                let _guard = state.account_updates.lock().await;
                let id = request.account.as_deref().context("请选择账号")?;
                let label = if request.action == Action::Rename {
                    let label = request
                        .label
                        .as_deref()
                        .map(str::trim)
                        .context("请填写账号名称")?;
                    if !(1..=80).contains(&label.chars().count()) {
                        return Err(anyhow::anyhow!("账号名称须为 1–80 个字符").into());
                    }
                    Some(label)
                } else {
                    None
                };
                pool.update(id, label)?;
                crate::oauth::login_status(&key).await
            }
            Action::Status => crate::oauth::login_status(&key).await,
        };
        json!({"ok":true,"accounts":pool.public()?,"session":session})
    } else {
        call_adapter(&state, source, serde_json::to_value(&request)?).await?
    };
    if request.action == Action::Login {
        let url = result["session"]["url"].as_str().context("授权地址缺失")?;
        result["browser_opened"] = json!(crate::oauth::open_browser(&manager.browser, url).await?);
    }
    Ok(Json(result))
}

pub(crate) async fn call_adapter(
    state: &ControlState,
    source: &Source,
    request: Value,
) -> Result<Value> {
    let adapter = source.adapter.as_ref().context("账号服务缺少适配程序")?;
    let context =
        json!({"config_path":state.path.as_ref(), "config_directory":state.path.parent()});
    let mut child = tokio::process::Command::new(control::render(&adapter.program, &context)?)
        .args(
            adapter
                .args
                .iter()
                .map(|arg| control::render(arg, &context))
                .collect::<Result<Vec<_>>>()?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("账号管理程序启动失败")?;
    let input = json!({"request":request, "settings":source.settings});
    let operation = async {
        let mut stdin = child.stdin.take().context("账号管理输入不可用")?;
        stdin.write_all(&serde_json::to_vec(&input)?).await?;
        drop(stdin);
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .context("账号管理输出不可用")?
            .take(262_145)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(bytes.len() <= 262_144, "账号管理返回超过 256 KiB");
        ensure!(child.wait().await?.success(), "账号管理程序执行失败");
        let result: Value = serde_json::from_slice(&bytes).context("账号管理返回格式无效")?;
        ensure!(
            result["ok"].as_bool() == Some(true),
            "{}",
            result["error"].as_str().unwrap_or("账号操作失败")
        );
        Ok::<_, anyhow::Error>(result)
    };
    tokio::time::timeout(Duration::from_secs(45), operation)
        .await
        .context("账号操作超时")?
}

pub(crate) fn routes() -> Router<ControlState> {
    Router::new().route("/provider-accounts/action", post(action))
}
