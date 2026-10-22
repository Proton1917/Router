use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use futures_util::future::BoxFuture;
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    DeviceAuthorizationUrl, EndpointNotSet, EndpointSet, PkceCodeChallenge, RedirectUrl,
    RefreshToken, RequestTokenError, Scope, StandardDeviceAuthorizationResponse, TokenResponse,
    TokenUrl,
    basic::{BasicClient, BasicTokenResponse},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, oneshot};

use crate::control::{self, ApiError, ControlState};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub provider: String,
    pub flow: Flow,
    pub client_id: String,
    pub token_url: String,
    #[serde(default)]
    pub authorization_url: Option<String>,
    #[serde(default)]
    pub device_authorization_url: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub credential_file: String,
    #[serde(default)]
    pub client_secret_file: Option<String>,
    #[serde(default)]
    pub client_auth_basic: bool,
    #[serde(default)]
    pub authorization_params: BTreeMap<String, String>,
    #[serde(default)]
    pub token_params: BTreeMap<String, String>,
    #[serde(default = "login_timeout")]
    pub login_timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    DeviceCode,
    AuthorizationCode,
}

fn login_timeout() -> u64 {
    600
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs()
}

fn endpoint(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).context("OAuth 地址格式错误")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "OAuth 服务端地址必须使用 HTTPS，且不包含用户名、密码或片段"
    );
    Ok(url)
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.provider.trim().is_empty() && !self.client_id.trim().is_empty(),
            "请填写服务商名称和 Client ID"
        );
        endpoint(&self.token_url)?;
        ensure!(
            !self.credential_file.trim().is_empty(),
            "OAuth 凭据文件路径不能为空"
        );
        ensure!(
            (60..=1800).contains(&self.login_timeout_seconds),
            "授权等待时间须为 60～1800 秒"
        );
        match self.flow {
            Flow::DeviceCode => {
                endpoint(
                    self.device_authorization_url
                        .as_deref()
                        .context("缺少设备码授权地址")?,
                )?;
            }
            Flow::AuthorizationCode => {
                endpoint(
                    self.authorization_url
                        .as_deref()
                        .context("缺少浏览器授权地址")?,
                )?;
                self.redirect()?;
            }
        }
        for key in self
            .authorization_params
            .keys()
            .chain(self.token_params.keys())
        {
            ensure!(
                ![
                    "client_id",
                    "client_secret",
                    "grant_type",
                    "code",
                    "state",
                    "scope",
                    "redirect_uri",
                    "code_challenge",
                    "code_challenge_method",
                    "code_verifier",
                    "refresh_token",
                    "device_code",
                    "response_type",
                    "access_token"
                ]
                .contains(&key.as_str()),
                "OAuth 附加参数不能覆盖协议字段 {key}"
            );
        }
        Ok(())
    }

    fn redirect(&self) -> Result<reqwest::Url> {
        let url = reqwest::Url::parse(self.redirect_uri.as_deref().context("缺少本机回调地址")?)?;
        ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "回调地址须为 http://127.0.0.1:端口/路径；端口 0 自动分配"
        );
        Ok(url)
    }

    fn client(
        &self,
    ) -> Result<
        BasicClient<EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>,
    > {
        let mut client = BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_token_uri(TokenUrl::new(self.token_url.clone())?)
            .set_auth_type(if self.client_auth_basic {
                AuthType::BasicAuth
            } else {
                AuthType::RequestBody
            });
        if let Some(path) = &self.client_secret_file {
            let secret = fs::read_to_string(path).context("无法读取 OAuth Client Secret 文件")?;
            ensure!(!secret.trim().is_empty(), "OAuth Client Secret 不能为空");
            client = client.set_client_secret(ClientSecret::new(secret.trim().to_owned()));
        }
        Ok(client)
    }
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("Router/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

// 只向界面返回协议错误代码，避免服务器响应正文包含令牌。
fn exchange_error<RE, T>(error: RequestTokenError<RE, T>) -> anyhow::Error
where
    RE: std::error::Error + 'static,
    T: oauth2::ErrorResponse + std::fmt::Display,
{
    match error {
        RequestTokenError::ServerResponse(error) => {
            let value = serde_json::to_value(error).unwrap_or(Value::Null);
            let message = match value["error"].as_str() {
                Some("invalid_grant") => "授权已过期或被撤销，请重新登录",
                Some("access_denied") => "用户取消或服务商拒绝了授权",
                Some("expired_token") => "设备码已过期，请重新登录",
                Some("invalid_client") => "Client ID 或 Client Secret 无效",
                Some("invalid_scope") => "服务商不支持所填写的权限范围",
                Some("unauthorized_client") => "此 Client ID 未获准使用所选授权方式",
                _ => "OAuth 服务拒绝授权或刷新，请核对参数并重新登录",
            };
            anyhow::anyhow!(message)
        }
        RequestTokenError::Request(_) => {
            anyhow::anyhow!("OAuth 服务连接失败，请检查网络后重新操作")
        }
        _ => anyhow::anyhow!("OAuth 服务返回无效响应，请检查令牌地址和授权参数"),
    }
}

#[derive(Serialize, Deserialize)]
struct Credential {
    token_url: String,
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
    scopes: Vec<String>,
    updated_at: u64,
}

fn credential_directory(settings: &Settings) -> Result<PathBuf> {
    let path = Path::new(&settings.credential_file);
    ensure!(path.is_absolute(), "OAuth 凭据必须使用绝对路径");
    let parent = path.parent().context("凭据目录不存在")?;
    fs::create_dir_all(parent)?;
    let parent = fs::canonicalize(parent)?;
    ensure!(
        !parent.ancestors().any(|p| p.join(".git").exists()),
        "OAuth 凭据必须保存在 Git 工作区之外"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))?;
    }
    Ok(parent)
}

