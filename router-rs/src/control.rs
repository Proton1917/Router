use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

use crate::routing::RouterConfig;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchTemplate {
    pub label: String,
    pub program: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub unset_env: Vec<String>,
    #[serde(default)]
    pub model_flags: Vec<String>,
    #[serde(default)]
    pub header_env: Option<String>,
    #[serde(default)]
    pub deferred: bool,
    #[serde(default)]
    pub deferred_model_env: Option<String>,
    #[serde(default)]
    pub capture_env: Vec<String>,
    #[serde(default)]
    pub api_env: BTreeMap<String, String>,
    #[serde(default)]
    pub api_gate: Option<EnvGate>,
    #[serde(default)]
    pub request_path: Option<String>,
    #[serde(default)]
    pub request_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub route_before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvGate {
    pub name: String,
    pub contains: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub template: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub match_all_models: bool,
    #[serde(default)]
    pub route_before: Option<String>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub unset_env: Vec<String>,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Management {
    pub listen: SocketAddr,
    pub assets_directory: PathBuf,
    pub token_file: PathBuf,
    pub credential_directory: PathBuf,
    pub launcher_directory: PathBuf,
    pub shell_file: PathBuf,
    pub log_directory: PathBuf,
    pub browser: ProcessSpec,
    #[serde(default)]
    pub service_start: Option<ProcessSpec>,
    #[serde(default)]
    pub service_restart: Option<ProcessSpec>,
    pub command_header: String,
    pub max_request_bytes: usize,
    pub templates: BTreeMap<String, LaunchTemplate>,
    pub commands: BTreeMap<String, CommandSpec>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub backend_protocols: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub profile_models: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub generated_routes: Vec<String>,
    #[serde(default)]
    pub generated_cases: Vec<String>,
}

impl Management {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.listen.ip().is_loopback(),
            "管理界面必须监听 loopback 地址"
        );
        ensure!(self.max_request_bytes > 0, "管理请求大小必须大于零");
        ensure!(self.listen.port() > 0, "管理界面需要明确的监听端口");
        self.command_header.parse::<http::HeaderName>()?;
        ensure!(!self.browser.program.is_empty(), "浏览器启动程序不能为空");
        let mut names = BTreeSet::new();
        for (name, command) in &self.commands {
            validate_name(name)?;
            ensure!(
                name != "router",
                "router 是管理程序名称，请为客户端选择其他名称"
            );
            ensure!(
                names.insert(name.to_lowercase()),
                "命令名称存在大小写冲突: {name}"
            );
            let template = self
                .templates
                .get(&command.template)
                .with_context(|| format!("命令 {name} 引用了不存在的启动模板"))?;
            if command.profile.is_some() {
                ensure!(
                    template.request_path.is_some(),
                    "命令 {name} 的启动方式未声明 API 接口路径"
                );
            }
        }
        for (name, template) in &self.templates {
            validate_name(name)?;
            ensure!(
                !template.program.is_empty(),
                "启动模板 {name} 的程序不能为空"
            );
            if let Some(header) = &template.header_env {
                validate_env(header)?;
            }
            for name in template
                .env
                .keys()
                .chain(template.api_env.keys())
                .chain(&template.unset_env)
                .chain(&template.capture_env)
            {
                validate_env(name)?;
            }
            if let Some(name) = &template.deferred_model_env {
                validate_env(name)?;
            }
            if let Some(gate) = &template.api_gate {
                validate_env(&gate.name)?;
            }
        }
        for command in self.commands.values() {
            for name in command.env.keys().chain(&command.unset_env) {
                validate_env(name)?;
            }
        }
        Ok(())
    }
}

fn validate_env(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && !value.as_bytes()[0].is_ascii_digit()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "无效环境变量名称: {value}"
    );
    Ok(())
}

pub fn validate_name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && !value.starts_with('-')
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
        "名称只能包含字母、数字、下划线和连字符，最长 64 字符"
    );
    Ok(())
}

pub fn load_value(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path)?).context("配置文件不是有效的 JSON")
}

pub fn management(value: &Value) -> Result<Management> {
    let result: Management = serde_json::from_value(
        value
            .get("management")
            .context("配置文件缺少 management，请先配置管理入口")?
            .clone(),
    )?;
    result.validate()?;
    Ok(result)
}

pub fn resolve_path(config: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        config.parent().unwrap_or_else(|| Path::new(".")).join(path)
    }
}

