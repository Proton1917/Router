use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::{
    control::{self, CommandSpec, ProcessSpec},
    server::{CommandAction, ConfigAction, CredentialAction, ResourceAction, RouterCommand},
};

pub async fn dispatch(path: &Path, command: RouterCommand) -> Result<()> {
    match command {
        RouterCommand::Serve => unreachable!("serve is handled by main"),
        RouterCommand::Web {
            no_open,
            restart,
            foreground,
        } => {
            ensure_gateway(path).await?;
            if foreground {
                return control::serve(path.to_owned()).await;
            }
            ensure_control(path, restart).await?;
            if !no_open {
                let manager = control::management(&control::load_value(path)?)?;
                process(
                    &manager.browser,
                    &json!({"url":control::browser_url(path)?}),
                    false,
                )?;
            }
            let manager = control::management(&control::load_value(path)?)?;
            println!("router 管理界面：http://{}", manager.listen);
        }
        RouterCommand::Status => {
            let value = control::load_value(path)?;
            let manager = control::management(&value)?;
            let health = gateway_health(&value)?;
            let online = reqwest::Client::new()
                .get(health)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            print_json(
                &json!({"gateway_online":online,"gateway_url":control::gateway_url(&value)?,"web_url":format!("http://{}",manager.listen),"commands":manager.commands.len(),"config":path}),
            )?;
        }
        RouterCommand::Run {
            name,
            dry_run,
            args,
        } => {
            if dry_run {
                print_json(&launch_preview(path, &name, &args)?)?;
            } else {
                ensure_gateway_for_command(path, &name).await?;
                run(path, &name, &args, false)?;
            }
        }
        RouterCommand::Execute { program, args } => {
            let name = std::env::var("ROUTER_COMMAND_NAME").context("缺少启动命令名称")?;
            let dry_run = std::env::var_os("ROUTER_LAUNCH_PREVIEW").is_some();
            let value = control::load_value(path)?;
            let manager = control::management(&value)?;
            let entry = manager.commands.get(&name).context("命令不存在")?;
            let template = &manager.templates[&entry.template];
            if !dry_run
                && template.request_path.is_some()
                && template.api_gate.as_ref().is_none_or(|gate| {
                    std::env::var(&gate.name)
                        .ok()
                        .is_some_and(|value| value.contains(&gate.contains))
                })
            {
                ensure_gateway(path).await?;
            }
            execute_client(path, &name, &program, &args, dry_run)?;
        }
        RouterCommand::Commands { action } => commands(path, action)?,
        RouterCommand::Backends { action } => resource(path, "/runtime/backends", action)?,
        RouterCommand::Models { action } => resource(path, "/profiles", action)?,
        RouterCommand::Templates { action } => resource(path, "/management/templates", action)?,
        RouterCommand::Routes { action } => resource(path, "/routes", action)?,
        RouterCommand::Config { action } => match action {
            ConfigAction::Show => print_json(&control::load_value(path)?)?,
            ConfigAction::Check => {
                crate::load_router_runtime_config(path)?;
                println!("配置校验通过");
            }
            ConfigAction::Apply { file } => {
                let current = control::load_value(path)?;
                let outcome = control::save_candidate(
                    path,
                    &control::revision(&current)?,
                    control::load_value(&file)?,
                )?;
                print_json(
                    &json!({"revision":outcome["revision"],"validation_changes":outcome["validation_changes"]}),
                )?;
            }
            ConfigAction::Management { file } => {
                let current = control::load_value(path)?;
                let source = control::load_value(&file)?;
                let mut candidate = current.clone();
                candidate["management"] = source.get("management").cloned().unwrap_or(source);
                let outcome =
                    control::save_candidate(path, &control::revision(&current)?, candidate)?;
                print_json(
                    &json!({"saved":true,"revision":outcome["revision"],"validation_changes":outcome["validation_changes"]}),
                )?;
            }
        },
        RouterCommand::Credentials { action } => match action {
            CredentialAction::Set { id, file } => {
                let value = fs::read_to_string(file)?;
                let target = control::save_credential(path, &id, &value)?;
                print_json(&json!({"saved":true,"path":target}))?;
            }
        },
    }
    Ok(())
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn gateway_health(value: &Value) -> Result<String> {
    Ok(format!(
        "{}{}",
        control::gateway_url(value)?,
        value
            .pointer("/runtime/server/health_path")
            .and_then(Value::as_str)
            .context("缺少健康检查路径")?
    ))
}

fn process(spec: &ProcessSpec, context: &Value, detached: bool) -> Result<()> {
    let mut command = Command::new(control::render(&spec.program, context)?);
    for arg in &spec.args {
        command.arg(control::render(arg, context)?);
    }
    if detached {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.spawn()?;
    } else {
        ensure!(command.status()?.success(), "启动程序执行失败");
    }
    Ok(())
}

async fn ensure_gateway_for_command(path: &Path, name: &str) -> Result<()> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let entry = manager.commands.get(name).context("命令不存在")?;
    if !manager.templates[&entry.template].deferred
        && manager.templates[&entry.template].request_path.is_some()
    {
        ensure_gateway(path).await?;
    }
    Ok(())
}