async fn credential_lock(settings: &Settings) -> Result<File> {
    let directory = credential_directory(settings)?;
    let name = Path::new(&settings.credential_file)
        .file_name()
        .context("缺少凭据文件名")?
        .to_string_lossy();
    let path = directory.join(format!(".{name}.lock"));
    tokio::task::spawn_blocking(move || {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        // 多个 Router 适配进程共享刷新锁，避免轮换刷新令牌时互相覆盖。
        for _ in 0..400 {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(error) => return Err(error.into()),
            }
        }
        bail!("等待 OAuth 凭据刷新锁超时")
    })
    .await?
}

fn read_credential(settings: &Settings) -> Result<Credential> {
    let bytes =
        fs::read(&settings.credential_file).context("尚未登录，请在 Router 的 OAuth 设置中登录")?;
    let credential: Credential =
        serde_json::from_slice(&bytes).context("OAuth 凭据文件格式错误")?;
    ensure!(
        credential.token_url == settings.token_url && credential.client_id == settings.client_id,
        "服务商授权参数已改变，请重新登录"
    );
    Ok(credential)
}

fn save_token(
    settings: &Settings,
    token: BasicTokenResponse,
    previous_refresh: Option<String>,
) -> Result<Credential> {
    ensure!(
        !token.access_token().secret().is_empty(),
        "OAuth 返回空访问令牌"
    );
    let credential = Credential {
        token_url: settings.token_url.clone(),
        client_id: settings.client_id.clone(),
        access_token: token.access_token().secret().clone(),
        refresh_token: token
            .refresh_token()
            .map(|t| t.secret().clone())
            .or(previous_refresh),
        expires_at: token
            .expires_in()
            .map(|d| now().saturating_add(d.as_secs())),
        scopes: token
            .scopes()
            .map(|s| s.iter().map(|s| s.to_string()).collect())
            .unwrap_or_else(|| settings.scopes.clone()),
        updated_at: now(),
    };
    let directory = credential_directory(settings)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut temporary, &credential)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&settings.credential_file)
        .map_err(|e| e.error)?;
    Ok(credential)
}