pub fn revision(value: &Value) -> Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().context("配置文件目录不存在")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn gateway_url(value: &Value) -> Result<String> {
    let address: SocketAddr = value
        .pointer("/runtime/server/listen")
        .and_then(Value::as_str)
        .context("缺少转发服务地址")?
        .parse()?;
    let address = if address.ip().is_unspecified() {
        SocketAddr::from(([127, 0, 0, 1], address.port()))
    } else {
        address
    };
    Ok(format!("http://{address}"))
}

pub fn render(template: &str, context: &Value) -> Result<String> {
    let mut environment = minijinja::Environment::new();
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    environment
        .render_str(template, context)
        .context("启动模板渲染失败")
}

pub fn command_context(value: &Value, name: &str) -> Result<Value> {
    let manager = management(value)?;
    let command = manager.commands.get(name).context("命令不存在")?;
    let template = &manager.templates[&command.template];
    let mut headers = template.request_headers.clone();
    headers.insert(manager.command_header.clone(), name.to_owned());
    let headers_text = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(
        json!({"config": value, "command_name": name, "command": command, "model": effective_model(value,name)?, "profile": command.profile.as_ref().and_then(|id| value["profiles"].get(id)), "gateway_url": gateway_url(value)?, "command_header": manager.command_header, "headers_text": headers_text}),
    )
}

pub fn effective_model(value: &Value, name: &str) -> Result<Option<String>> {
    let manager = management(value)?;
    let command = manager.commands.get(name).context("命令不存在")?;
    if let Some(model) = &command.model {
        return Ok(Some(model.clone()));
    }
    let Some(profile) = &command.profile else {
        return Ok(None);
    };
    let template = &manager.templates[&command.template];
    let protocol = template
        .request_path
        .as_ref()
        .and_then(|path| value["runtime"]["server"]["endpoints"].get(path))
        .and_then(Value::as_str);
    Ok(protocol
        .and_then(|protocol| {
            manager
                .profile_models
                .get(profile)
                .and_then(|models| models.get(protocol))
                .cloned()
        })
        .or_else(|| {
            value["profiles"][profile]["standard"]["model"]
                .as_str()
                .map(str::to_owned)
        }))
}

