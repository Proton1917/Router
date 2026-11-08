use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Deserialize)]
pub struct Selection {
    pub client: String,
    pub provider: String,
    pub base_url: String,
    pub key: String,
    pub model: String,
    pub command: String,
    pub install_dependencies: bool,
    #[serde(default)]
    pub authentication: String,
    #[serde(default)]
    pub accounts: Vec<SubscriptionAccount>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SubscriptionAccount {
    id: String,
    label: String,
    directory: String,
}

pub fn recipes(app: &AppHandle) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        super::resources(app)?.join("onboarding.json"),
    )?)?)
}

pub fn executable(home: &Path, name: &str) -> Option<PathBuf> {
    let mut directories = vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    directories.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    directories.push(PathBuf::from("/usr/bin"));
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|path| {
            if path == Path::new("/usr/bin/python3")
                && !std::process::Command::new("/usr/bin/xcode-select")
                    .arg("--print-path")
                    .output()
                    .is_ok_and(|output| output.status.success())
            {
                return false;
            }
            path.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

pub fn environment(app: &AppHandle, recipes: &Value) -> Result<Value> {
    let home = app.path().home_dir()?;
    let mut programs = BTreeMap::new();
    for client in recipes["clients"]
        .as_object()
        .context("安装配置缺少客户端")?
        .values()
    {
        let program = client["program"].as_str().context("安装配置缺少程序")?;
        programs.insert(program.to_owned(), executable(&home, program));
    }
    for program in ["python3", "brew"] {
        programs.insert(program.to_owned(), executable(&home, program));
    }
    Ok(json!(programs))
}

fn available_listener(preferred: &str) -> Result<TcpListener> {
    let listener = match TcpListener::bind(preferred) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            TcpListener::bind("127.0.0.1:0")?
        }
        Err(error) => return Err(error.into()),
    };
    Ok(listener)
}

pub fn available_addresses(gateway: &str, control: &str) -> Result<(String, String)> {
    let gateway = available_listener(gateway)?;
    let control = available_listener(control)?;
    Ok((
        gateway.local_addr()?.to_string(),
        control.local_addr()?.to_string(),
    ))
}

fn selected<'a>(recipes: &'a Value, selection: &Selection) -> Result<(&'a Value, &'a Value)> {
    let client = recipes["clients"]
        .get(&selection.client)
        .context("请选择客户端")?;
    if selection.authentication == "subscription" {
        ensure!(
            client["subscription_template"].is_string(),
            "请选择支持官方订阅的客户端"
        );
        return Ok((client, client));
    }
    let provider = recipes["providers"]
        .get(&selection.provider)
        .context("请选择服务商")?;
    ensure!(
        provider["protocols"]
            .as_array()
            .context("服务商协议配置错误")?
            .contains(&client["protocol"]),
        "请选择支持此客户端的服务商"
    );
    let url = reqwest::Url::parse(selection.base_url.trim())?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
        "请使用 HTTPS 地址或本机 HTTP 地址"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "API 地址请使用基础路径，凭据填写在 API Key 中"
    );
    ensure!(!selection.key.trim().is_empty(), "请填写 API Key");
    Ok((client, provider))
}