pub async fn access_token(settings: &Settings) -> Result<String> {
    let credential = read_credential(settings)?;
    if credential
        .expires_at
        .is_none_or(|expiry| expiry > now() + 60)
    {
        return Ok(credential.access_token);
    }
    let _lock = credential_lock(settings).await?;
    let credential = read_credential(settings)?;
    if credential
        .expires_at
        .is_none_or(|expiry| expiry > now() + 60)
    {
        return Ok(credential.access_token);
    }
    refresh(settings, credential).await
}

async fn refresh(settings: &Settings, credential: Credential) -> Result<String> {
    let refresh = credential
        .refresh_token
        .context("此授权没有刷新令牌，请重新登录")?;
    let client = settings.client()?;
    let refresh_token = RefreshToken::new(refresh.clone());
    let mut request = client.exchange_refresh_token(&refresh_token);
    for (key, value) in &settings.token_params {
        request = request.add_extra_param(key, value);
    }
    let token = request
        .request_async(&http_client()?)
        .await
        .map_err(exchange_error)?;
    Ok(save_token(settings, token, Some(refresh))?.access_token)
}

struct Session {
    status: Value,
    task: tokio::task::AbortHandle,
    started_at: u64,
}

fn sessions() -> &'static Mutex<BTreeMap<String, Session>> {
    static SESSIONS: OnceLock<Mutex<BTreeMap<String, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(Mutex::default)
}

type PendingToken = BoxFuture<'static, Result<BasicTokenResponse>>;

