use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

use crate::control::{self, ApiError, ControlState, ProcessSpec};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub label: String,
    pub backends: Vec<String>,
    pub adapter: ProcessSpec,
    pub settings: Value,
    pub ttl_seconds: u64,
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_backend: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Window {
    label: String,
    remaining_percent: Option<f64>,
    reset_at: Option<String>,
    #[serde(default)]
    disabled: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Group {
    label: String,
    windows: Vec<Window>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    label: String,
    value: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Quota {
    groups: Vec<Group>,
    #[serde(default)]
    details: Vec<Detail>,
    #[serde(default)]
    error: Option<String>,
}

struct Snapshot {
    source: Source,
    attempted: Instant,
    updated_at: Option<u64>,
    data: Option<Quota>,
    error: Option<String>,
}

#[derive(Default)]
pub(crate) struct Service {
    entries: Mutex<BTreeMap<String, Arc<Mutex<Option<Snapshot>>>>>,
}

#[derive(Deserialize)]
struct ReadRequest {
    id: String,
    #[serde(default)]
    refresh: bool,
}

async fn query(state: &ControlState, source: &Source) -> Result<Quota> {
    ensure!(
        (1..=120).contains(&source.timeout_seconds),
        "配额查询超时须为 1–120 秒"
    );
    ensure!(
        (15..=3600).contains(&source.ttl_seconds),
        "配额刷新间隔须为 15–3600 秒"
    );
    let token = if let Some(id) = &source.auth_backend {
        let config = crate::load_router_runtime_config(&state.path)?;
        let auth = config
            .runtime
            .backends
            .get(id)
            .and_then(|backend| backend.auth.as_ref())
            .context("配额来源的后端授权不存在")?;
        Some(
            tokio::time::timeout(
                Duration::from_secs(source.timeout_seconds),
                crate::read_backend_token(auth),
            )
            .await
            .context("配额授权读取超时")?
            .context("配额授权读取失败，请查看对应后端的登录状态")?,
        )
    } else {
        None
    };
    let context =
        json!({"config_path":state.path.as_ref(), "config_directory":state.path.parent()});
    let mut child =
        tokio::process::Command::new(control::render(&source.adapter.program, &context)?)
            .args(
                source
                    .adapter
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
            .context("配额适配程序启动失败")?;
    let operation = async {
        let input = json!({"settings":source.settings,"token":token,"config_directory":state.path.parent()});
        let mut stdin = child.stdin.take().context("配额输入不可用")?;
        stdin.write_all(&serde_json::to_vec(&input)?).await?;
        drop(stdin);
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .context("配额输出不可用")?
            .take(262_145)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(bytes.len() <= 262_144, "配额返回超过 256 KiB");
        ensure!(child.wait().await?.success(), "配额适配程序执行失败");
        let result: Quota = serde_json::from_slice(&bytes).context("配额返回格式无效")?;
        for window in result.groups.iter().flat_map(|group| &group.windows) {
            if let Some(percent) = window.remaining_percent {
                ensure!(
                    percent.is_finite() && (0.0..=100.0).contains(&percent),
                    "上游剩余百分比超出范围"
                );
            }
        }
        Ok(result)
    };
    let result = tokio::time::timeout(Duration::from_secs(source.timeout_seconds), operation)
        .await
        .context("配额查询超时")?;
    if result.is_err() {
        let _ = child.kill().await;
    }
    result
}

async fn read(
    State(state): State<ControlState>,
    Json(request): Json<ReadRequest>,
) -> Result<Json<Value>, ApiError> {
    let requested = Instant::now();
    let value = control::load_value(&state.path)?;
    let sources = control::management(&value)?.quota_sources;
    let source = sources.get(&request.id).context("此配额来源尚未配置")?;
    let entry = {
        let mut entries = state.quotas.entries.lock().await;
        entries.retain(|id, _| sources.contains_key(id));
        entries.entry(request.id.clone()).or_default().clone()
    };
    let mut cached = entry.lock().await;
    let reuse = cached.as_ref().is_some_and(|snapshot| {
        snapshot.source == *source
            && (snapshot.attempted >= requested
                || (!request.refresh
                    && snapshot.attempted.elapsed() < Duration::from_secs(source.ttl_seconds)))
    });
    if !reuse {
        let result = query(&state, source).await;
        let previous = cached.take().filter(|snapshot| snapshot.source == *source);
        let mut snapshot = Snapshot {
            source: source.clone(),
            attempted: Instant::now(),
            data: previous.as_ref().and_then(|s| s.data.clone()),
            updated_at: previous.and_then(|s| s.updated_at),
            error: None,
        };
        match result {
            Ok(data) if data.error.is_none() => {
                snapshot.updated_at = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());
                snapshot.data = Some(data);
            }
            Ok(data) => snapshot.error = data.error,
            Err(error) => snapshot.error = Some(error.to_string()),
        }
        *cached = Some(snapshot);
    }
    let snapshot = cached.as_ref().context("配额结果不可用")?;
    Ok(Json(
        json!({"id":request.id,"label":source.label,"data":snapshot.data,"updated_at":snapshot.updated_at,"error":snapshot.error,"stale":snapshot.error.is_some(),"ttl_seconds":source.ttl_seconds}),
    ))
}

pub(crate) fn routes() -> Router<ControlState> {
    Router::new().route("/quotas/read", post(read))
}