pub async fn ensure_gateway(path: &Path) -> Result<()> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let url = gateway_health(&value)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    if gateway_identity(&client, &url, path).await? {
        return Ok(());
    }
    if let Some(start) = &manager.service_start {
        process(
            start,
            &json!({"config_path":path,"executable":std::env::current_exe()?}),
            false,
        )?;
    } else {
        let log_directory = control::resolve_path(path, &manager.log_directory);
        fs::create_dir_all(&log_directory)?;
        let output = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_directory.join("gateway.log"))?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--config"])
            .arg(path)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output);
        command.env_remove("ROUTER_ADDR");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.spawn()?;
    }
    for _ in 0..50 {
        if gateway_identity(&client, &url, path).await? {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("转发服务启动后未通过健康检查")
}

async fn gateway_identity(client: &reqwest::Client, url: &str, path: &Path) -> Result<bool> {
    let response = match client.get(format!("{url}?format=json")).send().await {
        Ok(response) => response,
        Err(error) if error.is_connect() || error.is_timeout() => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        response.status().is_success(),
        "监听地址已有服务，但健康检查失败"
    );
    let value: Value = response
        .json()
        .await
        .context("现有服务未提供 router 实例信息，请重启对应转发服务")?;
    let expected = blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    ensure!(
        value["service"].as_str() == Some("router")
            && value["config_id"].as_str() == Some(&expected),
        "此监听地址上的 router 使用了其他配置文件，请调整地址或停止该实例"
    );
    Ok(true)
}

pub async fn restart_gateway(path: &Path) -> Result<()> {
    let manager = control::management(&control::load_value(path)?)?;
    let restart = manager.service_restart.context("未配置服务重启命令")?;
    process(
        &restart,
        &json!({"config_path":path,"executable":std::env::current_exe()?}),
        false,
    )?;
    ensure_gateway(path).await
}

async fn ensure_control(path: &Path, restart: bool) -> Result<()> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let url = format!("http://{}/api/health", manager.listen);
    let token = control::control_token(path)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let registry_path = path.with_extension("web-state.json");
    if registry_path.exists() {
        let registry = control::load_value(&registry_path)?;
        let address: std::net::SocketAddr = registry["listen"]
            .as_str()
            .context("管理服务状态缺少地址")?
            .parse()?;
        ensure!(
            address.ip().is_loopback(),
            "管理服务状态地址必须为 loopback"
        );
        let previous_token = fs::read_to_string(
            registry["token_file"]
                .as_str()
                .context("管理服务状态缺少令牌文件")?,
        )?;
        let previous_url = format!("http://{address}");
        if client
            .get(format!("{previous_url}/api/health"))
            .bearer_auth(previous_token.trim())
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            if !restart && address == manager.listen && previous_token.trim() == token {
                return Ok(());
            }
            client
                .post(format!("{previous_url}/api/stop"))
                .bearer_auth(previous_token.trim())
                .send()
                .await?
                .error_for_status()?;
            for _ in 0..50 {
                if client
                    .get(format!("{previous_url}/api/health"))
                    .bearer_auth(previous_token.trim())
                    .send()
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    if client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
    {
        ensure!(!restart, "现有管理服务没有重启状态记录，请先停止该进程");
        return Ok(());
    }
    let logs = control::resolve_path(path, &manager.log_directory);
    fs::create_dir_all(&logs)?;
    let output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("control.log"))?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--config")
        .arg(path)
        .args(["web", "--foreground", "--no-open"])
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    for _ in 0..50 {
        if let Some(status) = child.try_wait()? {
            bail!("管理服务启动失败: {status}，请查看 control.log");
        }
        if client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("管理服务启动后未通过健康检查")
}

fn commands(path: &Path, action: CommandAction) -> Result<()> {
    let current = control::load_value(path)?;
    let mut candidate = current.clone();
    match action {
        CommandAction::List => return print_json(&current["management"]["commands"]),
        CommandAction::Get { name } => {
            return print_json(
                current["management"]["commands"]
                    .get(&name)
                    .context("命令不存在")?,
            );
        }
        CommandAction::Install { name } => {
            return print_json(&install_commands(path, name.as_deref())?);
        }
        CommandAction::Add {
            name,
            template,
            model,
            profile,
        } => {
            control::validate_name(&name)?;
            ensure!(
                candidate["management"]["commands"].get(&name).is_none(),
                "命令已存在: {name}"
            );
            candidate["management"]["commands"][&name] =
                json!({"template":template,"model":model,"profile":profile,"enabled":true});
        }
        CommandAction::Put { name, file } => {
            control::validate_name(&name)?;
            let command: CommandSpec = serde_json::from_value(control::load_value(&file)?)?;
            candidate["management"]["commands"][&name] = serde_json::to_value(command)?;
        }
        CommandAction::Remove { name } => {
            ensure!(
                candidate["management"]["commands"]
                    .as_object_mut()
                    .context("commands 必须为对象")?
                    .remove(&name)
                    .is_some(),
                "命令不存在"
            );
        }
    }
    let outcome = control::save_candidate(path, &control::revision(&current)?, candidate)?;
    print_json(&json!({"revision":outcome["revision"],"saved":true}))
}

fn resource(path: &Path, pointer: &str, action: ResourceAction) -> Result<()> {
    let current = control::load_value(path)?;
    let mut candidate = current.clone();
    let collection = candidate.pointer_mut(pointer).context("配置集合不存在")?;
    let value = match action {
        ResourceAction::List => return print_json(collection),
        ResourceAction::Get { id } => {
            let item = if let Some(array) = collection.as_array() {
                array.iter().find(|item| item["id"].as_str() == Some(&id))
            } else {
                collection.get(&id)
            };
            return print_json(item.context("对象不存在")?);
        }
        ResourceAction::Put { id, file } => {
            let mut value = control::load_value(&file)?;
            if let Some(array) = collection.as_array_mut() {
                value["id"] = Value::String(id.clone());
                if let Some(index) = array
                    .iter()
                    .position(|item| item["id"].as_str() == Some(&id))
                {
                    array[index] = value;
                } else {
                    let index = array.len().saturating_sub(1);
                    array.insert(index, value);
                }
            } else {
                collection
                    .as_object_mut()
                    .context("配置集合不是对象")?
                    .insert(id, value);
            }
            candidate
        }
        ResourceAction::Remove { id } => {
            if let Some(array) = collection.as_array_mut() {
                let index = array
                    .iter()
                    .position(|item| item["id"].as_str() == Some(&id))
                    .context("对象不存在")?;
                array.remove(index);
            } else {
                ensure!(
                    collection
                        .as_object_mut()
                        .context("配置集合不是对象")?
                        .remove(&id)
                        .is_some(),
                    "对象不存在"
                );
            }
            candidate
        }
    };
    let outcome = control::save_candidate(path, &control::revision(&current)?, value)?;
    print_json(&json!({"revision":outcome["revision"],"saved":true}))
}

fn env_plan(
    value: &Value,
    name: &str,
    final_stage: bool,
) -> Result<(BTreeMap<String, String>, Vec<String>)> {
    let manager = control::management(value)?;
    let entry = manager.commands.get(name).context("命令不存在")?;
    let template = &manager.templates[&entry.template];
    let context = control::command_context(value, name)?;
    let mut environment = BTreeMap::new();
    for (key, value) in &template.env {
        environment.insert(key.clone(), control::render(value, &context)?);
    }
    let api = final_stage
        && template.request_path.is_some()
        && template.api_gate.as_ref().is_none_or(|gate| {
            environment
                .get(&gate.name)
                .cloned()
                .or_else(|| std::env::var(&gate.name).ok())
                .is_some_and(|value| value.contains(&gate.contains))
        });
    if api {
        for (key, value) in &template.api_env {
            environment.insert(key.clone(), control::render(value, &context)?);
        }
    }
    for (key, value) in &entry.env {
        environment.insert(key.clone(), control::render(value, &context)?);
    }
    if api && let Some(header_env) = &template.header_env {
        let existing = environment
            .get(header_env)
            .cloned()
            .or_else(|| std::env::var(header_env).ok())
            .unwrap_or_default();
        let mut lines = existing
            .lines()
            .filter(|line| {
                line.split_once(':').is_none_or(|(key, _)| {
                    !key.trim().eq_ignore_ascii_case(&manager.command_header)
                })
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for (key, value) in &template.request_headers {
            if !lines.iter().any(|line| {
                line.split_once(':')
                    .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
            }) {
                lines.push(format!("{key}: {}", control::render(value, &context)?));
            }
        }
        lines.push(format!("{}: {name}", manager.command_header));
        environment.insert(header_env.clone(), lines.join("\n"));
    }
    let removed = template
        .unset_env
        .iter()
        .chain(&entry.unset_env)
        .cloned()
        .collect();
    Ok((environment, removed))
}

struct LaunchSpec {
    program: String,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    removed: Vec<String>,
    deferred: bool,
}

fn launch_spec(path: &Path, name: &str, extra: &[String]) -> Result<LaunchSpec> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let entry = manager.commands.get(name).context("命令不存在")?;
    ensure!(entry.enabled, "命令已停用: {name}");
    let template = &manager.templates[&entry.template];
    let context = control::command_context(&value, name)?;
    let program = control::render(
        entry.program.as_deref().unwrap_or(&template.program),
        &context,
    )?;
    let mut args = template
        .args
        .iter()
        .map(|arg| control::render(arg, &context))
        .collect::<Result<Vec<_>>>()?;
    for (id, option) in &template.options {
        let selected = entry.options.get(id).unwrap_or(&option.default);
        args.extend(
            option.choices[selected]
                .args
                .iter()
                .map(|arg| control::render(arg, &context))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    args.extend(
        entry
            .args
            .iter()
            .map(|arg| control::render(arg, &context))
            .collect::<Result<Vec<_>>>()?,
    );
    let overridden = args.iter().chain(extra).any(|arg| {
        template
            .model_flags
            .iter()
            .any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))
    });
    let mut deferred_model = None;
    if !overridden
        && let (Some(model), Some(flag)) = (
            control::effective_model(&value, name)?,
            template.model_flags.first(),
        )
    {
        if let Some(variable) = &template.deferred_model_env {
            deferred_model = Some((variable.clone(), model));
        } else {
            args.extend([flag.clone(), model]);
        }
    }
    args.extend_from_slice(extra);
    let (mut environment, removed) = env_plan(&value, name, !template.deferred)?;
    if let Some((variable, model)) = deferred_model {
        environment.insert(variable, model);
    }
    if template.deferred {
        environment.insert(
            "ROUTER_EXECUTABLE".to_owned(),
            std::env::current_exe()?.to_string_lossy().into_owned(),
        );
        environment.insert("ROUTER_COMMAND_NAME".to_owned(), name.to_owned());
        environment.insert(
            "ROUTER_CONFIG".to_owned(),
            path.to_string_lossy().into_owned(),
        );
    }
    Ok(LaunchSpec {
        program,
        args,
        environment,
        removed,
        deferred: template.deferred,
    })
}

fn configured_process(
    program: &str,
    args: &[String],
    environment: &BTreeMap<String, String>,
    removed: &[String],
) -> Command {
    let mut process = Command::new(program);
    process.args(args);
    for name in removed {
        process.env_remove(name);
    }
    process.envs(environment);
    process
}

fn masked_environment(environment: BTreeMap<String, String>) -> BTreeMap<String, String> {
    environment
        .into_iter()
        .map(|(name, value)| {
            let upper = name.to_ascii_uppercase();
            let sensitive =
                upper.ends_with("_TOKEN") || upper.contains("SECRET") || upper.ends_with("API_KEY");
            (
                name,
                if sensitive && !value.is_empty() {
                    "••••".to_owned()
                } else {
                    value
                },
            )
        })
        .collect()
}

pub fn launch_preview(path: &Path, name: &str, extra: &[String]) -> Result<Value> {
    let LaunchSpec {
        program,
        args,
        environment,
        removed,
        deferred,
    } = launch_spec(path, name, extra)?;
    if deferred {
        let output = configured_process(&program, &args, &environment, &removed)
            .env("ROUTER_LAUNCH_PREVIEW", "1")
            .output()?;
        ensure!(
            output.status.success(),
            "启动器预览失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return serde_json::from_slice(&output.stdout).context("启动器未返回有效的预览 JSON");
    }
    Ok(
        json!({"program":program,"args":args,"env":masked_environment(environment),"unset_env":removed}),
    )
}

fn run(path: &Path, name: &str, extra: &[String], dry_run: bool) -> Result<()> {
    let LaunchSpec {
        program,
        args,
        environment,
        removed,
        ..
    } = launch_spec(path, name, extra)?;
    if dry_run {
        return print_json(&launch_preview(path, name, extra)?);
    }
    let mut process = configured_process(&program, &args, &environment, &removed);
    replace_process(&mut process)
}

fn execute_client(
    path: &Path,
    name: &str,
    program: &str,
    args: &[String],
    preview: bool,
) -> Result<()> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let entry = manager.commands.get(name).context("命令不存在")?;
    let template = &manager.templates[&entry.template];
    let (environment, mut removed) = env_plan(&value, name, true)?;
    removed.extend([
        "ROUTER_EXECUTABLE".to_owned(),
        "ROUTER_COMMAND_NAME".to_owned(),
        "ROUTER_LAUNCH_PREVIEW".to_owned(),
    ]);
    if let Some(variable) = &template.deferred_model_env {
        removed.push(variable.clone());
    }
    if preview {
        let mut visible = template
            .capture_env
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect::<BTreeMap<_, _>>();
        visible.extend(environment);
        return print_json(
            &json!({"program":program,"args":args,"env":masked_environment(visible),"unset_env":removed}),
        );
    }
    replace_process(&mut configured_process(
        program,
        args,
        &environment,
        &removed,
    ))
}

fn replace_process(command: &mut Command) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).context("无法启动客户端")
    }
    #[cfg(not(unix))]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn install_commands(path: &Path, selected: Option<&str>) -> Result<Value> {
    let value = control::load_value(path)?;
    let manager = control::management(&value)?;
    let shell = control::resolve_path(path, &manager.shell_file);
    let init = shell.with_extension("init.zsh");
    for (file, marker) in [
        (&shell, "# router-commands:"),
        (&init, "# 由 router 管理\n"),
    ] {
        if file.exists() {
            ensure!(
                fs::read_to_string(file)?.starts_with(marker),
                "目标文件由其他配置管理，未覆盖: {}",
                file.display()
            );
        }
    }
    let directory = control::resolve_path(path, &manager.launcher_directory);
    fs::create_dir_all(&directory)?;
    let inventory_path = directory.join(".router-managed.json");
    let mut inventory: BTreeMap<String, String> = if inventory_path.exists() {
        serde_json::from_slice(&fs::read(&inventory_path)?)?
    } else {
        BTreeMap::new()
    };
    let executable = std::env::current_exe()?;
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "router".to_owned(),
        format!(
            "#!/bin/sh\n# 由 router 管理\nif [ -z \"${{ROUTER_CONFIG:-}}\" ]; then ROUTER_CONFIG={}; fi\nexport ROUTER_CONFIG\nexec {} \"$@\"\n",
            quote(&path.to_string_lossy()),
            quote(&executable.to_string_lossy())
        ),
    );
    if let Some(name) = selected {
        ensure!(manager.commands.contains_key(name), "命令不存在");
    }
    let obsolete = inventory
        .keys()
        .filter(|name| name.as_str() != "router" && !manager.commands.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    for name in outputs.keys().chain(&obsolete) {
        let destination = directory.join(name);
        if destination.exists() {
            let data = fs::read(&destination)?;
            let hash = blake3::hash(&data).to_hex().to_string();
            ensure!(
                inventory.get(name) == Some(&hash)
                    || outputs
                        .get(name)
                        .is_some_and(|text| text.as_bytes() == data),
                "命令文件已存在或被外部修改，未覆盖: {}",
                destination.display()
            );
        }
    }
    for (name, text) in &outputs {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(text.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        temporary
            .persist(directory.join(name))
            .map_err(|error| error.error)?;
        inventory.insert(
            name.clone(),
            blake3::hash(text.as_bytes()).to_hex().to_string(),
        );
    }
    for name in obsolete {
        let destination = directory.join(&name);
        if destination.exists() {
            fs::remove_file(destination)?;
        }
        inventory.remove(&name);
    }
    control::write_json(&inventory_path, &serde_json::to_value(inventory)?)?;
    let parent = shell.parent().context("Shell 配置目录不存在")?;
    fs::create_dir_all(parent)?;
    let mut definitions = String::from(
        "if (( ${+_ROUTER_MANAGED_NAMES} )); then\n  for _router_old in \"${_ROUTER_MANAGED_NAMES[@]}\"; do\n    if (( ${+functions[$_router_old]} )); then unfunction -- \"$_router_old\"; fi\n  done\nfi\ntypeset -ga _ROUTER_MANAGED_NAMES=(",
    );
    definitions.push_str(
        &manager
            .commands
            .keys()
            .map(|name| quote(name))
            .collect::<Vec<_>>()
            .join(" "),
    );
    definitions.push_str(")\n");
    for name in manager.commands.keys() {
        definitions.push_str(&format!("if (( $+aliases[{name}] )); then unalias -- {name}; fi\nfunction {name}() {{ command {} --config {} run {} -- \"$@\"; }}\n",quote(&executable.to_string_lossy()),quote(&path.to_string_lossy()),quote(name)));
    }
    let hash = blake3::hash(definitions.as_bytes()).to_hex().to_string();
    let text = format!("# router-commands:{hash}\n{definitions}");
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(text.as_bytes())?;
    ensure!(
        Command::new("/bin/zsh")
            .arg("-n")
            .arg(temporary.path())
            .status()?
            .success(),
        "生成的 Shell 命令未通过语法检查"
    );
    temporary.persist(&shell).map_err(|error| error.error)?;
    let initialization = format!(
        "# 由 router 管理\n_router_sync_commands() {{\n  local router_command_revision\n  [[ -r {} ]] || return 1\n  IFS= read -r router_command_revision < {}\n  if [[ \"${{_ROUTER_COMMANDS_REVISION:-}}\" != \"$router_command_revision\" ]]; then\n    source {} || return\n    typeset -g _ROUTER_COMMANDS_REVISION=\"$router_command_revision\"\n  fi\n}}\nautoload -Uz add-zsh-hook\nadd-zsh-hook precmd _router_sync_commands\nadd-zsh-hook preexec _router_sync_commands\n_router_sync_commands\n",
        quote(&shell.to_string_lossy()),
        quote(&shell.to_string_lossy()),
        quote(&shell.to_string_lossy())
    );
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(initialization.as_bytes())?;
    temporary.persist(&init).map_err(|error| error.error)?;
    Ok(
        json!({"directory":directory,"installed":manager.commands.keys().collect::<Vec<_>>(),"shell_init":init}),
    )
}
