use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};

use crate::control::{self, ApiError, ControlState};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Source {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub path: String,
    pub request_url: Option<String>,
    pub credential_file: Option<PathBuf>,
    pub token_pointer: String,
    pub body_credentials: BTreeMap<String, String>,
    pub request_body: Option<Value>,
    pub required_pointers: Vec<String>,
    pub capability_flags: BTreeMap<String, String>,
    pub on_update: Option<control::ProcessSpec>,
    pub update_settings: Value,
    pub rows_pointer: String,
    pub id_pointers: Vec<String>,
    pub name_pointers: Vec<String>,
    pub context_pointers: Vec<String>,
    pub capability_pointers: Vec<String>,
    pub filter: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub auth_headers: BTreeMap<String, String>,
    pub destination: Value,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 30,
            timeout_seconds: 20,
            path: "/v1/models".into(),
            request_url: None,
            credential_file: None,
            token_pointer: "/access_token".into(),
            body_credentials: BTreeMap::new(),
            request_body: None,
            required_pointers: Vec::new(),
            capability_flags: BTreeMap::new(),
            on_update: None,
            update_settings: Value::Null,
            rows_pointer: "/data".into(),
            id_pointers: vec!["/id".into()],
            name_pointers: vec!["/name".into(), "/display_name".into()],
            context_pointers: vec!["/context_length".into(), "/context_window".into()],
            capability_pointers: vec![
                "/capabilities".into(),
                "/architecture/input_modalities".into(),
            ],
            filter: BTreeMap::new(),
            headers: BTreeMap::new(),
            auth_headers: BTreeMap::new(),
            destination: Value::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Model {
    id: String,
    name: String,
    context_window: Option<u64>,
    capabilities: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Snapshot {
    models: Vec<Model>,
    baseline: Option<Vec<Model>>,
    updated_at: Option<u64>,
    checked_at: Option<u64>,
    error: Option<String>,
    source: Source,
    base_url: String,
}

#[derive(Default)]
pub(crate) struct Service {
    entries: Mutex<BTreeMap<String, Arc<Mutex<Option<Snapshot>>>>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs()
}

fn pointers<'a>(row: &'a Value, paths: &[String]) -> Option<&'a Value> {
    paths
        .iter()
        .filter_map(|path| row.pointer(path))
        .find(|value| !value.is_null())
}