pub fn prepare_candidate(current: &Value, mut candidate: Value) -> Result<(Value, Vec<Value>)> {
    let manager = management(&candidate)?;
    let previous = if current.get("management").is_some() {
        management(current)?
    } else {
        let mut previous = manager.clone();
        previous.generated_routes.clear();
        previous.generated_cases.clear();
        previous
    };
    let generated = previous
        .generated_routes
        .iter()
        .chain(&manager.generated_routes)
        .collect::<BTreeSet<_>>();
    let generated_cases = previous
        .generated_cases
        .iter()
        .chain(&manager.generated_cases)
        .collect::<BTreeSet<_>>();
    let routes = candidate
        .get_mut("routes")
        .and_then(Value::as_array_mut)
        .context("routes 必须为数组")?;
    let positions = routes
        .iter()
        .enumerate()
        .filter_map(|(index, route)| route["id"].as_str().map(|id| (id.to_owned(), index)))
        .collect::<BTreeMap<_, _>>();
    routes.retain(|route| {
        route["id"]
            .as_str()
            .is_none_or(|id| !generated.iter().any(|old| old.as_str() == id))
    });
    candidate["validation_cases"]
        .as_array_mut()
        .context("validation_cases 必须为数组")?
        .retain(|case| {
            case["name"]
                .as_str()
                .is_none_or(|id| !generated_cases.iter().any(|old| old.as_str() == id))
        });
    let mut managed_ids = Vec::new();
    for (name, command) in &manager.commands {
        let Some(profile) = &command.profile else {
            continue;
        };
        ensure!(
            candidate["profiles"].get(profile).is_some(),
            "命令 {name} 的模型配置不存在: {profile}"
        );
        let template = &manager.templates[&command.template];
        let model =
            effective_model(&candidate, name)?.context("绑定模型路由时需要配置启动模型名称")?;
        let target_id = candidate["profiles"][profile]["standard"]["target"]
            .as_str()
            .context("模型后端不存在")?;
        if let Some(protocol) = template
            .request_path
            .as_ref()
            .and_then(|path| candidate["runtime"]["server"]["endpoints"].get(path))
            .and_then(Value::as_str)
            && let Some(supported) = manager.backend_protocols.get(target_id)
        {
            ensure!(
                supported.iter().any(|p| p == protocol),
                "后端 {target_id} 未声明支持 {protocol}，请核对后端协议或选择其他模型"
            );
        }
        let id = format!("command-{name}");
        let mut matcher = json!({"headers": [{"name": manager.command_header, "values": [name]}]});
        if !command.match_all_models {
            matcher["models"] = json!([model]);
        }
        let route = json!({"id": id, "match": matcher, "profile": profile});
        let routes = candidate["routes"].as_array_mut().expect("routes array");
        ensure!(
            !routes.iter().any(|r| r["id"].as_str() == Some(&id)),
            "路由名称与现有规则冲突: {id}"
        );
        let before = command
            .route_before
            .as_ref()
            .or(template.route_before.as_ref());
        let index = if let Some(before) = before {
            routes
                .iter()
                .position(|route| route["id"].as_str() == Some(before))
                .with_context(|| format!("路由插入位置不存在: {before}"))?
        } else {
            positions
                .get(&id)
                .copied()
                .unwrap_or(0)
                .min(routes.len().saturating_sub(1))
        };
        routes.insert(index, route);
        let context = command_context(&candidate, name)?;
        let mut headers = HeaderMap::new();
        let mut case_headers = BTreeMap::new();
        for (key, value) in &template.request_headers {
            let rendered = render(value, &context)?;
            headers.insert(key.parse::<http::HeaderName>()?, rendered.parse()?);
            case_headers.insert(key.clone(), rendered);
        }
        headers.insert(
            manager.command_header.parse::<http::HeaderName>()?,
            name.parse()?,
        );
        case_headers.insert(manager.command_header.clone(), name.clone());
        let runtime: crate::runtime::RuntimeConfig =
            serde_json::from_value(candidate["runtime"].clone())?;
        let client = runtime.client_for(
            &headers,
            template.request_path.as_deref().context("缺少接口路径")?,
        )?;
        let target = candidate["profiles"][profile]["standard"]["target"].clone();
        candidate["validation_cases"].as_array_mut().expect("cases array").push(json!({"name": id, "context": client.id, "model": model, "headers": case_headers, "expected": {"rule_id": id, "target": target, "model": model}}));
        managed_ids.push(id);
    }
    candidate["management"]["generated_routes"] = json!(managed_ids);
    candidate["management"]["generated_cases"] = json!(managed_ids);
    let strip = candidate["runtime"]["strip_request_headers"]
        .as_array_mut()
        .context("缺少 strip_request_headers")?;
    if !strip
        .iter()
        .any(|v| v.as_str() == Some(&manager.command_header))
    {
        strip.push(Value::String(manager.command_header.clone()));
    }
    let parsed: RouterConfig = serde_json::from_value(candidate.clone())?;
    let mut changes = Vec::new();
    for (index, case) in parsed.validation_cases.iter().enumerate() {
        let mut headers = HeaderMap::new();
        for (key, value) in &case.headers {
            headers.insert(key.parse::<http::HeaderName>()?, value.parse()?);
        }
        let mut request = json!({"model":case.model,"messages":[{"role":"user","content":case.last_user_content.as_deref().unwrap_or("")}]});
        if let Some(speed) = &case.speed {
            request["speed"] = Value::String(speed.clone());
        }
        let resolved = parsed.resolve(
            &case.context,
            &headers,
            &request,
            case.settings_fast_mode,
            case.settings_model.as_deref(),
        )?;
        if managed_ids.contains(&case.name) {
            ensure!(
                resolved.rule_id == case.name,
                "命令路由 {} 被 {} 覆盖，请调整插入位置",
                case.name,
                resolved.rule_id
            );
        }
        let mut expected =
            json!({"rule_id":resolved.rule_id,"target":resolved.target,"model":resolved.model});
        if let Some(provider) = resolved.provider {
            expected["provider"] = Value::String(provider);
        }
        let previous_expected = current["validation_cases"]
            .as_array()
            .and_then(|cases| {
                cases
                    .iter()
                    .find(|item| item["name"].as_str() == Some(&case.name))
            })
            .map(|item| item["expected"].clone())
            .unwrap_or(Value::Null);
        if previous_expected != expected {
            changes.push(json!({"name":case.name,"before":previous_expected,"after":expected}));
        }
        candidate["validation_cases"][index]["expected"] = expected;
    }
    RouterConfig::from_json(&serde_json::to_vec(&candidate)?)?;
    manager.validate()?;
    Ok((candidate, changes))
}

pub fn save_candidate(path: &Path, expected_revision: &str, candidate: Value) -> Result<Value> {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let current = load_value(path)?;
    ensure!(
        revision(&current)? == expected_revision,
        "配置已被其他窗口或程序修改，请刷新后重新应用"
    );
    let (candidate, changes) = prepare_candidate(&current, candidate)?;
    write_json(path, &candidate)?;
    Ok(json!({"revision":revision(&candidate)?,"config":candidate,"validation_changes":changes}))
}

