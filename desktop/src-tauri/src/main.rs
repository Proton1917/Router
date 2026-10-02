use std::{
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
};

#[derive(Parser)]
struct Arguments {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    setup: bool,
}

#[derive(Default)]
struct DesktopState {
    error: Mutex<Option<String>>,
    operation: tokio::sync::Mutex<()>,
}

#[derive(Serialize, Deserialize)]
struct Preferences {
    config: PathBuf,
}

fn resources(app: &AppHandle) -> Result<PathBuf> {
    Ok(app.path().resource_dir()?.join("resources"))
}

fn preferences_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(app.path().app_config_dir()?.join("preferences.json"))
}

fn saved_config(app: &AppHandle) -> Result<Option<PathBuf>> {
    let path = preferences_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let preferences: Preferences = serde_json::from_slice(&fs::read(path)?)?;
    Ok(Some(preferences.config))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("文件没有父目录")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

async fn run_router(binary: &Path, config: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(binary)
            .arg("--config")
            .arg(config)
            .args(args)
            .env_remove("ROUTER_ADDR")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("router 操作超时，请检查配置中的服务命令与日志")??;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

fn setup_guard(window: &WebviewWindow) -> Result<()> {
    ensure!(
        window.label() == "setup" && window.url()?.scheme() == "tauri",
        "此窗口不能执行桌面安装操作"
    );
    Ok(())
}

fn window_config(
    app: &AppHandle,
    path: Option<&Path>,
    label: &str,
    url: WebviewUrl,
) -> Result<tauri::utils::config::WindowConfig> {
    let configured = path
        .and_then(Path::parent)
        .map(|directory| directory.join("desktop.json"));
    let source = match configured {
        Some(path) if path.exists() => path,
        _ => resources(app)?.join("appearance.json"),
    };
    let mut appearance: serde_json::Map<String, Value> =
        serde_json::from_slice(&fs::read(source)?)?;
    let mut config: tauri::utils::config::WindowConfig = serde_json::from_value(
        appearance
            .remove(label)
            .context("桌面外观配置缺少窗口定义")?,
    )?;
    config.label = label.to_owned();
    config.url = url;
    Ok(config)
}

fn show_setup(app: &AppHandle) -> Result<()> {
    if let Some(window) = app.get_webview_window("setup") {
        window.show()?;
        window.set_focus()?;
    } else {
        let config = window_config(app, None, "setup", WebviewUrl::App("index.html".into()))?;
        WebviewWindowBuilder::from_config(app, &config)?
            .on_navigation(|url| url.scheme() == "tauri")
            .build()?;
    }
    Ok(())
}

fn report(app: &AppHandle, error: anyhow::Error) {
    let message = format!("{error:#}");
    eprintln!("{message}");
    *app.state::<DesktopState>()
        .error
        .lock()
        .expect("桌面错误状态不可用") = Some(message);
    if let Err(error) = show_setup(app) {
        eprintln!("无法显示配置窗口：{error:#}");
    }
}

async fn connect(app: &AppHandle, config: &Path) -> Result<()> {
    let config = fs::canonicalize(config).context("配置文件不存在")?;
    let binary = resources(app)?.join("router");
    run_router(&binary, &config, &["web", "--no-open"]).await?;
    let value: Value = serde_json::from_slice(&fs::read(&config)?)?;
    let address: SocketAddr = value["management"]["listen"]
        .as_str()
        .context("配置缺少网页监听地址")?
        .parse()?;
    ensure!(address.ip().is_loopback(), "桌面控制台仅允许本机回环地址");
    let token_path = PathBuf::from(
        value["management"]["token_file"]
            .as_str()
            .context("配置缺少管理令牌路径")?,
    );
    let token_path = if token_path.is_absolute() {
        token_path
    } else {
        config.parent().context("配置目录不存在")?.join(token_path)
    };
    let token = fs::read_to_string(token_path)?;
    let mut url: tauri::Url = format!("http://{address}/").parse()?;
    url.set_query(Some(&format!(
        "desktop_version={}",
        env!("CARGO_PKG_VERSION")
    )));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let health: Value = client
        .get(url.join("api/health")?)
        .bearer_auth(token.trim())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        health["service"] == "router",
        "控制台端口未提供 router 管理接口"
    );
    let origin = url.origin();
    let mut authenticated = url;
    authenticated.set_fragment(Some(&format!("desktop=1&token={}", token.trim())));
    let label = console_label(&config);
    let mut appearance = window_config(
        app,
        Some(&config),
        "console",
        WebviewUrl::External(authenticated),
    )?;
    appearance.label = label.clone();
    if let Some(window) = app.get_webview_window(&label) {
        window.show()?;
        window.set_focus()?;
    } else {
        WebviewWindowBuilder::from_config(app, &appearance)?
            .on_navigation(move |next| next.origin() == origin)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build()?;
    }
    for (other, window) in app.webview_windows() {
        if other.starts_with("console-") && other != label {
            window.close()?;
        }
    }
    write_json(&preferences_path(app)?, &Preferences { config })?;
    if let Some(window) = app.get_webview_window("setup") {
        window.close()?;
    }
    Ok(())
}

#[tauri::command]
fn setup_info(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    (|| -> Result<Value> {
        setup_guard(&window)?;
        let example: Value =
            serde_json::from_slice(&fs::read(resources(&app)?.join("examples/router.json"))?)?;
        let management: Value = serde_json::from_slice(&fs::read(
            resources(&app)?.join("examples/management.json"),
        )?)?;
        Ok(json!({
            "directory": app.path().home_dir()?.join(".config/router"),
            "launchers": app.path().home_dir()?.join(".local/bin"),
            "gateway": example["runtime"]["server"]["listen"],
            "control": management["listen"],
            "saved": saved_config(&app)?.is_some(),
            "error": app.state::<DesktopState>().error.lock().expect("桌面错误状态不可用").take(),
        }))
    })()
    .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn choose_config(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    async {
        setup_guard(&window)?;
        let state = app.state::<DesktopState>();
        let _guard = state.operation.lock().await;
        if let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("选择 router 配置")
            .add_filter("JSON", &["json"])
            .pick_file()
            .await
        {
            connect(&app, file.path()).await?;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await
    .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn open_saved(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    async {
        setup_guard(&window)?;
        let state = app.state::<DesktopState>();
        let _guard = state.operation.lock().await;
        connect(&app, &saved_config(&app)?.context("尚未选择配置")?).await
    }
    .await
    .map_err(|error| format!("{error:#}"))
}

fn copy_directory(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), target.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn shell_quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"))
}

fn initialize(
    app: &AppHandle,
    directory: &Path,
    launchers: &Path,
    gateway: &str,
    control: &str,
) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    ensure!(
        directory.is_absolute() && launchers.is_absolute(),
        "目录必须使用绝对路径"
    );
    let gateway: SocketAddr = gateway.parse().context("转发监听地址格式错误")?;
    let control: SocketAddr = control.parse().context("控制台监听地址格式错误")?;
    ensure!(
        gateway.ip().is_loopback() && control.ip().is_loopback(),
        "首次安装使用本机回环地址"
    );
    ensure!(
        gateway.port() > 0 && control.port() > 0 && gateway != control,
        "请指定两个不同的有效端口"
    );
    let gateway_socket = std::net::TcpListener::bind(gateway).context("转发端口已被占用")?;
    let control_socket = std::net::TcpListener::bind(control).context("控制台端口已被占用")?;
    ensure!(
        !directory.join("router.json").exists(),
        "此目录已有 router.json，请选择已有配置"
    );
    ensure!(
        !directory.join("application").exists(),
        "此目录已有安装资源，请选择已有配置或使用新目录"
    );
    ensure!(
        !launchers.join("router").exists(),
        "终端入口目录已有 router 命令，请使用其他目录或选择已有配置"
    );
    let source = resources(app)?;
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let directory = fs::canonicalize(directory)?;
    let installation = directory.join("application");
    fs::create_dir_all(installation.join("bin"))?;
    fs::copy(source.join("router"), installation.join("bin/router"))?;
    copy_directory(&source.join("web"), &installation.join("web"))?;
    copy_directory(&source.join("scripts"), &installation.join("scripts"))?;
    for name in ["LICENSE", "THIRD_PARTY_NOTICES.txt"] {
        fs::copy(source.join(name), installation.join(name))?;
    }
    fs::copy(
        source.join("appearance.json"),
        directory.join("desktop.json"),
    )?;
    let mut value: Value = serde_json::from_slice(&fs::read(source.join("examples/router.json"))?)?;
    let mut management: Value =
        serde_json::from_slice(&fs::read(source.join("examples/management.json"))?)?;
    value["runtime"]["server"]["listen"] = json!(gateway.to_string());
    management["listen"] = json!(control.to_string());
    management["assets_directory"] = json!(installation.join("web"));
    management["launcher_directory"] = json!(launchers);
    let config = directory.join("router.json");
    let label = format!(
        "app.router.gateway.{}",
        &blake3::hash(config.to_string_lossy().as_bytes()).to_hex()[..16]
    );
    let uid = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()?;
    ensure!(uid.status.success(), "无法读取当前用户 ID");
    let domain = format!("gui/{}", String::from_utf8(uid.stdout)?.trim());
    let service_file = directory.join("service.plist");
    for action in ["start", "restart"] {
        management[format!("service_{action}")] = json!({"program":"/bin/zsh", "args":[installation.join("scripts/macos-service.zsh"),action,&domain,&label,&service_file]});
    }
    value["management"] = management;
    write_json(&config, &value)?;
    fs::create_dir_all(directory.join("logs"))?;
    let service = json!({"Label": label, "ProgramArguments": ["/usr/bin/env", "-u", "ROUTER_ADDR", installation.join("bin/router"), "--config", config, "serve"], "RunAtLoad": true, "KeepAlive": true, "StandardOutPath": directory.join("logs/gateway.log"), "StandardErrorPath": directory.join("logs/gateway.log")});
    plist::to_file_xml(&service_file, &service)?;
    drop((gateway_socket, control_socket));
    Ok(config)
}

#[tauri::command]
async fn create_config(
    app: AppHandle,
    window: WebviewWindow,
    directory: String,
    launchers: String,
    gateway: String,
    control: String,
    shell: bool,
) -> Result<(), String> {
    async {
        setup_guard(&window)?;
        let state = app.state::<DesktopState>();
        let _guard = state.operation.lock().await;
        let installer_app = app.clone();
        let config = tauri::async_runtime::spawn_blocking(move || {
            initialize(
                &installer_app,
                Path::new(&directory),
                Path::new(&launchers),
                &gateway,
                &control,
            )
        })
        .await??;
        let binary = config
            .parent()
            .context("安装目录不存在")?
            .join("application/bin/router");
        run_router(&binary, &config, &["config", "check"]).await?;
        let result: Value =
            serde_json::from_slice(&run_router(&binary, &config, &["commands", "install"]).await?)?;
        if shell {
            let init = PathBuf::from(
                result["shell_init"]
                    .as_str()
                    .context("初始化脚本路径缺失")?,
            );
            let launcher = PathBuf::from(result["directory"].as_str().context("终端目录缺失")?);
            let line = format!(
                "\n# Router 终端入口\nexport PATH={}:\"$PATH\"\nsource {}\n",
                shell_quote(&launcher),
                shell_quote(&init)
            );
            let zshrc = app.path().home_dir()?.join(".zshrc");
            let current = if zshrc.exists() {
                fs::read_to_string(&zshrc)?
            } else {
                String::new()
            };
            if !current.contains(line.trim()) {
                if zshrc.exists() {
                    fs::copy(
                        &zshrc,
                        config
                            .parent()
                            .context("配置目录不存在")?
                            .join("zshrc.before-router"),
                    )?;
                }
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(zshrc)?
                    .write_all(line.as_bytes())?;
            }
        }
        connect(&app, &config).await
    }
    .await
    .map_err(|error| format!("{error:#}"))
}

fn main() {
    let arguments = Arguments::parse();
    let application = tauri::Builder::default()
        .manage(DesktopState::default())
        .invoke_handler(tauri::generate_handler![
            setup_info,
            choose_config,
            create_config,
            open_saved
        ])
        .setup(move |app| {
            let handle = app.handle();
            let configuration = MenuItem::with_id(
                handle,
                "configuration",
                "选择配置…",
                true,
                Some("CmdOrCtrl+O"),
            )?;
            let reopen =
                MenuItem::with_id(handle, "console", "打开控制台", true, Some("CmdOrCtrl+1"))?;
            let submenu = Submenu::with_items(
                handle,
                "Router",
                true,
                &[
                    &PredefinedMenuItem::about(handle, Some("关于 Router"), None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &reopen,
                    &configuration,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::hide(handle, None)?,
                    &PredefinedMenuItem::quit(handle, Some("退出控制台"))?,
                ],
            )?;
            let edit = Submenu::with_items(
                handle,
                "编辑",
                true,
                &[
                    &PredefinedMenuItem::undo(handle, None)?,
                    &PredefinedMenuItem::redo(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::cut(handle, None)?,
                    &PredefinedMenuItem::copy(handle, None)?,
                    &PredefinedMenuItem::paste(handle, None)?,
                    &PredefinedMenuItem::select_all(handle, None)?,
                ],
            )?;
            let window = Submenu::with_items(
                handle,
                "窗口",
                true,
                &[
                    &PredefinedMenuItem::minimize(handle, None)?,
                    &PredefinedMenuItem::maximize(handle, None)?,
                    &PredefinedMenuItem::close_window(handle, Some("关闭窗口"))?,
                ],
            )?;
            app.set_menu(Menu::with_items(handle, &[&submenu, &edit, &window])?)?;
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let config = match arguments.config {
                        Some(path) => Some(path),
                        None => saved_config(&handle)?,
                    };
                    if !arguments.setup
                        && let Some(config) = config
                    {
                        return connect(&handle, &config).await;
                    }
                    show_setup(&handle)
                }
                .await;
                if let Err(error) = result {
                    report(&handle, error);
                }
            });
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "configuration" => {
                if let Err(error) = show_setup(app) {
                    report(app, error);
                }
            }
            "console" => open_console(app),
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("无法启动 Router 桌面控制台");
    application.run(|app, event| {
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            open_console(app);
        }
    });
}

fn console_label(path: &Path) -> String {
    format!(
        "console-{}",
        &blake3::hash(path.to_string_lossy().as_bytes()).to_hex()[..16]
    )
}

fn open_console(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let state = app.state::<DesktopState>();
            let _guard = state.operation.lock().await;
            if let Some(config) = saved_config(&app)? {
                connect(&app, &config).await
            } else {
                show_setup(&app)
            }
        }
        .await;
        if let Err(error) = result {
            report(&app, error);
        }
    });
}