pub async fn models(app: &AppHandle, selection: &Selection) -> Result<Value> {
    let recipes = recipes(app)?;
    let (_, provider) = selected(&recipes, selection)?;
    let base = selection.base_url.trim().trim_end_matches('/');
    let path = provider["catalog_path"]
        .as_str()
        .context("服务商缺少模型目录")?;
    let url = if selection.base_url.trim_end_matches('/')
        == provider["base_url"].as_str().unwrap_or("")
    {
        provider["catalog_url"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{base}{path}"))
    } else {
        format!(
            "{base}{}",
            if base.ends_with("/v1") {
                "/models"
            } else {
                path
            }
        )
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .build()?;
    let mut request = client.get(url);
    for (name, value) in provider["headers"]
        .as_object()
        .context("服务商缺少认证配置")?
    {
        request = request.header(
            name,
            value
                .as_str()
                .context("认证模板格式错误")?
                .replace("{token}", selection.key.trim()),
        );
    }
    let response = request.send().await?;
    ensure!(
        response.status().is_success(),
        "读取模型目录返回 HTTP {}，请检查服务地址、API Key 和账号权限",
        response.status().as_u16()
    );
    let data: Value = response.json().await?;
    let rows = data["data"]
        .as_array()
        .context("模型目录缺少 data 数组；可直接填写服务商提供的模型 ID")?;
    let models: Vec<_> = rows.iter().filter_map(|row| row["id"].as_str().map(|id| json!({"id":id,"name":row["name"].as_str().or(row["display_name"].as_str()).unwrap_or(id)}))).collect();
    Ok(json!({"models":models}))
}

pub async fn dependencies(
    app: &AppHandle,
    selection: &Selection,
) -> Result<BTreeMap<String, PathBuf>> {
    let recipes = recipes(app)?;
    let (client, _) = selected(&recipes, selection)?;
    ensure!(
        selection.authentication == "subscription" || !selection.model.trim().is_empty(),
        "请选择模型或填写模型 ID"
    );
    ensure!(
        !selection.command.is_empty()
            && selection
                .command
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            && selection.command.as_bytes()[0].is_ascii_alphabetic(),
        "命令名以英文字母开头，可包含字母、数字、短横线和下划线"
    );
    if selection.authentication == "subscription" {
        ensure!(!selection.accounts.is_empty(), "请添加主账号");
    }
    let mut identifiers = std::collections::HashSet::new();
    let mut directories = std::collections::HashSet::new();
    for account in &selection.accounts {
        ensure!(
            !account.id.is_empty()
                && account
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                && identifiers.insert(&account.id),
            "账号标识须唯一，并由英文字母、数字、短横线或下划线组成"
        );
        let directory = Path::new(&account.directory);
        ensure!(directory.is_absolute(), "账号配置目录请填写绝对路径");
        let mut parent = directory;
        let mut suffix = Vec::new();
        while !parent.exists() {
            suffix.push(parent.file_name().context("账号目录格式错误")?);
            parent = parent.parent().context("账号目录缺少父目录")?;
        }
        let mut resolved = fs::canonicalize(parent)?;
        for component in suffix.into_iter().rev() {
            if component == ".." {
                resolved.pop();
            } else {
                resolved.push(component);
            }
        }
        ensure!(
            directories.insert(resolved.clone())
                && resolved.ancestors().all(|path| !path.join(".git").exists()),
            "各账号使用独立目录，并放在 Git 工作区之外"
        );
    }
    let home = app.path().home_dir()?;
    let program = client["program"].as_str().context("客户端缺少程序")?;
    let mut found = BTreeMap::new();
    for name in ["python3", program] {
        if let Some(path) = executable(&home, name) {
            found.insert(name.to_owned(), path);
            continue;
        }
        ensure!(
            selection.install_dependencies,
            "需要安装 {name}，请启用自动安装所需程序"
        );
        let brew = executable(&home, "brew")
            .context("请安装 Homebrew 后点击重试；也可自行安装所选客户端与 Python 3")?;
        app.emit("setup-progress", format!("正在安装 {name}…"))?;
        let args: Vec<String> = if name == "python3" {
            vec!["install".into(), "python".into()]
        } else {
            serde_json::from_value(client["install_args"].clone())?
        };
        let output = tokio::time::timeout(
            Duration::from_secs(1200),
            tokio::process::Command::new(brew)
                .args(args)
                .env("HOMEBREW_NO_AUTO_UPDATE", "1")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("安装超时，请检查网络后重试")??;
        ensure!(
            output.status.success(),
            "安装 {name} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        found.insert(
            name.to_owned(),
            executable(&home, name).with_context(|| format!("安装后仍需确认 {name} 的程序位置"))?,
        );
    }
    Ok(found)
}

pub async fn configure(
    app: &AppHandle,
    config: &Path,
    selection: &Selection,
    programs: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let recipes = recipes(app)?;
    let (client, provider) = selected(&recipes, selection)?;
    let directory = config.parent().context("配置目录缺失")?;
    if selection.authentication == "subscription" {
        let mut value: Value = serde_json::from_slice(&fs::read(config)?)?;
        let template_id = client["subscription_template"]
            .as_str()
            .context("订阅启动模板缺失")?;
        let template = &mut value["management"]["templates"][template_id];
        template["program"] = json!(programs["python3"]);
        template["account_adapter"]["program"] = json!(programs["python3"]);
        template["env"]["ROUTER_SUBSCRIPTION_CLIENT"] =
            json!(programs[client["program"].as_str().context("客户端程序缺失")?]);
        value["management"]["commands"][&selection.command] = json!({"template":template_id,"label":"Claude Code","enabled":true,"options":{"auth-mode":"account"},"env":{"ROUTER_SUBSCRIPTION_ACCOUNTS":serde_json::to_string(&selection.accounts)?}});
        return apply_value(config, &value).await;
    }
    let credentials = directory.join("credentials");
    fs::create_dir_all(&credentials)?;
    fs::set_permissions(&credentials, fs::Permissions::from_mode(0o700))?;
    let credential = credentials.join("setup-provider");
    let mut secret = tempfile::NamedTempFile::new_in(&credentials)?;
    use std::io::Write;
    secret.write_all(selection.key.trim().as_bytes())?;
    secret.as_file().sync_all()?;
    secret.persist(&credential).map_err(|e| e.error)?;
    let mut value: Value = serde_json::from_slice(&fs::read(config)?)?;
    let base = selection.base_url.trim().trim_end_matches('/');
    let mut backend =
        json!({"base_url":base,"auth":{"token_file":credential,"headers":provider["headers"]}});
    if base.ends_with("/v1") {
        backend["path_rewrites"] = json!({"/v1/messages":"/messages","/v1/responses":"/responses","/v1/responses/compact":"/responses/compact","/v1/models":"/models"});
    }
    value["runtime"]["backends"]["setup-provider"] = backend;
    value["profiles"]["setup-model"] =
        json!({"standard":{"target":"setup-provider","model":selection.model.trim()}});
    value["management"]["backend_protocols"]["setup-provider"] = provider["protocols"].clone();
    value["management"]["labels"] = json!({"backend:setup-provider":provider["label"]});
    value["management"]["commands"][&selection.command] = json!({"template":selection.client,"label":format!("{} · {}",client["label"].as_str().unwrap_or("API"),provider["label"].as_str().unwrap_or("API")),"enabled":true,"model":selection.model.trim(),"model_bindings":[{"id":"primary","model":selection.model.trim(),"profile":"setup-model","visible":true,"compact_with_self":true,"subagents_with_self":true}]});
    if client["subscription_template"].is_string() {
        value["management"]["commands"][&selection.command]["options"]["auth-mode"] = json!("api");
        value["management"]["commands"][&selection.command]["env"]["ROUTER_SUBSCRIPTION_ACCOUNTS"] =
            json!(serde_json::to_string(&selection.accounts)?);
    }
    for (field, routes) in [
        ("compact_with_self", "compaction_routes"),
        ("subagents_with_self", "subagent_routes"),
    ] {
        let enabled = value["management"]["templates"][&selection.client][routes]
            .as_array()
            .is_some_and(|items| !items.is_empty());
        value["management"]["commands"][&selection.command]["model_bindings"][0][field] =
            json!(enabled);
    }
    let python = programs.get("python3").context("Python 路径缺失")?;
    let program_name = client["program"].as_str().context("客户端程序缺失")?;
    let program = programs.get(program_name).context("客户端程序路径缺失")?;
    for template in value["management"]["templates"]
        .as_object_mut()
        .context("启动模板缺失")?
        .values_mut()
    {
        if template["program"] == "python3" {
            template["program"] = json!(python);
        }
        if template["program"] == program_name {
            template["program"] = json!(program);
        }
        if template["model_adapter"]["program"] == "python3" {
            template["model_adapter"]["program"] = json!(python);
        }
        if template["account_adapter"].is_object() {
            template["account_adapter"]["program"] = json!(python);
            if template["env"]["ROUTER_SUBSCRIPTION_CLIENT"] == program_name {
                template["env"]["ROUTER_SUBSCRIPTION_CLIENT"] = json!(program);
            }
        }
        if let Some(args) = template["args"].as_array_mut() {
            for arg in args {
                if arg == program_name {
                    *arg = json!(program);
                }
            }
        }
    }
    value["management"]["context_discovery"]["adapter"]["program"] = json!(python);
    value["management"]["mods"]["adapter"]["program"] = json!(python);
    let mut catalog = json!({"path":provider["catalog_path"],"headers":provider["headers"]});
    // 目录认证复用后端，固定请求头排除令牌占位符。
    catalog["headers"]
        .as_object_mut()
        .expect("服务商请求头")
        .retain(|_, value| !value.as_str().unwrap_or("").contains("{token}"));
    if base == provider["base_url"].as_str().unwrap_or("") && provider["catalog_url"].is_string() {
        catalog["request_url"] = provider["catalog_url"].clone();
    } else if base.ends_with("/v1") {
        catalog["path"] = json!("/models");
    }
    value["management"]["model_catalogs"] = json!({"setup-provider":catalog});
    apply_value(config, &value).await
}

async fn apply_value(config: &Path, value: &Value) -> Result<()> {
    let directory = config.parent().context("配置目录缺失")?;
    let mut candidate = tempfile::Builder::new()
        .prefix("setup-")
        .suffix(".json")
        .tempfile_in(directory)?;
    serde_json::to_writer_pretty(&mut candidate, value)?;
    candidate.as_file().sync_all()?;
    super::run_router(
        &directory.join("application/bin/router"),
        config,
        &[
            "config",
            "apply",
            "--file",
            candidate.path().to_str().context("配置路径编码错误")?,
        ],
    )
    .await?;
    Ok(())
}
