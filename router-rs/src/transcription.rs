use std::{
    collections::BTreeMap,
    convert::Infallible,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path as PathParam, Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, Notify, RwLock, watch},
};

use crate::{
    control::{self, ApiError, ControlState},
    routing::RouterConfig,
};

pub const TASK_HEADER: &str = "x-router-task";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub profile: String,
    pub reported_model_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub program: PathBuf,
    pub engine_config: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub directory: PathBuf,
    pub models: BTreeMap<String, Model>,
    pub default_model: String,
    pub transcription_prompt: String,
    #[serde(default)]
    pub engine_options: BTreeMap<String, u64>,
    pub max_upload_bytes: u64,
    pub max_storage_bytes: u64,
    pub max_temp_bytes_per_job: u64,
    pub max_result_bytes: u64,
    pub max_jobs: usize,
    pub max_concurrent_jobs: usize,
    pub max_job_seconds: u64,
    pub retention_days: u64,
    pub cleanup_seconds: u64,
    pub cancel_grace_seconds: u64,
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.program.as_os_str().is_empty() && !self.engine_config.as_os_str().is_empty(),
            "请配置 SPT 程序和引擎配置文件"
        );
        ensure!(
            !self.directory.as_os_str().is_empty(),
            "转写任务目录不能为空"
        );
        ensure!(
            self.max_concurrent_jobs > 0
                && self.max_concurrent_jobs <= 8
                && self.max_jobs >= self.max_concurrent_jobs,
            "任务数量或并发设置无效"
        );
        ensure!(
            self.max_upload_bytes > 0
                && self.max_temp_bytes_per_job > 0
                && self.max_result_bytes > 0
                && self.max_result_bytes <= 64 * 1024 * 1024,
            "转写资源限制无效"
        );
        let reserve = self
            .max_temp_bytes_per_job
            .checked_add(self.max_result_bytes * 2)
            .and_then(|n| n.checked_mul(self.max_concurrent_jobs as u64))
            .context("转写空间设置过大")?;
        ensure!(
            self.max_storage_bytes > reserve.saturating_add(self.max_upload_bytes),
            "总空间上限须大于上传及运行任务预留空间"
        );
        ensure!(
            self.max_job_seconds > 0
                && self.retention_days > 0
                && self.cleanup_seconds >= 10
                && self.cancel_grace_seconds > 0,
            "转写时间限制无效"
        );
        ensure!(
            self.transcription_prompt.len() <= 32768
                && !self.transcription_prompt.trim().is_empty(),
            "转写提示词不能为空或超过 32 KiB"
        );
        ensure!(
            self.models.is_empty() || self.models.contains_key(&self.default_model),
            "默认转写模型不存在"
        );
        for (id, model) in &self.models {
            control::validate_name(id)?;
            ensure!(
                !model.profile.is_empty() && !model.reported_model_prefix.is_empty(),
                "转写模型需要路由配置和响应模型标识"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    id: String,
    name: String,
    model: String,
    mode: String,
    kind: String,
    status: String,
    created_at: u64,
    updated_at: u64,
    stage: String,
    completed_ms: u64,
    total_ms: u64,
    message: String,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    model_label: String,
    #[serde(default)]
    api_model: String,
    #[serde(default)]
    target: String,
}

impl Job {
    fn terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "succeeded" | "failed" | "cancelled" | "interrupted"
        )
    }
}