fn normalize(data: &Value, source: &Source) -> Result<Vec<Model>> {
    let collection = data
        .pointer(&source.rows_pointer)
        .context("模型目录缺少配置的条目字段")?;
    let rows: Vec<_> = match collection {
        Value::Array(rows) => rows.iter().map(|row| (None, row)).collect(),
        Value::Object(rows) => rows
            .iter()
            .map(|(key, row)| (Some(key.as_str()), row))
            .collect(),
        _ => anyhow::bail!("模型目录需要数组或对象"),
    };
    let mut models = BTreeMap::new();
    for (key, row) in rows {
        if source
            .required_pointers
            .iter()
            .any(|pointer| row.pointer(pointer).is_none())
        {
            continue;
        }
        if source.filter.iter().any(|(pointer, expected)| {
            row.pointer(pointer).and_then(Value::as_str) != Some(expected)
        }) {
            continue;
        }
        let id = pointers(row, &source.id_pointers)
            .and_then(Value::as_str)
            .or(key)
            .context("模型目录条目缺少 ID")?
            .trim();
        ensure!(!id.is_empty(), "模型目录包含空 ID");
        let name = pointers(row, &source.name_pointers)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(id);
        let mut capabilities: Vec<String> = source
            .capability_pointers
            .iter()
            .filter_map(|pointer| row.pointer(pointer).and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        capabilities.extend(
            source
                .capability_flags
                .iter()
                .filter(|(pointer, _)| row.pointer(pointer).and_then(Value::as_bool) == Some(true))
                .map(|(_, label)| label.clone()),
        );
        capabilities.sort();
        capabilities.dedup();
        models.insert(
            id.to_owned(),
            Model {
                id: id.into(),
                name: name.into(),
                context_window: pointers(row, &source.context_pointers).and_then(Value::as_u64),
                capabilities,
            },
        );
    }
    Ok(models.into_values().collect())
}

async fn fetch(backend: &crate::runtime::BackendConfig, source: &Source) -> Result<Vec<Model>> {
    ensure!(
        (30..=86400).contains(&source.interval_seconds),
        "目录检查间隔须为 30–86400 秒"
    );
    ensure!(
        (1..=120).contains(&source.timeout_seconds),
        "目录查询超时须为 1–120 秒"
    );
    let credential = source
        .credential_file
        .as_ref()
        .map(|file| -> Result<Value> {
            Ok(serde_json::from_slice(
                &std::fs::read(file).context("目录授权文件读取失败")?,
            )?)
        })
        .transpose()?;
    let url = if let Some(url) = &source.request_url {
        let parsed = reqwest::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.username().is_empty()
                && parsed.password().is_none(),
            "目录服务需要不含凭据的 HTTPS 地址"
        );
        ensure!(credential.is_some(), "独立目录服务需要配置其授权文件");
        url.clone()
    } else {
        ensure!(
            source.path.starts_with('/') && !source.path.starts_with("//"),
            "目录路径须以 / 开始"
        );
        format!("{}{}", backend.base_url.trim_end_matches('/'), source.path)
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(source.timeout_seconds))
        .build()?;
    let mut request = if let Some(mut body) = source.request_body.clone() {
        for (key, pointer) in &source.body_credentials {
            body[key] = credential
                .as_ref()
                .and_then(|value| value.pointer(pointer))
                .context("目录授权文件缺少请求字段")?
                .clone();
        }
        client.post(url).json(&body)
    } else {
        client.get(url)
    }
    .header("cache-control", "no-cache");
    for (name, value) in &source.headers {
        request = request.header(name, value);
    }
    if let Some(credential) = &credential {
        let token = credential
            .pointer(&source.token_pointer)
            .and_then(Value::as_str)
            .context("目录授权文件缺少访问令牌")?;
        request = request.bearer_auth(token);
    } else if let Some(auth) = &backend.auth {
        if source.auth_headers.is_empty() {
            request = crate::apply_backend_auth(request, auth).await?;
        } else {
            let token = crate::read_backend_token(auth).await?;
            for (name, template) in &source.auth_headers {
                request = request.header(name, template.replace("{token}", token.trim()));
            }
        }
    }
    let mut response = request.send().await.context("模型目录连接失败")?;
    ensure!(
        response.status().is_success(),
        "模型目录返回 HTTP {}",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 8 * 1024 * 1024,
            "模型目录超过 8 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    normalize(
        &serde_json::from_slice::<Value>(&bytes).context("模型目录返回无效 JSON")?,
        source,
    )
}

fn storage(path: &Path, id: &str) -> Result<PathBuf> {
    control::validate_name(id)?;
    Ok(path
        .parent()
        .context("配置路径缺少上级目录")?
        .join("model-catalogs")
        .join(format!("{id}.json")))
}

fn save(path: &Path, snapshot: &Snapshot) -> Result<()> {
    let directory = path.parent().context("目录缓存路径无效")?;
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    control::write_json(path, &serde_json::to_value(snapshot)?)
}

async fn publish(path: &Path, source: &Source, models: &[Model]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let Some(spec) = &source.on_update else {
        return Ok(());
    };
    let context = json!({"config_path":path,"config_directory":path.parent()});
    let mut child = tokio::process::Command::new(control::render(&spec.program, &context)?)
        .args(
            spec.args
                .iter()
                .map(|arg| control::render(arg, &context))
                .collect::<Result<Vec<_>>>()?,
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().context("目录同步输入不可用")?;
    stdin
        .write_all(&serde_json::to_vec(
            &json!({"models":models,"settings":source.update_settings}),
        )?)
        .await?;
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(source.timeout_seconds), child.wait())
        .await
        .context("目录桥接同步超时")??;
    ensure!(status.success(), "目录桥接同步失败，请检查适配程序和配置");
    Ok(())
}

fn result(id: &str, snapshot: &Snapshot) -> Value {
    let previous: BTreeMap<_, _> = snapshot
        .baseline
        .as_ref()
        .into_iter()
        .flatten()
        .map(|row| (&row.id, row))
        .collect();
    let current: BTreeMap<_, _> = snapshot.models.iter().map(|row| (&row.id, row)).collect();
    let added: Vec<_> = current
        .keys()
        .filter(|id| snapshot.baseline.is_some() && !previous.contains_key(*id))
        .collect();
    let removed: Vec<_> = previous
        .keys()
        .filter(|id| !current.contains_key(*id))
        .collect();
    let changed: Vec<_> = current
        .iter()
        .filter(|(id, row)| previous.get(*id).is_some_and(|old| old != *row))
        .map(|(id, _)| id)
        .collect();
    json!({"backend":id,"models":snapshot.models,"updated_at":snapshot.updated_at,"checked_at":snapshot.checked_at,
        "error":snapshot.error,"added":added,"removed":removed,"changed":changed,"monitoring":snapshot.source.enabled,
        "interval_seconds":snapshot.source.interval_seconds})
}

