use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;

fn installation(config: &Path) -> Result<PathBuf> {
    Ok(config.parent().context("配置目录缺失")?.join("application"))
}

pub fn needs_update(app: &AppHandle, config: &Path) -> Result<bool> {
    let directory = installation(config)?;
    let value: Value = serde_json::from_slice(&fs::read(config)?)?;
    let service_file = config
        .parent()
        .context("配置目录缺失")?
        .join("service.plist");
    if !service_file.exists()
        || value["management"]["assets_directory"].as_str() != directory.join("web").to_str()
    {
        return Ok(false);
    }
    let service: Value = plist::from_file(service_file)?;
    if !service["ProgramArguments"].as_array().is_some_and(|args| {
        args.iter()
            .any(|arg| arg.as_str() == directory.join("bin/router").to_str())
    }) {
        return Ok(false);
    }
    let current = directory.join("installation.json");
    if !current.exists() {
        return Ok(true);
    }
    let deployed: Value = serde_json::from_slice(&fs::read(current)?)?;
    let bundled: Value =
        serde_json::from_slice(&fs::read(super::resources(app)?.join("installation.json"))?)?;
    Ok(deployed != bundled)
}

pub async fn update(app: &AppHandle, config: &Path) -> Result<()> {
    let directory = installation(config)?;
    let parent = config.parent().context("配置目录缺失")?;
    let value: Value = serde_json::from_slice(&fs::read(config)?)?;
    let token_path = PathBuf::from(
        value["management"]["token_file"]
            .as_str()
            .context("管理令牌路径缺失")?,
    );
    let token_path = if token_path.is_absolute() {
        token_path
    } else {
        parent.join(token_path)
    };
    // 通过现有服务读取任务状态，更新前保留正在执行的任务。
    super::run_router(&directory.join("bin/router"), config, &["web", "--no-open"]).await?;
    let token = fs::read_to_string(token_path)?;
    let address = value["management"]["listen"]
        .as_str()
        .context("控制台地址缺失")?;
    let jobs: Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?
        .get(format!("http://{address}/api/transcription/jobs"))
        .bearer_auth(token.trim())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        jobs["jobs"]
            .as_array()
            .context("转写任务状态缺失")?
            .iter()
            .all(|job| !matches!(
                job["status"].as_str(),
                Some("running" | "queued" | "cancelling")
            )),
        "转写任务正在执行，请在控制台完成或取消任务后更新"
    );
    let stage = tempfile::Builder::new()
        .prefix("application-update-")
        .tempdir_in(parent)?;
    let staged = stage.path().join("application");
    super::copy_directory(&directory, &staged)?;
    let source = super::resources(app)?;
    for (input, output) in [
        ("router", "bin/router"),
        ("spt-core", "bin/spt-core"),
        ("LICENSE", "LICENSE"),
        ("THIRD_PARTY_NOTICES.txt", "THIRD_PARTY_NOTICES.txt"),
        ("SPT-LICENSE", "SPT-LICENSE"),
        ("installation.json", "installation.json"),
    ] {
        fs::copy(source.join(input), staged.join(output))?;
    }
    for name in ["web", "scripts", "spt-licenses"] {
        fs::remove_dir_all(staged.join(name))?;
        super::copy_directory(&source.join(name), &staged.join(name))?;
    }
    super::run_router(&staged.join("bin/router"), config, &["config", "check"]).await?;
    let stage = stage.keep();
    let previous = stage.join("previous");
    fs::rename(&directory, &previous)?;
    if let Err(error) = fs::rename(&staged, &directory) {
        fs::rename(&previous, &directory)?;
        fs::remove_dir_all(&stage)?;
        return Err(error.into());
    }
    if let Err(error) = super::run_router(
        &directory.join("bin/router"),
        config,
        &["web", "--restart", "--no-open"],
    )
    .await
    {
        fs::rename(&directory, &staged)?;
        fs::rename(&previous, &directory)?;
        super::run_router(
            &directory.join("bin/router"),
            config,
            &["web", "--restart", "--no-open"],
        )
        .await
        .context("恢复已有组件后启动服务失败")?;
        fs::remove_dir_all(&stage)?;
        return Err(error.context("更新失败，已恢复原有组件"));
    }
    fs::remove_dir_all(stage)?;
    Ok(())
}