pub struct Service {
    config: PathBuf,
    directory: PathBuf,
    jobs: Mutex<BTreeMap<String, Job>>,
    admission: Mutex<()>,
    active: RwLock<BTreeMap<String, Arc<RouterConfig>>>,
    cancellations: Mutex<BTreeMap<String, watch::Sender<bool>>>,
    changed: watch::Sender<u64>,
    wake: Notify,
    shutdown: watch::Receiver<bool>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs()
}
fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("缺少任务目录")?)?;
    serde_json::to_writer(&mut file, value)?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn disk_bytes(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if entry.file_type()?.is_dir() {
            total += disk_bytes(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

impl Service {
    pub async fn start(config: &Path, shutdown: watch::Receiver<bool>) -> Result<Arc<Self>> {
        let value = control::load_value(config)?;
        let configured = if value["management"].is_null() {
            None
        } else {
            control::management(&value)?.transcription
        };
        let directory = control::resolve_path(
            config,
            configured
                .as_ref()
                .map(|v| v.directory.as_path())
                .unwrap_or(Path::new("transcriptions")),
        );
        let (changed, _) = watch::channel(0);
        let mut jobs = BTreeMap::new();
        if directory.exists() {
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let id = entry.file_name().to_string_lossy().into_owned();
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                if id.starts_with("upload-") {
                    fs::remove_dir_all(entry.path())?;
                    continue;
                }
                if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                let record = entry.path().join("job.json");
                if !record.exists() {
                    fs::remove_dir_all(entry.path())?;
                    continue;
                }
                let mut job: Job = serde_json::from_slice(&fs::read(&record)?)?;
                ensure!(job.id == id, "任务记录标识不一致");
                if !job.terminal() {
                    job.status = "interrupted".into();
                    job.message = "服务已重新启动，请重新提交任务。".into();
                    job.updated_at = now();
                    write_json(&record, &job)?;
                }
                Self::clean_media(&entry.path())?;
                jobs.insert(id, job);
            }
        }
        let service = Arc::new(Self {
            config: config.into(),
            directory,
            jobs: Mutex::new(jobs),
            admission: Mutex::new(()),
            active: RwLock::new(BTreeMap::new()),
            cancellations: Mutex::new(BTreeMap::new()),
            changed,
            wake: Notify::new(),
            shutdown,
        });
        let runner = service.clone();
        tokio::spawn(async move {
            runner.schedule().await;
        });
        Ok(service)
    }

    fn settings(&self) -> Result<Settings> {
        let settings = control::management(&control::load_value(&self.config)?)?
            .transcription
            .context("请先配置语音转写引擎")?;
        settings.validate()?;
        ensure!(
            control::resolve_path(&self.config, &settings.directory) == self.directory,
            "修改任务目录后需要重启 Router"
        );
        Ok(settings)
    }

    pub async fn runtime(&self, token: &HeaderValue) -> Result<Arc<RouterConfig>> {
        self.active
            .read()
            .await
            .get(token.to_str()?)
            .cloned()
            .context("转写任务已结束或任务标识无效")
    }

    fn announce(&self) {
        self.changed.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub async fn finish_shutdown(&self) -> Result<()> {
        while !self.cancellations.lock().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let mut jobs = self.jobs.lock().await;
        for job in jobs.values_mut().filter(|job| !job.terminal()) {
            job.status = "interrupted".into();
            job.message = "Router 服务停止，任务已中断".into();
            job.updated_at = now();
            Self::clean_media(&self.directory.join(&job.id))?;
            write_json(&self.directory.join(&job.id).join("job.json"), job)?;
        }
        Ok(())
    }

    async fn update(&self, id: &str, persist: bool, edit: impl FnOnce(&mut Job)) -> Result<()> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs.get_mut(id).context("转写任务不存在")?;
        edit(job);
        job.updated_at = now();
        if persist {
            write_json(&self.directory.join(id).join("job.json"), job)?;
        }
        drop(jobs);
        self.announce();
        Ok(())
    }

    fn clean_media(directory: &Path) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("input.")
                || ["engine.toml", "route.json", "execution.json"].contains(&name.as_str())
            {
                fs::remove_file(entry.path())?;
            } else if name == "tmp" || name == "source" {
                fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }

    async fn cleanup(&self, settings: &Settings) -> Result<()> {
        let mut jobs = self.jobs.lock().await;
        let expired: Vec<_> = jobs
            .values()
            .filter(|j| {
                j.terminal()
                    && now().saturating_sub(j.updated_at) / 86400 >= settings.retention_days
            })
            .map(|j| j.id.clone())
            .collect();
        for id in expired {
            fs::remove_dir_all(self.directory.join(&id))?;
            jobs.remove(&id);
        }
        Ok(())
    }

    async fn schedule(self: Arc<Self>) {
        let mut shutdown = self.shutdown.clone();
        loop {
            if *shutdown.borrow() {
                return;
            }
            let mut interval = 60;
            match self.settings() {
                Ok(settings) => {
                    interval = settings.cleanup_seconds;
                    if let Err(error) = self.cleanup(&settings).await {
                        eprintln!("转写清理失败：{error:#}");
                    }
                    let mut jobs = self.jobs.lock().await;
                    let count = jobs
                        .values()
                        .filter(|j| j.status == "running" || j.status == "cancelling")
                        .count();
                    let mut queued: Vec<_> = jobs
                        .values()
                        .filter(|j| j.status == "queued")
                        .map(|j| (j.created_at, j.id.clone()))
                        .collect();
                    queued.sort();
                    for (_, id) in queued
                        .into_iter()
                        .take(settings.max_concurrent_jobs.saturating_sub(count))
                    {
                        let job = jobs.get_mut(&id).expect("queued job");
                        job.status = "running".into();
                        if let Err(error) =
                            write_json(&self.directory.join(&id).join("job.json"), job)
                        {
                            job.status = "failed".into();
                            job.message = format!("{error:#}");
                            continue;
                        }
                        let (cancel, receiver) = watch::channel(false);
                        self.cancellations.lock().await.insert(id.clone(), cancel);
                        let service = self.clone();
                        tokio::spawn(async move {
                            let result = service.run(&id, receiver).await;
                            if let Err(error) = result {
                                let message = format!("{error:#}");
                                if let Err(error) = service
                                    .update(&id, true, |job| {
                                        job.status = "failed".into();
                                        job.message = message;
                                    })
                                    .await
                                {
                                    eprintln!("转写记录失败：{error:#}");
                                }
                            }
                            service.active.write().await.remove(&id);
                            service.cancellations.lock().await.remove(&id);
                            if let Err(error) = Self::clean_media(&service.directory.join(&id)) {
                                eprintln!("转写临时文件清理失败：{error:#}");
                            }
                            service.wake.notify_one();
                        });
                    }
                    drop(jobs);
                    self.announce();
                }
                Err(error) => {
                    if control::load_value(&self.config)
                        .is_ok_and(|v| !v["management"]["transcription"].is_null())
                    {
                        eprintln!("转写设置错误：{error:#}");
                    }
                }
            }
            tokio::select! {
                _ = self.wake.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {},
                _ = shutdown.changed() => return,
            }
        }
    }

    async fn run(&self, id: &str, mut cancel: watch::Receiver<bool>) -> Result<()> {
        let directory = self.directory.join(id);
        let settings: Settings =
            serde_json::from_slice(&fs::read(directory.join("execution.json"))?)?;
        let config = crate::load_router_runtime_config(&directory.join("route.json"))?;
        self.active
            .write()
            .await
            .insert(id.into(), Arc::new(config));
        let job = self.jobs.lock().await[id].clone();
        let input = directory.join("source").join(&job.name);
        let mut command = tokio::process::Command::new(&settings.program);
        command
            .args(["worker", "--input"])
            .arg(input)
            .arg("--output")
            .arg(directory.join("transcript.md"))
            .arg("--result")
            .arg(directory.join("result.json"))
            .arg("--cancel-on-stdin-eof")
            .arg("--job-id")
            .arg(id)
            .arg("--ffmpeg")
            .arg(&settings.ffmpeg)
            .arg("--ffprobe")
            .arg(&settings.ffprobe);
        if job.mode == "raw" {
            command.arg("--raw");
        }
        command.arg("--kind").arg(&job.kind);
        command
            .env("SPT_CONFIG_PATH", directory.join("engine.toml"))
            .env("SPT_STATE_DIR", directory.join("tmp/state"))
            .env_remove("OPENROUTER_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().context("无法启动 SPT，请检查引擎路径")?;
        let pid = child.id().context("SPT 没有进程 ID")?;
        let _stdin = child.stdin.take();
        let stdout = child.stdout.take().context("缺少工作进程事件通道")?;
        let mut stderr = child.stderr.take().context("缺少工作进程诊断通道")?;
        let diagnostics = tokio::spawn(async move {
            let mut tail = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stderr.read(&mut buffer).await?;
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..n]);
                if tail.len() > 32768 {
                    tail.drain(..tail.len() - 32768);
                }
            }
            Ok::<_, std::io::Error>(String::from_utf8_lossy(&tail).into_owned())
        });
        let events = async {
            let mut reader = BufReader::new(stdout);
            let mut completed = false;
            loop {
                let mut line = Vec::new();
                let n = (&mut reader)
                    .take(65537)
                    .read_until(b'\n', &mut line)
                    .await?;
                if n == 0 {
                    break;
                }
                ensure!(n <= 65536, "SPT 事件超过 64 KiB");
                let event: Value = serde_json::from_slice(&line).context("SPT 事件格式错误")?;
                ensure!(
                    event["version"] == 1 && event["job_id"] == id,
                    "SPT 事件版本或任务编号不符"
                );
                if event["event"] == "result" && event["status"] == "succeeded" {
                    completed = true;
                }
                self.update(id, false, |job| {
                    if let Some(stage) = event["phase"].as_str() {
                        job.stage = stage.chars().take(120).collect();
                    }
                    if let Some(n) = event["processed_ms"].as_u64() {
                        job.completed_ms = n;
                    }
                    if let Some(n) = event["total_ms"].as_u64() {
                        job.total_ms = n;
                    }
                    if let Some(message) = event["message"].as_str() {
                        job.message = message.chars().take(2000).collect();
                        if event["event"] == "advisory" && job.warnings.len() < 32 {
                            job.warnings.push(job.message.clone());
                        }
                    }
                })
                .await?;
            }
            Ok::<_, anyhow::Error>(completed)
        };
        tokio::pin!(events);
        let mut shutdown = self.shutdown.clone();
        enum End {
            Done(Result<bool>),
            Cancel,
            Shutdown,
            Timeout,
        }
        let end = tokio::select! {
            result = &mut events => End::Done(result),
            _ = async { if !*cancel.borrow() { let _ = cancel.changed().await; } } => End::Cancel,
            _ = async { if !*shutdown.borrow() { let _ = shutdown.changed().await; } } => End::Shutdown,
            _ = tokio::time::sleep(Duration::from_secs(settings.max_job_seconds)) => End::Timeout,
        };
        let stopping = !matches!(&end, End::Done(Ok(_)));
        if stopping {
            // 每个任务有独立进程组，取消同时覆盖媒体子进程。
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        let wait = async {
            if !matches!(&end, End::Done(_)) {
                let (_, status) = tokio::join!(&mut events, child.wait());
                status
            } else {
                child.wait().await
            }
        };
        let status =
            match tokio::time::timeout(Duration::from_secs(settings.cancel_grace_seconds), wait)
                .await
            {
                Ok(status) => status?,
                Err(_) => {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                    child.wait().await?
                }
            };
        let diagnostic = diagnostics.await??;
        let (state, message) = match end {
            End::Cancel => ("cancelled", "任务已取消".to_owned()),
            End::Shutdown => ("interrupted", "Router 服务停止，任务已中断".to_owned()),
            End::Timeout => ("failed", "任务超过配置的最长运行时间".to_owned()),
            End::Done(result) => {
                let completed = result?;
                let worker_message = self.jobs.lock().await[id].message.clone();
                ensure!(
                    status.success(),
                    "SPT 执行失败（{status}）：{worker_message}\n{}",
                    diagnostic
                        .chars()
                        .rev()
                        .take(2000)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                );
                ensure!(completed, "SPT 未报告完成事件：{worker_message}");
                for name in ["transcript.md", "result.json"] {
                    ensure!(
                        fs::metadata(directory.join(name))?.len() <= settings.max_result_bytes,
                        "转写结果超过配置大小上限"
                    );
                }
                ("succeeded", "转写完成".to_owned())
            }
        };
        self.update(id, true, |job| {
            job.status = state.into();
            job.message = message;
        })
        .await
    }
}

#[derive(Deserialize)]
struct Upload {
    name: String,
    model: String,
    mode: String,
    kind: String,
}

async fn create(
    State(state): State<ControlState>,
    Query(upload): Query<Upload>,
    body: Body,
) -> Result<Json<Job>, ApiError> {
    create_inner(state, upload, body).await.map_err(Into::into)
}

async fn create_inner(state: ControlState, upload: Upload, body: Body) -> Result<Json<Job>> {
    let service = state.transcription;
    let _admission = service.admission.lock().await;
    let mut settings = service.settings()?;
    ensure!(
        matches!(upload.mode.as_str(), "quality" | "raw")
            && matches!(upload.kind.as_str(), "audio" | "ocr"),
        "转写模式无效"
    );
    ensure!(
        upload.name.len() <= 512 && !upload.name.contains(['/', '\\', '\0']),
        "文件名无效"
    );
    let extension = Path::new(&upload.name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let allowed = if upload.kind == "ocr" {
        ["png", "jpg", "jpeg", "webp"].as_slice()
    } else {
        [
            "wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aiff", "aif", "mp4", "webm",
        ]
        .as_slice()
    };
    ensure!(
        allowed.contains(&extension.as_str()),
        "文件格式不在支持列表中"
    );
    let selected = settings
        .models
        .get(&upload.model)
        .context("请选择已配置的转写模型")?
        .clone();
    for path in [
        &mut settings.program,
        &mut settings.engine_config,
        &mut settings.ffmpeg,
        &mut settings.ffprobe,
    ] {
        *path = control::resolve_path(&service.config, path);
        ensure!(path.is_file(), "所需文件不存在：{}", path.display());
    }
    private_directory(&service.directory)?;
    service.cleanup(&settings).await?;
    ensure!(
        service.jobs.lock().await.len() < settings.max_jobs,
        "任务记录数量已达上限，请删除已完成任务或调整保留设置"
    );
    let used = disk_bytes(&service.directory)?;
    let reserve = (settings.max_temp_bytes_per_job + settings.max_result_bytes * 2)
        * settings.max_concurrent_jobs as u64;
    ensure!(
        used.saturating_add(reserve)
            .saturating_add(settings.max_upload_bytes)
            < settings.max_storage_bytes,
        "任务存储空间不足，请清理结果或调整上限"
    );
    let id = format!("{:032x}", rand::random::<u128>());
    let staging = tempfile::Builder::new()
        .prefix("upload-")
        .tempdir_in(&service.directory)?;
    private_directory(staging.path())?;
    let directory = service.directory.join(&id);
    private_directory(&staging.path().join("source"))?;
    let mut file =
        tokio::fs::File::create(staging.path().join("source").join(&upload.name)).await?;
    let mut stream = body.into_data_stream();
    let mut bytes = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        bytes += chunk.len() as u64;
        ensure!(
            bytes <= settings.max_upload_bytes,
            "上传文件超过配置的大小上限"
        );
        file.write_all(&chunk).await?;
    }
    ensure!(bytes > 0, "上传文件为空");
    file.flush().await?;
    drop(file);
    let mut route = control::load_value(&service.config)?;
    let destination = route["profiles"][&selected.profile]["standard"].clone();
    ensure!(destination.is_object(), "转写模型引用的路由不存在");
    let model = destination["model"]
        .as_str()
        .context("转写路由需要明确的 API 模型")?
        .to_owned();
    let target = destination["target"]
        .as_str()
        .context("转写后端不存在")?
        .to_owned();
    let model_label = route["management"]["labels"][format!("profile:{}", selected.profile)]
        .as_str()
        .unwrap_or(&model)
        .to_owned();
    let base = control::gateway_url(&route)? + "/v1";
    let dry_run_header = route["runtime"]["server"]["dry_run_header"].clone();
    route["management"] = Value::Null;
    route["profiles"] = json!({"transcription":{"standard":destination}});
    route["routes"] = json!([{"id":"transcription","match":{},"profile":"transcription"}]);
    route["validation_cases"] = json!([{"name":"transcription","context":"spt","model":model,"expected":{"rule_id":"transcription","target":target,"model":model,"provider":route["profiles"]["transcription"]["standard"]["provider"]}}]);
    route["runtime"]["clients"] = json!([{"id":"spt"}]);
    route["runtime"]["request_text_rewrites"] = json!([]);
    route["runtime"]["selection"]["speed_sources"] = json!([]);
    route["runtime"]["strip_request_headers"]
        .as_array_mut()
        .context("请求头策略缺失")?
        .push(json!(TASK_HEADER));
    // 任务快照使用原配置目录解析相对凭据和适配程序路径。
    let mut engine: toml::Value = toml::from_str(&fs::read_to_string(&settings.engine_config)?)?;
    let table = engine.as_table_mut().context("SPT 配置必须是 TOML 表")?;
    for (key, value) in &settings.engine_options {
        ensure!(
            table.get(key).is_some_and(toml::Value::is_integer),
            "引擎参数不是已有整数设置：{key}"
        );
        table.insert(key.clone(), toml::Value::Integer(i64::try_from(*value)?));
    }
    table.insert("router".into(), toml::Value::try_from(json!({"api_base":base,"workspace_directory":directory.join("tmp"),"model":model,"reported_model_prefix":selected.reported_model_prefix,"target":target,"dry_run_header":dry_run_header,"headers":{(TASK_HEADER):id},"transcription_prompt":settings.transcription_prompt}))?);
    table.insert(
        "max_temp_bytes".into(),
        toml::Value::Integer(settings.max_temp_bytes_per_job as i64),
    );
    table.insert(
        "max_transcript_bytes".into(),
        toml::Value::Integer(settings.max_result_bytes as i64),
    );
    let mut engine_file = tempfile::NamedTempFile::new_in(staging.path())?;
    engine_file.write_all(toml::to_string(&engine)?.as_bytes())?;
    engine_file
        .persist(staging.path().join("engine.toml"))
        .map_err(|e| e.error)?;
    // 快照保存在原配置目录下；相对路径需在写入前转换。
    absolutize_runtime(&mut route, &service.config)?;
    RouterConfig::from_json(&serde_json::to_vec(&route)?)?;
    write_json(&staging.path().join("route.json"), &route)?;
    write_json(&staging.path().join("execution.json"), &settings)?;
    let job = Job {
        id: id.clone(),
        name: upload.name,
        model: upload.model,
        mode: upload.mode,
        kind: upload.kind,
        status: "queued".into(),
        created_at: now(),
        updated_at: now(),
        stage: "queued".into(),
        completed_ms: 0,
        total_ms: 0,
        message: "等待处理".into(),
        warnings: Vec::new(),
        model_label,
        api_model: model,
        target,
    };
    write_json(&staging.path().join("job.json"), &job)?;
    fs::rename(staging.path(), &directory)?;
    service.jobs.lock().await.insert(id, job.clone());
    service.announce();
    service.wake.notify_one();
    Ok(Json(job))
}

fn absolutize_runtime(value: &mut Value, config: &Path) -> Result<()> {
    for backend in value["runtime"]["backends"]
        .as_object_mut()
        .context("后端配置不存在")?
        .values_mut()
    {
        if let Some(file) = backend.pointer_mut("/auth/token_file")
            && let Some(path) = file.as_str()
        {
            *file = json!(control::resolve_path(config, Path::new(path)));
        }
        if let Some(file) = backend.pointer_mut("/auth/token_command/0")
            && let Some(path) = file.as_str()
        {
            *file = json!(control::resolve_path(config, Path::new(path)));
        }
    }
    let directory = value
        .pointer_mut("/runtime/server/body_processing/spool_directory")
        .context("正文暂存目录不存在")?;
    *directory = json!(control::resolve_path(
        config,
        Path::new(directory.as_str().context("暂存目录无效")?)
    ));
    Ok(())
}

async fn list(State(state): State<ControlState>) -> Result<Json<Value>, ApiError> {
    let service = state.transcription;
    let settings = service.settings();
    let jobs: Vec<_> = service.jobs.lock().await.values().cloned().rev().collect();
    Ok(Json(
        json!({"jobs":jobs,"configured":settings.is_ok(),"error":settings.err().map(|e|format!("{e:#}"))}),
    ))
}

async fn action(
    State(state): State<ControlState>,
    PathParam(id): PathParam<String>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    action_inner(state, id, request).await.map_err(Into::into)
}

async fn action_inner(state: ControlState, id: String, request: Value) -> Result<Json<Value>> {
    let service = state.transcription;
    let _guard = if request["action"] == "delete" {
        Some(service.admission.lock().await)
    } else {
        None
    };
    let mut jobs = service.jobs.lock().await;
    let job = jobs.get_mut(&id).context("转写任务不存在")?;
    match request["action"].as_str() {
        Some("delete") => {
            ensure!(job.terminal(), "请先取消正在执行的任务");
            fs::remove_dir_all(service.directory.join(&id))?;
            jobs.remove(&id);
        }
        Some("cancel") => {
            ensure!(!job.terminal(), "任务已经结束");
            if job.status == "queued" {
                job.status = "cancelled".into();
                job.message = "任务已取消".into();
                Service::clean_media(&service.directory.join(&id))?;
            } else {
                job.status = "cancelling".into();
                if let Some(cancel) = service.cancellations.lock().await.get(&id) {
                    cancel.send_replace(true);
                }
            }
            job.updated_at = now();
            write_json(&service.directory.join(&id).join("job.json"), job)?;
        }
        _ => bail!("未知任务操作"),
    }
    drop(jobs);
    service.announce();
    Ok(Json(json!({"ok":true})))
}

async fn result(
    State(state): State<ControlState>,
    PathParam((id, format)): PathParam<(String, String)>,
) -> Result<Response, ApiError> {
    result_inner(state, id, format).await.map_err(Into::into)
}

async fn result_inner(state: ControlState, id: String, format: String) -> Result<Response> {
    let service = state.transcription;
    let jobs = service.jobs.lock().await;
    let job = jobs.get(&id).context("转写任务不存在")?;
    ensure!(job.status == "succeeded", "任务结果尚未就绪");
    let (name, mime) = match format.as_str() {
        "markdown" => ("transcript.md", "text/markdown; charset=utf-8"),
        "json" => ("result.json", "application/json"),
        _ => bail!("结果格式无效"),
    };
    let file = tokio::fs::File::open(service.directory.join(&id).join(name)).await?;
    drop(jobs);
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (
                header::CONTENT_DISPOSITION,
                if format == "json" {
                    "attachment; filename=result.json"
                } else {
                    "attachment; filename=transcript.md"
                },
            ),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}

async fn changes(
    State(state): State<ControlState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let stream = futures_util::stream::unfold(
        (
            state.transcription.changed.subscribe(),
            state.transcription.shutdown.clone(),
            true,
        ),
        |(mut receiver, mut shutdown, initial)| async move {
            if *shutdown.borrow() {
                return None;
            }
            if !initial {
                tokio::select! {result = receiver.changed() => {if result.is_err(){return None;}}, _=shutdown.changed()=>return None}
            }
            let revision = *receiver.borrow_and_update();
            Some((
                Ok(Event::default().event("tasks").data(revision.to_string())),
                (receiver, shutdown, false),
            ))
        },
    );
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

pub fn routes() -> Router<ControlState> {
    Router::new()
        .route(
            "/transcription/jobs",
            get(list).post(create).layer(DefaultBodyLimit::disable()),
        )
        .route("/transcription/jobs/{id}", post(action))
        .route("/transcription/jobs/{id}/{format}", get(result))
        .route("/transcription/changes", get(changes))
}