async fn prepare(settings: &Settings) -> Result<(Value, PendingToken)> {
    let http = http_client()?;
    match settings.flow {
        Flow::DeviceCode => {
            let client =
                settings
                    .client()?
                    .set_device_authorization_url(DeviceAuthorizationUrl::new(
                        settings
                            .device_authorization_url
                            .clone()
                            .context("缺少设备码地址")?,
                    )?);
            let mut request = client.exchange_device_code();
            for scope in &settings.scopes {
                request = request.add_scope(Scope::new(scope.clone()));
            }
            for (key, value) in &settings.authorization_params {
                request = request.add_extra_param(key, value);
            }
            let details: StandardDeviceAuthorizationResponse =
                request.request_async(&http).await.map_err(exchange_error)?;
            let url = details
                .verification_uri_complete()
                .map(|u| u.secret().clone())
                .unwrap_or_else(|| details.verification_uri().to_string());
            endpoint(&url)?;
            let status = json!({"state":"pending", "url":url, "user_code":details.user_code().secret(), "expires_at":now()+details.expires_in().as_secs().min(settings.login_timeout_seconds)});
            let params = settings.token_params.clone();
            Ok((
                status,
                Box::pin(async move {
                    let mut request = client.exchange_device_access_token(&details);
                    for (key, value) in &params {
                        request = request.add_extra_param(key, value);
                    }
                    request
                        .request_async(&http, tokio::time::sleep, None)
                        .await
                        .map_err(exchange_error)
                }),
            ))
        }
        Flow::AuthorizationCode => {
            let mut redirect = settings.redirect()?;
            let listener = tokio::net::TcpListener::bind((
                std::net::Ipv4Addr::LOCALHOST,
                redirect.port().context("缺少回调端口")?,
            ))
            .await
            .context("OAuth 回调端口被占用，请修改端口")?;
            redirect
                .set_port(Some(listener.local_addr()?.port()))
                .map_err(|_| anyhow::anyhow!("回调端口无效"))?;
            let client = settings
                .client()?
                .set_auth_uri(AuthUrl::new(
                    settings.authorization_url.clone().context("缺少授权地址")?,
                )?)
                .set_redirect_uri(RedirectUrl::new(redirect.to_string())?);
            let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
            let mut request = client
                .authorize_url(CsrfToken::new_random)
                .set_pkce_challenge(challenge);
            for scope in &settings.scopes {
                request = request.add_scope(Scope::new(scope.clone()));
            }
            for (key, value) in &settings.authorization_params {
                request = request.add_extra_param(key, value);
            }
            let (url, state) = request.url();
            let (sender, receiver) = oneshot::channel::<Result<String>>();
            let sender = Arc::new(Mutex::new(Some(sender)));
            let callback = Router::new().route(
                redirect.path(),
                get(move |Query(query): Query<BTreeMap<String, String>>| {
                    let sender = sender.clone();
                    let state = state.secret().clone();
                    async move {
                        if query.get("state") != Some(&state) {
                            return (
                                StatusCode::BAD_REQUEST,
                                "授权状态校验失败，请返回 Router 重新登录。",
                            );
                        }
                        let result = if query.contains_key("error") {
                            Err(anyhow::anyhow!("授权已取消或被服务商拒绝"))
                        } else {
                            query
                                .get("code")
                                .filter(|c| !c.is_empty())
                                .cloned()
                                .context("授权回调缺少 code")
                        };
                        let success = result.is_ok();
                        if let Some(sender) = sender.lock().await.take() {
                            let _ = sender.send(result);
                        }
                        (
                            StatusCode::OK,
                            if success {
                                "授权已接收，请返回 Router 查看登录结果。"
                            } else {
                                "授权未完成，请返回 Router 查看详情。"
                            },
                        )
                    }
                }),
            );
            let status = json!({"state":"pending", "url":url.as_str(), "redirect_uri":redirect.as_str(), "expires_at":now()+settings.login_timeout_seconds});
            let params = settings.token_params.clone();
            Ok((
                status,
                Box::pin(async move {
                    let received = Arc::new(Mutex::new(None));
                    let result = received.clone();
                    axum::serve(listener, callback)
                        .with_graceful_shutdown(async move {
                            *result.lock().await =
                                Some(receiver.await.context("OAuth 回调已中断").and_then(|r| r));
                        })
                        .await?;
                    let code = received
                        .lock()
                        .await
                        .take()
                        .context("OAuth 回调服务已结束")??;
                    let mut request = client
                        .exchange_code(AuthorizationCode::new(code))
                        .set_pkce_verifier(verifier);
                    for (key, value) in &params {
                        request = request.add_extra_param(key, value);
                    }
                    request.request_async(&http).await.map_err(exchange_error)
                }),
            ))
        }
    }
}

#[derive(Deserialize)]
struct Action {
    id: String,
    action: String,
}