pub fn save_credential(path: &Path, id: &str, secret: &str) -> Result<PathBuf> {
    validate_name(id)?;
    let secret = secret.trim();
    ensure!(
        !secret.trim().is_empty() && !secret.contains(['\n', '\r']),
        "凭据不能为空或包含换行"
    );
    let manager = management(&load_value(path)?)?;
    let directory = resolve_path(path, &manager.credential_directory);
    fs::create_dir_all(&directory)?;
    let directory = fs::canonicalize(directory)?;
    for ancestor in directory.ancestors() {
        ensure!(
            !ancestor.join(".git").exists(),
            "凭据目录必须位于 Git 工作区之外"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    let destination = directory.join(id);
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(secret.trim().as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&destination)
        .map_err(|error| error.error)?;
    Ok(destination)
}

fn token(path: &Path, manager: &Management) -> Result<String> {
    let token_path = resolve_path(path, &manager.token_file);
    if token_path.exists() {
        let value = fs::read_to_string(token_path)?.trim().to_owned();
        ensure!(
            value.len() >= 32 && !value.contains(char::is_whitespace),
            "管理令牌必须至少 32 字符且不能包含空白"
        );
        return Ok(value);
    }
    fs::create_dir_all(token_path.parent().context("令牌目录不存在")?)?;
    let value = rand::random::<[u8; 32]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut temporary =
        tempfile::NamedTempFile::new_in(token_path.parent().expect("token parent"))?;
    temporary.write_all(value.as_bytes())?;
    temporary
        .persist_noclobber(token_path)
        .map_err(|error| error.error)?;
    Ok(value)
}

#[derive(Clone)]
struct ControlState {
    path: Arc<PathBuf>,
    token: Arc<String>,
    authority: String,
    lock: Arc<Mutex<()>>,
    shutdown: Arc<tokio::sync::Notify>,
}

struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(error: E) -> Self {
        Self(error.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{:#}",self.0)})),
        )
            .into_response()
    }
}

async fn protect(State(state): State<ControlState>, request: Request, next: Next) -> Response {
    let host = request.headers().get("host").and_then(|v| v.to_str().ok());
    if host != Some(&state.authority) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(origin) = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        && origin != format!("http://{}", state.authority)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let actual = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if !actual.is_some_and(|v| blake3::hash(v.as_bytes()) == blake3::hash(state.token.as_bytes())) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", http::HeaderValue::from_static("no-store"));
    response
}

async fn state(State(state): State<ControlState>) -> Result<Json<Value>, ApiError> {
    let value = load_value(&state.path)?;
    let server = crate::load_router_runtime_config(&state.path)?
        .runtime
        .server
        .context("缺少 server")?;
    let health = format!("{}{}", gateway_url(&value)?, server.health_path);
    let online = reqwest::Client::new()
        .get(health)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success());
    Ok(Json(
        json!({"config":value,"revision":revision(&value)?,"status":{"gateway_online":online,"gateway_url":gateway_url(&value)?,"config_path":state.path.as_ref(),"version":env!("CARGO_PKG_VERSION")}}),
    ))
}

#[derive(Deserialize)]
struct ConfigWrite {
    revision: String,
    config: Value,
}

async fn preview(
    State(state): State<ControlState>,
    Json(request): Json<ConfigWrite>,
) -> Result<Json<Value>, ApiError> {
    let _guard = state.lock.lock().await;
    let current = load_value(&state.path)?;
    if revision(&current)? != request.revision {
        return Err(anyhow::anyhow!("配置已变更，请刷新后重新应用").into());
    }
    let (config, changes) = prepare_candidate(&current, request.config)?;
    Ok(Json(json!({"config":config,"validation_changes":changes})))
}

async fn apply(
    State(state): State<ControlState>,
    Json(request): Json<ConfigWrite>,
) -> Result<Json<Value>, ApiError> {
    let _guard = state.lock.lock().await;
    Ok(Json(save_candidate(
        &state.path,
        &request.revision,
        request.config,
    )?))
}

#[derive(Deserialize)]
struct CredentialWrite {
    id: String,
    value: String,
}

async fn credential(
    State(state): State<ControlState>,
    Json(request): Json<CredentialWrite>,
) -> Result<Json<Value>, ApiError> {
    let _guard = state.lock.lock().await;
    let file = save_credential(&state.path, &request.id, &request.value)?;
    Ok(Json(json!({"path":file,"saved":true})))
}

#[derive(Deserialize)]
struct InstallRequest {
    name: Option<String>,
}

async fn install(
    State(state): State<ControlState>,
    Json(request): Json<InstallRequest>,
) -> Result<Json<Value>, ApiError> {
    let _guard = state.lock.lock().await;
    Ok(Json(crate::cli::install_commands(
        &state.path,
        request.name.as_deref(),
    )?))
}

async fn restart(State(state): State<ControlState>) -> Result<Json<Value>, ApiError> {
    crate::cli::restart_gateway(&state.path).await?;
    Ok(Json(json!({"restarted":true})))
}

async fn stop_control(State(state): State<ControlState>) -> Json<Value> {
    state.shutdown.notify_one();
    Json(json!({"stopping":true}))
}

#[derive(Deserialize)]
struct LaunchPreview {
    name: String,
    #[serde(default)]
    args: Vec<String>,
}

async fn launch_preview(
    State(state): State<ControlState>,
    Json(request): Json<LaunchPreview>,
) -> Result<Json<Value>, ApiError> {
    let path = state.path.clone();
    Ok(Json(
        tokio::task::spawn_blocking(move || {
            crate::cli::launch_preview(&path, &request.name, &request.args)
        })
        .await
        .map_err(anyhow::Error::from)??,
    ))
}

#[derive(Deserialize)]
struct BackendProbe {
    id: String,
    path: String,
}

async fn backend_probe(
    State(state): State<ControlState>,
    Json(request): Json<BackendProbe>,
) -> Result<Json<Value>, ApiError> {
    let config = crate::load_router_runtime_config(&state.path)?;
    let backend = config
        .runtime
        .backends
        .get(&request.id)
        .context("后端不存在")?;
    if !request.path.starts_with('/')
        || request.path.starts_with("//")
        || request.path.contains(['?', '#'])
    {
        return Err(anyhow::anyhow!("请填写以 / 开始的 API 路径").into());
    }
    let server = config.runtime.server.as_ref().context("缺少 server")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(server.connect_timeout_ms))
        .build()?;
    let mut builder = client.get(format!(
        "{}{}",
        backend.base_url.trim_end_matches('/'),
        request.path
    ));
    if let Some(auth) = &backend.auth {
        builder = crate::apply_backend_auth(builder, auth).await?;
    }
    let response = builder.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Ok(Json(
            json!({"status":status.as_u16(),"models":[],"message":"上游返回此 HTTP 状态，请核对 API 路径和授权"}),
        ));
    }
    let data: Value = response.json().await?;
    let models = data
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Json(json!({"status":status.as_u16(),"models":models})))
}