impl Service {
    pub(crate) fn start(path: Arc<PathBuf>, mut shutdown: watch::Receiver<bool>) -> Arc<Self> {
        let service = Arc::new(Self::default());
        let worker = service.clone();
        tokio::spawn(async move {
            loop {
                if *shutdown.borrow() {
                    break;
                }
                if let Ok(value) = control::load_value(&path)
                    && let Ok(manager) = control::management(&value)
                {
                    for (id, source) in &manager.model_catalogs {
                        if !source.enabled {
                            continue;
                        }
                        tokio::select! {
                            _ = shutdown.changed() => return,
                            result = worker.read(&path,id,false,false) => { if let Err(error) = result { eprintln!("模型目录 {id}: {error}"); } }
                        }
                    }
                }
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                }
            }
        });
        service
    }

    async fn read(&self, path: &Path, id: &str, refresh: bool, acknowledge: bool) -> Result<Value> {
        let requested = now();
        let value = control::load_value(path)?;
        let config = crate::load_router_runtime_config(path)?;
        let backend = config
            .runtime
            .backends
            .get(id)
            .context("后端不存在，请先保存并应用后端配置")?;
        let source = control::management(&value)?
            .model_catalogs
            .get(id)
            .cloned()
            .unwrap_or_default();
        let file = storage(path, id)?;
        let entry = {
            let mut entries = self.entries.lock().await;
            entries.retain(|id, _| config.runtime.backends.contains_key(id));
            entries.entry(id.into()).or_default().clone()
        };
        let mut cache = entry.lock().await;
        if cache.is_none() {
            match std::fs::read(&file) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(snapshot) => *cache = Some(snapshot),
                    Err(_) => eprintln!("模型目录 {id} 缓存格式变化，将从服务端重新读取"),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if cache
            .as_ref()
            .is_none_or(|s| s.source != source || s.base_url != backend.base_url)
        {
            *cache = Some(Snapshot {
                source: source.clone(),
                base_url: backend.base_url.clone(),
                ..Default::default()
            });
        }
        let snapshot = cache.as_mut().context("模型目录状态不可用")?;
        if acknowledge {
            snapshot.baseline = Some(snapshot.models.clone());
            save(&file, snapshot)?;
            return Ok(result(id, snapshot));
        }
        let due = snapshot
            .checked_at
            .is_none_or(|at| requested.saturating_sub(at) >= source.interval_seconds);
        let concurrent = snapshot.checked_at.is_some_and(|at| at > requested);
        if due || (refresh && !concurrent) {
            let response = tokio::time::timeout(
                Duration::from_secs(source.timeout_seconds),
                fetch(backend, &source),
            )
            .await;
            snapshot.checked_at = Some(now());
            match response {
                Ok(Ok(models)) => {
                    if models != snapshot.models
                        && let Err(error) = publish(path, &source, &models).await
                    {
                        snapshot.error = Some(error.to_string());
                        return Ok(result(id, snapshot));
                    }
                    if snapshot.baseline.is_none() {
                        snapshot.baseline = Some(models.clone());
                    }
                    snapshot.models = models;
                    snapshot.updated_at = snapshot.checked_at;
                    snapshot.error = None;
                    save(&file, snapshot)?;
                }
                Ok(Err(error)) => snapshot.error = Some(error.to_string()),
                Err(_) => snapshot.error = Some("模型目录查询超时".into()),
            }
        }
        Ok(result(id, snapshot))
    }
}

#[derive(Deserialize)]
struct ReadRequest {
    backend: String,
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    acknowledge: bool,
}

async fn read(
    State(state): State<ControlState>,
    Json(request): Json<ReadRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state
            .catalogs
            .read(
                &state.path,
                &request.backend,
                request.refresh,
                request.acknowledge,
            )
            .await?,
    ))
}

pub(crate) fn routes() -> Router<ControlState> {
    Router::new().route("/catalogs/read", post(read))
}