async fn action(
    State(state): State<ControlState>,
    Json(request): Json<Action>,
) -> Result<Json<Value>, ApiError> {
    let config = crate::load_router_runtime_config(&state.path)?;
    let settings = config
        .runtime
        .backends
        .get(&request.id)
        .and_then(|b| b.auth.as_ref())
        .and_then(|a| a.oauth.clone())
        .context("请先保存并应用此后端的 OAuth 配置")?;
    let key = settings.credential_file.clone();
    match request.action.as_str() {
        "status" => {
            let sessions = sessions().lock().await;
            let pending = sessions.get(&key).map(|s| s.status.clone());
            let credential = match read_credential(&settings) {
                Ok(c) => {
                    json!({"authenticated":c.expires_at.is_none_or(|e| e > now()), "can_refresh":c.refresh_token.is_some(), "expires_at":c.expires_at,"updated_at":c.updated_at,"scopes":c.scopes})
                }
                Err(_) => json!({"authenticated":false}),
            };
            Ok(Json(
                json!({"provider":settings.provider,"credential":credential,"session":pending}),
            ))
        }
        "login" => {
            credential_directory(&settings)?;
            let mut guard = sessions().lock().await;
            guard.retain(|_, session| {
                !session.task.is_finished() || now().saturating_sub(session.started_at) < 3600
            });
            if let Some(session) = guard.get(&key)
                && !session.task.is_finished()
            {
                return Err(anyhow::anyhow!("此账号已有登录流程，请完成或取消后重新登录").into());
            }
            let (status, pending) = prepare(&settings).await?;
            let target = key.clone();
            let task = tokio::spawn(async move {
                let result = tokio::time::timeout(
                    Duration::from_secs(settings.login_timeout_seconds),
                    async {
                        let token = pending.await?;
                        let _lock = credential_lock(&settings).await?;
                        save_token(&settings, token, None)?;
                        Ok::<_, anyhow::Error>(())
                    },
                )
                .await;
                let status = match result {
                    Ok(Ok(())) => json!({"state":"success"}),
                    Ok(Err(error)) => json!({"state":"error", "message":error.to_string()}),
                    Err(_) => {
                        json!({"state":"expired", "message":"授权等待时间已结束，请重新登录"})
                    }
                };
                if let Some(session) = sessions().lock().await.get_mut(&target) {
                    session.status = status;
                }
            });
            guard.insert(
                key,
                Session {
                    status: status.clone(),
                    task: task.abort_handle(),
                    started_at: now(),
                },
            );
            drop(guard);
            let manager = control::management(&control::load_value(&state.path)?)?;
            let context = json!({"url":status["url"]});
            let mut browser =
                tokio::process::Command::new(control::render(&manager.browser.program, &context)?);
            for arg in &manager.browser.args {
                browser.arg(control::render(arg, &context)?);
            }
            let opened = browser
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .is_ok_and(|s| s.success());
            Ok(Json(json!({"session":status,"browser_opened":opened})))
        }
        "cancel" | "logout" => {
            if let Some(session) = sessions().lock().await.remove(&key) {
                session.task.abort();
            }
            if request.action == "logout" {
                let _lock = credential_lock(&settings).await?;
                match fs::remove_file(&settings.credential_file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(Json(json!({"ok":true})))
        }
        "refresh" => {
            let _lock = credential_lock(&settings).await?;
            refresh(&settings, read_credential(&settings)?).await?;
            Ok(Json(json!({"ok":true})))
        }
        _ => Err(anyhow::anyhow!("未知 OAuth 操作").into()),
    }
}

pub fn routes() -> Router<ControlState> {
    Router::new().route("/oauth/action", post(action))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        serde_json::from_str(include_str!("../examples/oauth.json")).unwrap()
    }

    #[test]
    fn oauth_rejects_protocol_overrides_and_external_callbacks() {
        let mut settings = settings();
        settings.validate().unwrap();
        settings
            .token_params
            .insert("refresh_token".into(), String::new());
        assert!(settings.validate().is_err());
        settings.token_params.clear();
        settings.flow = Flow::AuthorizationCode;
        settings.redirect_uri = Some("http://0.0.0.0:49152/oauth/callback".into());
        assert!(settings.validate().is_err());
    }

    #[tokio::test]
    async fn pkce_callback_checks_state_and_releases_listener_after_denial() {
        let mut settings = settings();
        settings.flow = Flow::AuthorizationCode;
        let (status, pending) = prepare(&settings).await.unwrap();
        let authorization = reqwest::Url::parse(status["url"].as_str().unwrap()).unwrap();
        let params: BTreeMap<_, _> = authorization.query_pairs().into_owned().collect();
        assert_eq!(params["code_challenge_method"], "S256");
        assert!(!params.contains_key("code_verifier"));
        let callback = reqwest::Url::parse(status["redirect_uri"].as_str().unwrap()).unwrap();
        let pending = tokio::spawn(pending);
        let client = http_client().unwrap();
        let response = client
            .get(callback.clone())
            .query(&[("state", "invalid")])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = client
            .get(callback.clone())
            .query(&[
                ("state", params["state"].as_str()),
                ("error", "access_denied"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.text().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().to_string().contains("拒绝"));
        tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, callback.port().unwrap()))
            .await
            .unwrap();
    }
}