pub async fn serve(path: PathBuf) -> Result<()> {
    let value = load_value(&path)?;
    let manager = management(&value)?;
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let state_value = ControlState {
        path: Arc::new(path.clone()),
        token: Arc::new(token(&path, &manager)?),
        authority: manager.listen.to_string(),
        lock: Arc::new(Mutex::new(())),
        shutdown: shutdown.clone(),
    };
    let api = Router::new()
        .route("/state", get(state))
        .route("/preview", post(preview))
        .route("/apply", post(apply))
        .route("/credentials", post(credential))
        .route("/commands/install", post(install))
        .route("/commands/preview", post(launch_preview))
        .route("/backends/probe", post(backend_probe))
        .route("/service/restart", post(restart))
        .route("/stop", post(stop_control))
        .route(
            "/health",
            get(|| async { Json(json!({"service":"router-control"})) }),
        )
        .route_layer(middleware::from_fn_with_state(state_value.clone(), protect))
        .with_state(state_value);
    let assets = resolve_path(&path, &manager.assets_directory);
    ensure!(
        assets.join("index.html").is_file(),
        "前端资源不存在: {}",
        assets.display()
    );
    let app = Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new(assets))
        .layer(DefaultBodyLimit::max(manager.max_request_bytes));
    let listener = tokio::net::TcpListener::bind(manager.listen).await?;
    write_json(
        &path.with_extension("web-state.json"),
        &json!({"listen":manager.listen,"token_file":resolve_path(&path,&manager.token_file),"pid":std::process::id()}),
    )?;
    eprintln!("router web listening on http://{}", manager.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! { _ = crate::shutdown_signal() => {}, _ = shutdown.notified() => {} }
        })
        .await?;
    Ok(())
}

pub fn browser_url(path: &Path) -> Result<String> {
    let manager = management(&load_value(path)?)?;
    Ok(format!(
        "http://{}/#token={}",
        manager.listen,
        token(path, &manager)?
    ))
}

pub fn control_token(path: &Path) -> Result<String> {
    token(path, &management(&load_value(path)?)?)
}
