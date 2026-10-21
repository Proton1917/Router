import React, { useEffect, useRef, useState } from "react";
import { fetchEventSource } from "@microsoft/fetch-event-source";
import defaults from "./transcription-defaults.json";
import "./transcription.css";

const statuses = {
  queued: "排队中",
  running: "正在处理",
  cancelling: "正在取消",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
  interrupted: "已中断",
};
const stages = {
  queued: "等待处理",
  preparing: "检查音频",
  canonicalizing: "转换音频",
  checking_route: "检查模型连接",
  transcribing: "转写正文",
  segmenting: "划分发言",
  speakers: "识别说话人",
  writing: "保存结果",
  recognizing: "识别文字",
};
const terminal = (job) =>
  ["succeeded", "failed", "cancelled", "interrupted"].includes(job.status);
const size = (bytes) => `${(bytes / 1048576).toFixed(1)} MiB`;
const time = (ms) =>
  `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
const auth = () => ({
  Authorization: `Bearer ${sessionStorage.getItem("router-token") || ""}`,
});

export function TranscriptionWorkspace({
  config,
  dirty,
  api,
  mutate,
  editing,
  ui: { Button, Field, Modal },
}) {
  const settings = config.management.transcription;
  const [jobs, setJobs] = useState([]);
  const [file, setFile] = useState(null);
  const [model, setModel] = useState(settings?.default_model || "");
  const [mode, setMode] = useState("quality");
  const [kind, setKind] = useState("audio");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [uploadProgress, setUploadProgress] = useState(0);
  const [selected, setSelected] = useState(null);
  const [preview, setPreview] = useState("");
  const [edit, setEdit] = useState(null);
  const [confirm, setConfirm] = useState(null);
  const input = useRef();
  const upload = useRef();
  const alive = useRef(true);

  async function refresh() {
    const result = await api("transcription/jobs");
    if (alive.current)
      setJobs(
        result.jobs.sort(
          (a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id),
        ),
      );
  }
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    editing(!!edit);
    return () => editing(false);
  }, [!!edit]);
  useEffect(() => {
    if (!settings?.models?.[model]) setModel(settings?.default_model || "");
  }, [settings]);
  useEffect(() => {
    const controller = new AbortController();
    let pending = Promise.resolve();
    fetchEventSource("/api/transcription/changes", {
      signal: controller.signal,
      headers: auth(),
      openWhenHidden: true,
      onopen(response) {
        if (!response.ok)
          throw Object.assign(new Error("无法读取转写任务"), {
            status: response.status,
          });
      },
      onmessage(event) {
        if (event.event === "tasks")
          pending = pending.then(refresh).catch((e) => setError(e.message));
      },
      onclose() {
        throw new Error("任务更新连接中断");
      },
      onerror(error) {
        if ([401, 403].includes(error.status)) throw error;
        return 1500;
      },
    }).catch((e) => {
      if (!controller.signal.aborted) setError(e.message);
    });
    return () => controller.abort();
  }, []);

  async function act(job, action) {
    setError("");
    try {
      await api(`transcription/jobs/${job.id}`, { action });
      await refresh();
      if (action === "delete" && selected === job.id) {
        setSelected(null);
        setPreview("");
      }
    } catch (e) {
      setError(e.message);
    }
  }
  async function result(job, download = false, format = "markdown") {
    setError("");
    try {
      const response = await fetch(
        `/api/transcription/jobs/${job.id}/${download ? format : "json"}`,
        { headers: auth() },
      );
      if (!response.ok) throw new Error((await response.json()).error);
      if (download) {
        const url = URL.createObjectURL(await response.blob());
        const link = document.createElement("a");
        link.href = url;
        link.download =
          job.name.replace(/\.[^.]*$/, "") +
          (format === "json" ? ".json" : ".md");
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
      } else {
        const data = await response.json();
        if (alive.current) {
          setPreview(data);
          setSelected(job.id);
        }
      }
    } catch (e) {
      setError(e.message);
    }
  }
  function choose(next) {
    if (next && settings && next.size > settings.max_upload_bytes) {
      setError(`文件超过 ${size(settings.max_upload_bytes)} 的上传上限。`);
      return;
    }
    setFile(next);
    setError("");
  }
  async function start() {
    if (!file || !model || !settings || busy) return;
    setBusy(true);
    setError("");
    setUploadProgress(0);
    try {
      const job = await new Promise((resolve, reject) => {
        const xhr = new XMLHttpRequest();
        upload.current = xhr;
        xhr.open(
          "POST",
          `/api/transcription/jobs?${new URLSearchParams({ name: file.name, model, mode, kind })}`,
        );
        xhr.setRequestHeader("Authorization", auth().Authorization);
        xhr.setRequestHeader("Content-Type", "application/octet-stream");
        xhr.upload.onprogress = (e) => {
          if (e.lengthComputable && alive.current)
            setUploadProgress(Math.round((e.loaded / e.total) * 100));
        };
        xhr.onerror = () => reject(new Error("上传中断，请检查 Router 服务。"));
        xhr.onabort = () => reject(new Error("上传已取消。"));
        xhr.onload = () => {
          try {
            const data = JSON.parse(xhr.responseText);
            if (xhr.status >= 200 && xhr.status < 300) resolve(data);
            else reject(new Error(data.error || "上传失败"));
          } catch {
            reject(new Error("服务返回了无法识别的上传结果。"));
          }
        };
        xhr.send(file);
      });
      if (alive.current) {
        setSelected(job.id);
        setPreview("");
        setFile(null);
        await refresh();
      }
    } catch (e) {
      if (alive.current) setError(e.message);
    } finally {
      upload.current = null;
      if (alive.current) setBusy(false);
    }
  }
  const chosen = jobs.find((job) => job.id === selected);
  const label = (id) => {
    const profile = settings?.models?.[id]?.profile;
    return config.management.labels?.[`profile:${profile}`] || profile || id;
  };
  const destination =
    config.profiles[settings?.models?.[model]?.profile]?.standard;
  function update(path, value) {
    setEdit((previous) => ({ ...previous, [path]: value }));
  }
  function addModel() {
    const profile = Object.keys(config.profiles).find(
      (key) => config.profiles[key].standard?.model,
    );
    if (!profile) return;
    let index = 1;
    while (edit.models[`model-${index}`]) index++;
    const id = `model-${index}`;
    setEdit((old) => ({
      ...old,
      default_model: old.default_model || id,
      models: {
        ...old.models,
        [id]: {
          profile,
          reported_model_prefix: config.profiles[profile].standard.model,
        },
      },
    }));
  }
  return (
    <div className="transcription-workspace">
      <div className="spt-intro">
        <div>
          <span className="eyebrow">AUDIO TO TEXT</span>
          <h2>把录音变成文字稿</h2>
          <p>
            选择模型，保留时间范围与说话人。任务由后台执行，关闭窗口后继续处理。
          </p>
        </div>
        <Button onClick={() => setEdit(structuredClone(settings || defaults))}>
          转写设置
        </Button>
      </div>
      {error && (
        <div className="spt-error" role="alert">
          {error}
          <button aria-label="关闭转写提示" onClick={() => setError("")}>
            ×
          </button>
        </div>
      )}
      {!settings && (
        <div className="callout">
          先打开转写设置，配置引擎与可用模型。依赖：SPT、FFmpeg、FFprobe。
        </div>
      )}
      <section className="spt-composer">
        <div
          className={`spt-drop ${file ? "has-file" : ""}`}
          onDragOver={(e) => {
            e.preventDefault();
            e.dataTransfer.dropEffect = "copy";
          }}
          onDrop={(e) => {
            e.preventDefault();
            choose(e.dataTransfer.files[0]);
          }}
        >
          <input
            ref={input}
            type="file"
            aria-label="选择转写文件"
            accept={
              kind === "ocr"
                ? ".png,.jpg,.jpeg,.webp"
                : ".wav,.mp3,.m4a,.aac,.flac,.ogg,.opus,.aiff,.aif,.mp4,.webm"
            }
            onChange={(e) => choose(e.target.files[0])}
          />
          <span className="spt-file-icon" aria-hidden="true">
            {kind === "audio" ? "♫" : "▤"}
          </span>
          <strong>{file ? file.name : "将文件拖到这里"}</strong>
          <span>
            {file
              ? size(file.size)
              : kind === "audio"
                ? "M4A、MP3、WAV、FLAC 等音频"
                : "PNG、JPEG、WebP 图片"}
          </span>
          <Button disabled={busy} onClick={() => input.current.click()}>
            {file ? "更换文件" : "选择文件"}
          </Button>
        </div>
        <div className="spt-controls">
          <div className="form-grid">
            <Field label="处理类型">
              <select
                value={kind}
                disabled={busy}
                onChange={(e) => {
                  setKind(e.target.value);
                  setFile(null);
                }}
              >
                <option value="audio">语音转写</option>
                <option value="ocr">图片文字识别</option>
              </select>
            </Field>
            <Field
              label="转写模型"
              hint={
                destination
                  ? `${destination.target}${destination.provider ? " · " + destination.provider : ""}`
                  : "在转写设置中添加支持音频或图片的模型。"
              }
            >
              <select
                value={model}
                disabled={busy}
                onChange={(e) => setModel(e.target.value)}
              >
                <option value="">选择模型</option>
                {Object.keys(settings?.models || {}).map((id) => (
                  <option key={id} value={id}>
                    {label(id)}
                  </option>
                ))}
              </select>
            </Field>
            {kind === "audio" && (
              <Field label="文字处理">
                <select
                  value={mode}
                  disabled={busy}
                  onChange={(e) => setMode(e.target.value)}
                >
                  <option value="quality">整理标点 · 保留原话</option>
                  <option value="raw">原始转写</option>
                </select>
              </Field>
            )}
          </div>
          <div className="spt-start">
            <small>
              {dirty
                ? "请先应用配置修改，再启动任务。"
                : settings
                  ? `单文件 ≤ ${size(settings.max_upload_bytes)} · 同时处理 ${settings.max_concurrent_jobs} 个任务`
                  : "配置引擎后即可使用"}
            </small>
            <Button
              primary
              disabled={!settings || !file || !model || dirty || busy}
              onClick={start}
            >
              {busy ? `上传 ${uploadProgress}%` : "开始处理"}
            </Button>
            {busy && (
              <Button onClick={() => upload.current?.abort()}>取消上传</Button>
            )}
          </div>
        </div>
      </section>
      <div className="spt-history-heading">
        <h2>
          任务记录 <span>{jobs.length}</span>
        </h2>
        <small>
          任务结束后清理上传文件与临时音频
          {settings ? `，结果保留 ${settings.retention_days} 天` : ""}
        </small>
      </div>
      <div className={`spt-results ${chosen ? "with-preview" : ""}`}>
        <div className="spt-jobs">
          {!jobs.length && (
            <div className="spt-empty">尚无任务。选择一段录音开始转写。</div>
          )}
          {jobs.map((job) => (
            <article
              className={`spt-job ${selected === job.id ? "selected" : ""}`}
              key={job.id}
            >
              <div className="spt-job-title">
                <button
                  onClick={() => {
                    setSelected(job.id);
                    setPreview("");
                    if (job.status === "succeeded") result(job);
                  }}
                >
                  {job.name}
                </button>
                <span className={`spt-state ${job.status}`}>
                  {statuses[job.status] || job.status}
                </span>
              </div>
              <div className="spt-job-meta">
                <span>{job.model_label || label(job.model)}</span>
                <time>{new Date(job.created_at * 1000).toLocaleString()}</time>
              </div>
              {!terminal(job) && (
                <>
                  <progress
                    aria-label={`${job.name} 处理进度`}
                    max={job.total_ms || 1}
                    value={
                      job.total_ms
                        ? Math.min(job.completed_ms, job.total_ms)
                        : undefined
                    }
                  />
                  <div className="spt-job-meta">
                    <span>{stages[job.stage] || job.stage}</span>
                    <span>
                      {job.total_ms
                        ? `${time(job.completed_ms)} / ${time(job.total_ms)}`
                        : "等待阶段结果"}
                    </span>
                  </div>
                </>
              )}
              <p className={job.status === "failed" ? "spt-failure" : ""}>
                {job.message}
              </p>
              <footer>
                {job.status === "succeeded" && (
                  <>
                    <Button onClick={() => result(job)}>预览文字稿</Button>
                    <Button onClick={() => result(job, true)}>
                      导出 Markdown
                    </Button>
                  </>
                )}
                {!terminal(job) ? (
                  <Button
                    disabled={job.status === "cancelling"}
                    onClick={() => act(job, "cancel")}
                  >
                    取消任务
                  </Button>
                ) : (
                  <button className="spt-link" onClick={() => setConfirm(job)}>
                    删除记录
                  </button>
                )}
              </footer>
            </article>
          ))}
        </div>
        {chosen && (
          <aside className="spt-preview">
            <header>
              <div>
                <span className="eyebrow">TRANSCRIPT</span>
                <h3>{chosen.name}</h3>
              </div>
              <button
                className="icon-button"
                aria-label="关闭文字稿"
                onClick={() => {
                  setSelected(null);
                  setPreview("");
                }}
              >
                ×
              </button>
            </header>
            {chosen.warnings?.length > 0 && (
              <div className="callout">{chosen.warnings.join("；")}</div>
            )}
            {preview ? (
              <>
                <div className="spt-transcript">
                  {preview.kind === "ocr" ? (
                    <p>{preview.text}</p>
                  ) : (
                    preview.segments?.map((segment, index) => (
                      <section key={index}>
                        <div className="spt-job-meta">
                          {time(segment.start_ms)}–{time(segment.end_ms)}
                        </div>
                        {segment.turns.map((turn, i) => (
                          <p key={i}>
                            <strong>{turn.speaker_id}</strong>
                            {turn.text}
                          </p>
                        ))}
                      </section>
                    ))
                  )}
                </div>
                <details className="advanced">
                  <summary>模型与用量</summary>
                  <p>
                    {preview.models_reported?.join("、")} ·{" "}
                    {preview.router_target}
                  </p>
                  <p>
                    输入 {preview.usage?.reported_prompt_tokens} tokens · 输出{" "}
                    {preview.usage?.reported_completion_tokens} tokens
                  </p>
                  <p>
                    {preview.usage?.reported_cost_usd == null
                      ? "后端未报告费用"
                      : `费用 $${preview.usage.reported_cost_usd}`}
                  </p>
                </details>
              </>
            ) : (
              <p>
                {chosen.status === "succeeded"
                  ? "点击预览文字稿加载结果。"
                  : chosen.message}
              </p>
            )}
            {chosen.status === "succeeded" && (
              <Button onClick={() => result(chosen, true, "json")}>
                导出结构化结果
              </Button>
            )}
          </aside>
        )}
      </div>
      {confirm && (
        <Modal
          title="删除转写任务"
          close={() => setConfirm(null)}
          footer={
            <>
              <Button onClick={() => setConfirm(null)}>保留</Button>
              <Button
                danger
                onClick={() => {
                  act(confirm, "delete");
                  setConfirm(null);
                }}
              >
                删除任务和结果
              </Button>
            </>
          }
        >
          <p>
            将删除“{confirm.name}”的任务记录与本机转写结果。已导出的文件保留。
          </p>
        </Modal>
      )}
      {edit && (
        <Modal
          title="转写设置"
          wide
          close={() => setEdit(null)}
          footer={
            <>
              <span className="footer-note">
                保存草稿后，点击页面底部应用修改。
              </span>
              <Button onClick={() => setEdit(null)}>取消</Button>
              <Button
                primary
                onClick={() => {
                  mutate((config) => {
                    config.management.transcription = edit;
                  });
                  setEdit(null);
                }}
              >
                保存草稿
              </Button>
            </>
          }
        >
          <section className="form-section">
            <h3>可用模型</h3>
            <p className="muted">
              选择已在 Router
              中配置的模型。语音转写需要音频输入和结构化输出能力。
            </p>
            {Object.entries(edit.models).map(([id, item]) => (
              <div className="spt-model-row" key={id}>
                <Field label="模型配置">
                  <select
                    value={item.profile}
                    onChange={(e) =>
                      setEdit((old) => ({
                        ...old,
                        models: {
                          ...old.models,
                          [id]: {
                            profile: e.target.value,
                            reported_model_prefix:
                              config.profiles[e.target.value].standard.model,
                          },
                        },
                      }))
                    }
                  >
                    {Object.entries(config.profiles)
                      .filter(([, v]) => v.standard.model)
                      .map(([key]) => (
                        <option key={key} value={key}>
                          {config.management.labels?.[`profile:${key}`] || key}
                        </option>
                      ))}
                  </select>
                </Field>
                <Field
                  label="响应模型标识"
                  hint="须与后端实际返回的模型名一致，可填写版本前缀。"
                >
                  <input
                    value={item.reported_model_prefix}
                    onChange={(e) =>
                      setEdit((old) => ({
                        ...old,
                        models: {
                          ...old.models,
                          [id]: {
                            ...item,
                            reported_model_prefix: e.target.value,
                          },
                        },
                      }))
                    }
                  />
                </Field>
                <Button
                  onClick={() =>
                    setEdit((old) => {
                      const models = { ...old.models };
                      delete models[id];
                      return {
                        ...old,
                        models,
                        default_model:
                          old.default_model === id
                            ? Object.keys(models)[0] || ""
                            : old.default_model,
                      };
                    })
                  }
                >
                  移除
                </Button>
              </div>
            ))}
            <Button onClick={addModel}>添加模型</Button>
            <Field label="默认模型">
              <select
                value={edit.default_model}
                onChange={(e) => update("default_model", e.target.value)}
              >
                <option value="">选择默认模型</option>
                {Object.entries(edit.models).map(([id, item]) => (
                  <option key={id} value={id}>
                    {config.management.labels?.[`profile:${item.profile}`] ||
                      item.profile}
                  </option>
                ))}
              </select>
            </Field>
          </section>
          <section className="form-section">
            <h3>处理与保留</h3>
            <div className="form-grid">
              {[
                ["max_upload_bytes", "单文件上限（MiB）", 1048576],
                ["max_storage_bytes", "总存储上限（MiB）", 1048576],
                ["max_temp_bytes_per_job", "单任务临时空间（MiB）", 1048576],
                ["max_concurrent_jobs", "同时运行任务数", 1],
                ["max_jobs", "最多保留任务数", 1],
                ["retention_days", "结果保留天数", 1],
              ].map(([key, label, scale]) => (
                <Field label={label} key={key}>
                  <input
                    type="number"
                    min="1"
                    value={edit[key] / scale}
                    onChange={(e) =>
                      update(key, Number(e.target.value) * scale)
                    }
                  />
                </Field>
              ))}
            </div>
            <Field label="转写提示词">
              <textarea
                rows={4}
                value={edit.transcription_prompt}
                onChange={(e) => update("transcription_prompt", e.target.value)}
              />
            </Field>
          </section>
          <section className="form-section">
            <h3>分段与输出</h3>
            <div className="form-grid">
              {[
                ["chunk_seconds", "音频片段秒数"],
                ["max_output_tokens", "单次输出 tokens 上限"],
                ["split_output_tokens", "触发继续分段的 tokens"],
                ["retries", "请求最多重试次数"],
              ].map(([key, label]) => (
                <Field label={label} key={key}>
                  <input
                    type="number"
                    min={key === "retries" ? 0 : 1}
                    value={
                      edit.engine_options?.[key] ?? defaults.engine_options[key]
                    }
                    onChange={(e) =>
                      update("engine_options", {
                        ...edit.engine_options,
                        [key]: Number(e.target.value),
                      })
                    }
                  />
                </Field>
              ))}
            </div>
          </section>
          <details className="advanced">
            <summary>引擎、依赖与任务目录</summary>
            <div className="form-grid">
              {[
                ["program", "SPT 可执行文件"],
                ["engine_config", "SPT 引擎配置文件"],
                ["ffmpeg", "FFmpeg 可执行文件"],
                ["ffprobe", "FFprobe 可执行文件"],
                ["directory", "任务存储目录（修改后重启服务）"],
              ].map(([key, label]) => (
                <Field key={key} label={label}>
                  <input
                    value={edit[key]}
                    onChange={(e) => update(key, e.target.value)}
                  />
                </Field>
              ))}
            </div>
            <p className="muted">
              依赖缺失时先安装 SPT 和 FFmpeg，再填写其路径。相对路径以 Router
              配置所在目录为基准。
            </p>
          </details>
        </Modal>
      )}
    </div>
  );
}
