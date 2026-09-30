use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use clap::{CommandFactory, Parser};
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;

mod body;
mod cli;
mod control;
mod integrations;
mod prepare;
mod routing;
mod runtime;
mod server;

use server::{Arguments, Protocol, ServerConfig};

use routing::{ResolvedRoute, RouterConfig};
use runtime::{AuthConfig, BackendConfig, ClientConfig, CorsPolicy, SettingsSource};
#[cfg(test)]
use runtime::{PayloadNormalization, ToolDefaults, WebSearchPolicy};

#[derive(Clone)]
struct AppState {
    client: Client,
    runtime_routing_config: Arc<PathBuf>,
    server: Arc<ServerConfig>,
    processing: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ClientSettings {
    fast_mode: bool,
    model: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    if arguments.command.is_none() && !arguments.check {
        Arguments::command().print_help()?;
        println!();
        return Ok(());
    }
    let config_path = fs::canonicalize(
        arguments
            .config
            .context("请使用 --config 或 ROUTER_CONFIG 指定配置文件")?,
    )
    .context("configuration file is unavailable")?;
    if let Some(command) = arguments.command
        && !matches!(command, server::RouterCommand::Serve)
    {
        return cli::dispatch(&config_path, command).await;
    }
    let config = load_router_runtime_config(&config_path)?;
    let server = config
        .runtime
        .server
        .context("runtime.server is required")?;
    if arguments.check {
        println!("configuration valid");
        return Ok(());
    }
    let addr = arguments.listen.unwrap_or(server.listen);
    fs::create_dir_all(&server.body_processing.spool_directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            &server.body_processing.spool_directory,
            fs::Permissions::from_mode(0o700),
        )?;
    }
    let state = AppState {
        client: Client::builder()
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(server.connect_timeout_ms))
            .timeout(Duration::from_millis(server.request_timeout_ms))
            .build()
            .context("failed to build HTTP client")?,
        runtime_routing_config: Arc::new(config_path),
        processing: Arc::new(tokio::sync::Semaphore::new(
            server.body_processing.max_concurrent_requests,
        )),
        server: Arc::new(server),
    };

    let app = Router::new()
        .route(&state.server.health_path, get(health))
        .fallback(proxy)
        .with_state(state);

    eprintln!("router listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn health(
    State(state): State<AppState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if query.get("format").is_some_and(|value| value == "json") {
        return axum::Json(serde_json::json!({"service":"router","version":env!("CARGO_PKG_VERSION"),"config_id":blake3::hash(state.runtime_routing_config.to_string_lossy().as_bytes()).to_hex().to_string()})).into_response();
    }
    "ok".into_response()
}

async fn proxy(State(state): State<AppState>, req: Request) -> Response {
    let config = load_router_runtime_config(state.runtime_routing_config.as_ref());
    let result = match &config {
        Ok(config) => proxy_inner(state, req, config).await,
        Err(error) => Err(anyhow::anyhow!("{error:#}")),
    };
    let mut response = match result {
        Ok(response) => response,
        Err(error) => {
            eprintln!("router error: {error:#}");
            (
                StatusCode::BAD_GATEWAY,
                format!("router upstream error: {error:#}"),
            )
                .into_response()
        }
    };
    if let Ok(config) = config {
        apply_cors_headers(&mut response, &config.runtime.cors);
    }
    response
}

async fn proxy_inner(
    state: AppState,
    req: Request,
    runtime_config: &RouterConfig,
) -> Result<Response> {
    let (parts, body) = req.into_parts();
    let method = parts.method;
    let uri = parts.uri;
    let headers = parts.headers;
    let server = runtime_config
        .runtime
        .server
        .as_ref()
        .context("runtime.server is required")?;
    if server.listen != state.server.listen
        || server.health_path != state.server.health_path
        || server.connect_timeout_ms != state.server.connect_timeout_ms
        || server.request_timeout_ms != state.server.request_timeout_ms
        || server.body_processing != state.server.body_processing
    {
        bail!("listener or transport configuration changed; restart the router to apply it");
    }
    let protocol = server
        .protocol_for(uri.path())
        .context("request path is not configured in server.endpoints")?;
    let messages_protocol = protocol == Protocol::Messages;
    let client = runtime_config.runtime.client_for(&headers, uri.path())?;
    let word_gateway = client.word_gateway;
    if method == Method::OPTIONS {
        return cors_preflight_response(&headers, &runtime_config.runtime.cors);
    }

    if word_gateway && method == Method::GET && protocol == Protocol::Models {
        eprintln!("{method} {uri} word=true -> local models");
        return word_models_response(runtime_config);
    }
    let client_settings = configured_client_settings(client, &headers);
    if !word_gateway
        && !is_dry_run(&headers, server)
        && protocol != Protocol::Messages
        && protocol != Protocol::Models
        && let Some(route) =
            transparent_route(runtime_config, &client.id, &headers, &client_settings)?
    {
        return forward_streaming(
            &state,
            body,
            &method,
            &uri,
            &headers,
            runtime_config,
            &route,
        )
        .await;
    }
    let permit = state.processing.clone().acquire_owned().await?;
    let incoming =
        body::JsonBody::receive(body, &server.body_processing, server.max_request_bytes).await?;
    let config = runtime_config.clone();
    let context = client.id.clone();
    let request_headers = headers.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        prepare::prepare(
            incoming,
            &config,
            &context,
            &request_headers,
            &client_settings,
            word_gateway,
            protocol,
        )
    })
    .await
    .context("request processing task failed")??;
    let resolved_route = &prepared.route;
    let requested_stream = prepared.requested_stream;
    let word_stream_response = word_gateway && messages_protocol && requested_stream;
    let target = resolved_route.target.as_str();
    let backend = runtime_config
        .runtime
        .backends
        .get(target)
        .context("resolved backend is missing")?;
    if !messages_protocol
        && protocol != Protocol::Models
        && protocol != Protocol::Passthrough
        && (word_gateway || resolved_route.force_nonstream)
    {
        bail!("selected adapter is incompatible with the configured endpoint protocol");
    }
    let openrouter_web_search_bridge = prepared.search_tools > 0;
    let openrouter_web_search_model =
        openrouter_web_search_bridge.then(|| resolved_route.model.clone().unwrap_or_default());
    let nonstream_upstream_response =
        !word_gateway && messages_protocol && requested_stream && !prepared.outgoing_stream;
    let filter_openrouter_done =
        backend.filter_sse_done && messages_protocol && prepared.outgoing_stream;
    let upstream_url = upstream_url(backend, &uri);
    let response_cache_ttl = (!openrouter_web_search_bridge
        && backend
            .cache
            .as_ref()
            .is_some_and(|cache| cache.paths.iter().any(|path| path == uri.path())))
    .then_some(resolved_route.response_cache_ttl_seconds)
    .flatten();
    eprintln!(
        "{method} {uri} rule={} model={} outgoing={} bytes={} outgoing_bytes={} stream={} -> {target}",
        resolved_route.rule_id,
        prepared.original_model,
        resolved_route.model.as_deref().unwrap_or_default(),
        prepared.original_bytes,
        prepared.body.len,
        prepared.outgoing_stream
    );

    if is_dry_run(&headers, server) {
        return dry_run_response(resolved_route, backend, &uri, &prepared);
    }

    let mut builder = state.client.request(method.clone(), upstream_url);
    builder = copy_request_headers(
        builder,
        &headers,
        resolved_route,
        &runtime_config.runtime.strip_request_headers,
    );
    if let Some(auth) = &backend.auth {
        builder = apply_backend_auth(builder, auth).await?;
    }
    for (name, value) in &resolved_route.header_policy.set {
        builder = builder.header(name, value);
    }
    if let (Some(ttl), Some(cache)) = (response_cache_ttl, &backend.cache) {
        builder = builder
            .header(&cache.request_enable_header, &cache.request_enable_value)
            .header(&cache.request_ttl_header, ttl.to_string());
    }

    let upstream = builder
        .header("content-length", prepared.body.len)
        .body(prepared.body.into_http_body().await?)
        .send()
        .await
        .context("failed to send upstream request")?;

    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    // A streaming request can receive a JSON error. Only successful SSE
    // responses may enter the event filter; otherwise it buffers away the body.
    let filter_openrouter_done =
        filter_openrouter_done && is_successful_event_stream(status, &upstream_headers);
    log_upstream_response(
        target,
        backend,
        word_gateway,
        &method,
        &uri,
        status,
        &upstream_headers,
    );
    if backend.rewrite_model_catalog && protocol == Protocol::Models {
        let body = upstream
            .bytes()
            .await
            .context("failed to read models response")?;
        return rewritten_models_response(status, &upstream_headers, &body, runtime_config);
    }
    if openrouter_web_search_bridge {
        let mut body = Vec::new();
        let is_event_stream = upstream_headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
        let mut upstream_stream = upstream.bytes_stream();
        while let Some(chunk) = upstream_stream.next().await {
            let chunk = chunk.context("failed to read OpenRouter web search response")?;
            body.extend_from_slice(&chunk);
            if is_event_stream && sse_message_is_complete(&body) {
                break;
            }
        }
        let body = Bytes::from(body);
        eprintln!(
            "OpenRouter web search response: model={} complete={} bytes={}",
            openrouter_web_search_model.as_deref().unwrap_or("-"),
            !is_event_stream || sse_message_is_complete(&body),
            body.len()
        );
        return rewritten_openrouter_web_search_response(status, &upstream_headers, &body);
    }
    if word_gateway && messages_protocol || nonstream_upstream_response {
        let body = upstream
            .bytes()
            .await
            .context("failed to read message response")?;
        return rewritten_word_message_response(
            status,
            &upstream_headers,
            &body,
            word_stream_response || nonstream_upstream_response,
            resolved_route
                .model
                .as_deref()
                .unwrap_or(&runtime_config.word_gateway.probe_model),
        );
    }

    let upstream_stream = upstream.bytes_stream();
    let stream = upstream_stream
        .scan(Vec::<u8>::new(), move |buffer, chunk| {
            let filter_openrouter_done = filter_openrouter_done;
            let next = match chunk {
                Ok(chunk) if filter_openrouter_done => {
                    buffer.extend_from_slice(&chunk);
                    let filtered = drain_complete_sse_events(buffer);
                    Ok((!filtered.is_empty()).then(|| Bytes::from(filtered)))
                }
                Ok(chunk) => Ok(Some(chunk)),
                Err(error) => Err(std::io::Error::other(error)),
            };
            futures_util::future::ready(Some(next))
        })
        .filter_map(|chunk| {
            futures_util::future::ready(match chunk {
                Ok(Some(chunk)) => Some(Ok(chunk)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
        });

    let mut response = Response::builder().status(status);
    for (name, value) in upstream_headers.iter() {
        if should_forward_response_header(name) {
            response = response.header(name, value);
        }
    }

    response
        .body(Body::from_stream(stream))
        .context("failed to build response")
}

async fn apply_backend_auth(
    mut builder: reqwest::RequestBuilder,
    auth: &AuthConfig,
) -> Result<reqwest::RequestBuilder> {
    let token = if let Some(name) = &auth.token_env {
        env::var(name)
            .with_context(|| format!("credential environment variable {name} is not set"))?
    } else if let Some(path) = &auth.token_file {
        fs::read_to_string(path).context("failed to read configured credential file")?
    } else {
        let output = tokio::process::Command::new(&auth.token_command[0])
            .args(&auth.token_command[1..])
            .output()
            .await
            .context("credential helper failed to start")?;
        if !output.status.success() {
            bail!("credential helper exited with {}", output.status);
        }
        String::from_utf8(output.stdout).context("credential helper returned non-UTF-8 output")?
    };
    let token = token.trim();
    if token.is_empty() {
        bail!("configured credential is empty");
    }
    for (name, template) in &auth.headers {
        builder = builder.header(name, template.replace("{token}", token));
    }
    Ok(builder)
}

fn transparent_route(
    config: &RouterConfig,
    context: &str,
    headers: &HeaderMap,
    settings: &ClientSettings,
) -> Result<Option<ResolvedRoute>> {
    for rule in &config.routes {
        let matcher = &rule.matcher;
        if !matcher.contexts.is_empty() && !matcher.contexts.iter().any(|id| id == context)
            || !matcher
                .headers
                .iter()
                .all(|predicate| predicate.matches(headers))
            || matcher
                .absent_headers
                .iter()
                .any(|name| headers.contains_key(name))
        {
            continue;
        }
        if !matcher.models.is_empty()
            || !matcher.model_prefixes.is_empty()
            || !matcher.model_contains.is_empty()
            || matcher.stream.is_some()
            || !matcher.last_user_contains_all.is_empty()
        {
            return Ok(None);
        }
        if !matcher.settings_models.is_empty()
            && !settings.model.as_ref().is_some_and(|model| {
                matcher
                    .settings_models
                    .iter()
                    .any(|expected| config.runtime.selection.model_ids_match(expected, model))
            })
        {
            continue;
        }
        let profile = &config.profiles[&rule.profile];
        let destination = &profile.standard;
        if profile.fast.is_some()
            || profile.provider_override.is_some()
            || destination.model.is_some()
            || destination.model_template.is_some()
            || destination.provider.is_some()
            || destination.reasoning_adapter.is_some()
            || destination.force_nonstream
            || destination.response_cache_ttl_seconds.is_some()
            || destination.payload_normalization != runtime::PayloadNormalization::default()
            || destination.field_policy != routing::FieldPolicy::default()
        {
            return Ok(None);
        }
        return config
            .resolve(
                context,
                headers,
                &Value::Null,
                settings.fast_mode,
                settings.model.as_deref(),
            )
            .map(Some);
    }
    Ok(None)
}

async fn forward_streaming(
    state: &AppState,
    body: Body,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    config: &RouterConfig,
    route: &ResolvedRoute,
) -> Result<Response> {
    let backend = &config.runtime.backends[&route.target];
    let server = config
        .runtime
        .server
        .as_ref()
        .context("runtime.server is required")?;
    let max = server.max_request_bytes;
    let permit = state.processing.clone().acquire_owned().await?;
    let stream = body.into_data_stream().scan(0usize, move |total, chunk| {
        let _ = &permit;
        let chunk = chunk.map_err(std::io::Error::other).and_then(|chunk| {
            *total = total.saturating_add(chunk.len());
            if *total > max {
                Err(std::io::Error::other(
                    "request exceeds configured byte limit",
                ))
            } else {
                Ok(chunk)
            }
        });
        futures_util::future::ready(Some(chunk))
    });
    let mut builder = copy_request_headers(
        state
            .client
            .request(method.clone(), upstream_url(backend, uri)),
        headers,
        route,
        &config.runtime.strip_request_headers,
    );
    if let Some(auth) = &backend.auth {
        builder = apply_backend_auth(builder, auth).await?;
    }
    for (name, value) in &route.header_policy.set {
        builder = builder.header(name, value);
    }
    let upstream = builder
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await?;
    eprintln!(
        "{method} {} rule={} transfer=streaming -> {} status={}",
        uri.path(),
        route.rule_id,
        route.target,
        upstream.status()
    );
    let mut response = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers() {
        if should_forward_response_header(name) {
            response = response.header(name, value);
        }
    }
    Ok(response.body(Body::from_stream(upstream.bytes_stream()))?)
}

fn cors_preflight_response(headers: &HeaderMap, policy: &CorsPolicy) -> Result<Response> {
    let allow_headers = headers
        .get("access-control-request-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(&policy.allow_headers);
    let origin = if policy.reflect_request_origin {
        headers
            .get("origin")
            .and_then(|value| value.to_str().ok())
            .unwrap_or(&policy.allow_origin)
    } else {
        &policy.allow_origin
    };
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("access-control-allow-origin", origin)
        .header("access-control-allow-headers", allow_headers)
        .body(Body::empty())
        .context("failed to build CORS preflight response")
}

#[cfg(test)]
fn apply_resolved_route(
    body: &[u8],
    route: &ResolvedRoute,
    word_gateway: bool,
    config: &RouterConfig,
) -> Result<Vec<u8>> {
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else {
        return Ok(body.to_vec());
    };
    let requested_stream = json.get("stream").and_then(Value::as_bool).unwrap_or(false);

    route.apply_reasoning_policy(&mut json)?;
    route.apply_field_policy(&mut json)?;
    let Some(obj) = json.as_object_mut() else {
        return Ok(body.to_vec());
    };
    if let Some(model) = &route.model {
        obj.insert("model".to_string(), Value::String(model.clone()));
    }
    if route.force_nonstream {
        obj.insert("stream".to_string(), Value::Bool(false));
    }
    if let Some(provider) = &route.provider {
        let backend = config
            .runtime
            .backends
            .get(&route.target)
            .context("resolved backend is missing")?;
        if backend.provider_fields.is_empty() {
            bail!("selected provider requires backend.provider_fields");
        }
        for (pointer, template) in &backend.provider_fields {
            let mut value = template.clone();
            render_provider_template(&mut value, provider);
            routing::set_json_pointer(&mut json, pointer, value)?;
        }
    }

    normalize_message_roles(&mut json, &route.payload_normalization);
    if let Some(defaults) = &route.payload_normalization.tool_defaults {
        normalize_tool_definitions(&mut json, defaults);
    }
    if route.payload_normalization.normalize_object_required {
        normalize_object_schema(&mut json);
    }

    if word_gateway
        && (route.force_nonstream
            || (config.word_gateway.force_nonstream_when_requested && requested_stream))
    {
        let Some(obj) = json.as_object_mut() else {
            return Ok(body.to_vec());
        };
        obj.insert("stream".to_string(), Value::Bool(false));
        if config.word_gateway.remove_tools_when_nonstream {
            obj.remove("tools");
            obj.remove("tool_choice");
            obj.remove("mcp_servers");
        }
        if let Some(range) = config.word_gateway.max_tokens_when_nonstream {
            let max_tokens = obj.get("max_tokens").and_then(Value::as_u64).unwrap_or(0);
            if !(range.min..=range.max).contains(&max_tokens) {
                obj.insert(
                    "max_tokens".to_string(),
                    Value::Number(max_tokens.clamp(range.min, range.max).into()),
                );
            }
        }
    }

    serde_json::to_vec(&json).context("failed to serialize routed request")
}

fn configured_client_settings(client: &ClientConfig, headers: &HeaderMap) -> ClientSettings {
    let Some(source) = &client.settings else {
        return ClientSettings::default();
    };
    let path = source
        .path_env
        .iter()
        .find_map(|name| env::var_os(name).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(&source.path));
    let mut settings = fs::read(path)
        .ok()
        .map_or_else(ClientSettings::default, |bytes| {
            client_settings_from_json(&bytes, source)
        });
    if let Some(spec) = &source.model_header
        && let Some(model) = header_parameter(headers, &spec.header, &spec.parameter)
    {
        settings.model = Some(model.to_string());
    }
    settings
}

fn render_provider_template(value: &mut Value, provider: &str) {
    match value {
        Value::String(text) => *text = text.replace("{provider}", provider),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| render_provider_template(item, provider)),
        Value::Object(object) => object
            .values_mut()
            .for_each(|item| render_provider_template(item, provider)),
        _ => {}
    }
}

fn load_router_runtime_config(path: &std::path::Path) -> Result<RouterConfig> {
    let config = fs::read(path)
        .with_context(|| format!("failed to read router config {}", path.display()))?;
    let mut config = RouterConfig::from_json(&config)
        .with_context(|| format!("invalid router config {}", path.display()))?;
    config.runtime.resolve_paths(
        path.parent()
            .context("configuration directory is missing")?,
    );
    Ok(config)
}

fn client_settings_from_json(settings: &[u8], source: &SettingsSource) -> ClientSettings {
    let Ok(json) = serde_json::from_slice::<Value>(settings) else {
        return ClientSettings::default();
    };
    ClientSettings {
        fast_mode: json
            .pointer(&source.fast_mode_pointer)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        model: json
            .pointer(&source.model_pointer)
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn header_parameter<'a>(headers: &'a HeaderMap, name: &str, key: &str) -> Option<&'a str> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .split(';')
        .skip(1)
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(candidate, value)| (candidate.trim() == key).then_some(value.trim()))
        .filter(|value| !value.is_empty())
}

fn normalize_openrouter_web_search_tools(obj: &mut serde_json::Map<String, Value>) -> usize {
    let mut rewritten = 0;
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            let Some(tool_obj) = tool.as_object_mut() else {
                continue;
            };
            let is_anthropic_web_search = tool_obj
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.starts_with("web_search_"));
            if !is_anthropic_web_search {
                continue;
            }

            let mut parameters = serde_json::Map::new();
            if let Some(value) = tool_obj.remove("allowed_domains") {
                parameters.insert("allowed_domains".to_string(), value);
            }
            if let Some(value) = tool_obj.remove("blocked_domains") {
                parameters.insert("excluded_domains".to_string(), value);
            }
            if let Some(value) = tool_obj.remove("user_location") {
                parameters.insert("user_location".to_string(), value);
            }

            tool_obj.clear();
            tool_obj.insert(
                "type".to_string(),
                Value::String("openrouter:web_search".to_string()),
            );
            if !parameters.is_empty() {
                tool_obj.insert("parameters".to_string(), Value::Object(parameters));
            }
            rewritten += 1;
        }
    }
    if rewritten > 0
        && let Some(choice) = obj.get_mut("tool_choice").and_then(Value::as_object_mut)
        && choice.get("name").and_then(Value::as_str) == Some("web_search")
    {
        choice.remove("name");
        choice.insert("type".to_string(), Value::String("any".to_string()));
    }
    rewritten
}

#[cfg(test)]
fn bridge_openrouter_web_search_request(
    body: &[u8],
    requested_stream: bool,
    policy: &WebSearchPolicy,
) -> (Vec<u8>, usize) {
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else {
        return (body.to_vec(), 0);
    };
    let Some(obj) = json.as_object_mut() else {
        return (body.to_vec(), 0);
    };
    let rewritten_tools = normalize_openrouter_web_search_tools(obj);
    if rewritten_tools == 0 {
        return (body.to_vec(), 0);
    }

    let max_tokens = obj
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(policy.max_output_tokens)
        .min(policy.max_output_tokens);
    obj.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
    obj.insert(
        "reasoning".to_string(),
        serde_json::json!({ "effort": policy.reasoning_effort }),
    );
    if requested_stream {
        // Some provider-specific routes normally use a synthesized non-stream
        // response. Search citations are only complete in OpenRouter's SSE, and
        // this bridge consumes and normalizes that stream before Claude sees it.
        obj.insert("stream".to_string(), Value::Bool(true));
    }
    let rewritten = serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec());
    (rewritten, rewritten_tools)
}

#[cfg(test)]
fn normalize_message_roles(json: &mut Value, policy: &PayloadNormalization) {
    let Some(messages) = json.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        if let Some(role) = message.get("role").and_then(Value::as_str)
            && let Some(mapped) = policy.message_role_map.get(role)
        {
            message["role"] = Value::String(mapped.clone());
        }
    }
}

#[cfg(test)]
fn normalize_tool_definitions(json: &mut Value, defaults: &ToolDefaults) {
    let Some(tools) = json.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for tool in tools {
        let Some(object) = tool.as_object_mut() else {
            continue;
        };
        let Some(name) = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let has_description = object
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|description| !description.trim().is_empty());
        if !has_description {
            object.insert(
                "description".to_string(),
                Value::String(defaults.description_template.replace("{name}", &name)),
            );
        }
        if !object.get("input_schema").is_some_and(Value::is_object) {
            object.insert("input_schema".to_string(), defaults.input_schema.clone());
        }
    }
}

fn normalize_object_schema(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                normalize_object_schema(item);
            }
        }
        Value::Object(object) => {
            let object_schema = object.get("type").and_then(Value::as_str) == Some("object")
                && object.contains_key("properties");
            if object_schema && !object.get("required").is_some_and(Value::is_array) {
                object.insert("required".to_string(), Value::Array(Vec::new()));
            }
            for child in object.values_mut() {
                normalize_object_schema(child);
            }
        }
        _ => {}
    }
}

fn upstream_url(backend: &BackendConfig, uri: &Uri) -> String {
    let path = backend
        .path_rewrites
        .get(uri.path())
        .map(String::as_str)
        .unwrap_or_else(|| uri.path());
    let query = uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    format!("{}{path}{query}", backend.base_url.trim_end_matches('/'))
}

fn word_models_response(config: &RouterConfig) -> Result<Response> {
    let models = config
        .word_gateway
        .models
        .iter()
        .map(|model| word_model(model, &config.word_gateway.model_defaults))
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "object": "list",
        "data": models
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .context("failed to build Word models response")
}

fn word_model(model: &routing::WordModel, defaults: &serde_json::Map<String, Value>) -> Value {
    let mut value = serde_json::json!({
        "id": model.id,
        "object": "model",
        "type": "model",
        "name": model.name,
        "display_name": model.name,
        "context_window": model.context_window,
        "max_output_tokens": model.max_output_tokens,
        "supports_reasoning": model.supports_reasoning
    });
    let object = value.as_object_mut().expect("model metadata object");
    for (key, item) in defaults.iter().chain(model.metadata.iter()) {
        object.insert(key.clone(), item.clone());
    }
    value
}

fn rewritten_models_response(
    status: reqwest::StatusCode,
    upstream_headers: &reqwest::header::HeaderMap,
    body: &Bytes,
    config: &RouterConfig,
) -> Result<Response> {
    let rewritten =
        rewrite_openrouter_models_for_word(body, config).unwrap_or_else(|| body.to_vec());
    let mut response = Response::builder().status(status);
    for (name, value) in upstream_headers.iter() {
        if should_forward_response_header(name) && name.as_str() != "content-type" {
            response = response.header(name, value);
        }
    }
    response
        .header("content-type", "application/json")
        .body(Body::from(rewritten))
        .context("failed to build rewritten models response")
}

fn apply_cors_headers(response: &mut Response, policy: &CorsPolicy) {
    for (name, value) in [
        ("access-control-allow-origin", policy.allow_origin.clone()),
        ("access-control-allow-methods", policy.allow_methods.clone()),
        ("access-control-allow-headers", policy.allow_headers.clone()),
        (
            "access-control-expose-headers",
            policy.expose_headers.clone(),
        ),
        (
            "access-control-allow-private-network",
            policy.allow_private_network.to_string(),
        ),
        ("access-control-max-age", policy.max_age_seconds.to_string()),
    ] {
        response
            .headers_mut()
            .entry(name)
            .or_insert(value.parse().expect("validated CORS policy"));
    }
}

fn rewrite_openrouter_models_for_word(body: &Bytes, config: &RouterConfig) -> Option<Vec<u8>> {
    let prefix = config
        .word_gateway
        .response_model_prefix_to_strip
        .as_deref()?;
    let required_prefix = config
        .word_gateway
        .response_model_required_prefix
        .as_deref();
    let mut json = serde_json::from_slice::<Value>(body).ok()?;
    let models = json.get_mut("data")?.as_array_mut()?;
    for model in models {
        let Some(obj) = model.as_object_mut() else {
            continue;
        };
        let Some(id) = obj.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(stripped) = id.strip_prefix(prefix) else {
            continue;
        };
        if required_prefix.is_none_or(|required| stripped.starts_with(required)) {
            obj.insert("id".to_string(), Value::String(stripped.to_string()));
        }
    }
    serde_json::to_vec(&json).ok()
}

fn rewritten_word_message_response(
    status: reqwest::StatusCode,
    upstream_headers: &reqwest::header::HeaderMap,
    body: &Bytes,
    synthesize_stream: bool,
    fallback_model: &str,
) -> Result<Response> {
    let is_event_stream = upstream_headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));
    let rewritten = if synthesize_stream {
        synthesize_word_message_stream(body, fallback_model).unwrap_or_else(|| body.to_vec())
    } else if is_event_stream {
        rewrite_word_message_stream(body).unwrap_or_else(|| body.to_vec())
    } else {
        rewrite_word_message_body(body).unwrap_or_else(|| body.to_vec())
    };
    let mut response = Response::builder().status(status);
    for (name, value) in upstream_headers.iter() {
        if should_forward_response_header(name) && name.as_str() != "content-type" {
            response = response.header(name, value);
        }
    }
    let content_type = if synthesize_stream || is_event_stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    response
        .header("content-type", content_type)
        .body(Body::from(rewritten))
        .context("failed to build rewritten Word message response")
}

fn rewritten_openrouter_web_search_response(
    status: reqwest::StatusCode,
    upstream_headers: &reqwest::header::HeaderMap,
    body: &Bytes,
) -> Result<Response> {
    let is_event_stream = upstream_headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));
    let rewritten = if status.is_success() && is_event_stream {
        rewrite_openrouter_web_search_stream(body).unwrap_or_else(|| body.to_vec())
    } else if status.is_success() {
        rewrite_openrouter_web_search_body(body).unwrap_or_else(|| body.to_vec())
    } else {
        body.to_vec()
    };
    let mut response = Response::builder().status(status);
    for (name, value) in upstream_headers.iter() {
        if should_forward_response_header(name) && name.as_str() != "content-type" {
            response = response.header(name, value);
        }
    }
    let content_type = if is_event_stream {
        "text/event-stream"
    } else {
        upstream_headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/json")
    };
    response
        .header("content-type", content_type)
        .body(Body::from(rewritten))
        .context("failed to build rewritten OpenRouter web search response")
}

fn rewrite_openrouter_web_search_body(body: &Bytes) -> Option<Vec<u8>> {
    let mut message = serde_json::from_slice::<Value>(body).ok()?;
    let content = message.get_mut("content")?.as_array_mut()?;
    let server_tool_position = content.iter().position(|block| {
        block.get("type").and_then(Value::as_str) == Some("server_tool_use")
            && block
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == "openrouter:web_search" || name == "web_search")
    })?;
    let server_tool_id = content[server_tool_position]
        .get("id")
        .and_then(Value::as_str)?
        .to_string();
    if let Some(obj) = content[server_tool_position].as_object_mut() {
        obj.insert("name".to_string(), Value::String("web_search".to_string()));
    }

    let mut results = Vec::new();
    for block in content.iter() {
        collect_openrouter_web_search_results(block, &mut results);
    }
    let already_has_result = content
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("web_search_tool_result"));
    if !already_has_result {
        content.insert(
            server_tool_position + 1,
            serde_json::json!({
                "type": "web_search_tool_result",
                "tool_use_id": server_tool_id,
                "content": results
            }),
        );
    }
    serde_json::to_vec(&message).ok()
}

fn rewrite_openrouter_web_search_stream(body: &Bytes) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(body).ok()?;
    let normalized = text.replace("\r\n", "\n");
    let events = normalized
        .split("\n\n")
        .filter_map(parse_sse_event)
        .collect::<Vec<_>>();

    let mut server_tool_index = None;
    let mut server_tool_id = None;
    let mut results = Vec::<Value>::new();
    let mut already_has_result = false;
    for (_, data) in &events {
        if data
            .get("content_block")
            .and_then(|block| block.get("type"))
            .and_then(Value::as_str)
            == Some("server_tool_use")
            && data
                .get("content_block")
                .and_then(|block| block.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|name| name == "openrouter:web_search" || name == "web_search")
        {
            server_tool_index = data.get("index").and_then(Value::as_u64);
            server_tool_id = data
                .get("content_block")
                .and_then(|block| block.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if data
            .get("content_block")
            .and_then(|block| block.get("type"))
            .and_then(Value::as_str)
            == Some("web_search_tool_result")
        {
            already_has_result = true;
        }
        collect_openrouter_web_search_results(data, &mut results);
    }

    let server_tool_index = server_tool_index?;
    let server_tool_id = server_tool_id?;
    let result_index = server_tool_index + 1;
    let mut out = String::with_capacity(normalized.len() + 1024);
    let mut inserted_result = already_has_result;
    for (event_name, mut data) in events {
        if let Some(content_block) = data.get_mut("content_block").and_then(Value::as_object_mut)
            && content_block.get("type").and_then(Value::as_str) == Some("server_tool_use")
            && content_block
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == "openrouter:web_search")
        {
            content_block.insert("name".to_string(), Value::String("web_search".to_string()));
        }

        let original_index = data.get("index").and_then(Value::as_u64);
        if original_index.is_some_and(|index| index > server_tool_index)
            && let Some(obj) = data.as_object_mut()
        {
            obj.insert(
                "index".to_string(),
                Value::Number((original_index.unwrap_or_default() + 1).into()),
            );
        }
        push_sse_event(&mut out, &event_name, data);

        if !inserted_result
            && event_name == "content_block_stop"
            && original_index == Some(server_tool_index)
        {
            push_sse_event(
                &mut out,
                "content_block_start",
                serde_json::json!({
                    "type": "content_block_start",
                    "index": result_index,
                    "content_block": {
                        "type": "web_search_tool_result",
                        "tool_use_id": server_tool_id.clone(),
                        "content": results.clone()
                    }
                }),
            );
            push_sse_event(
                &mut out,
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": result_index
                }),
            );
            inserted_result = true;
        }
    }
    inserted_result.then(|| out.into_bytes())
}

fn parse_sse_event(event: &str) -> Option<(String, Value)> {
    let trimmed = event.trim();
    if trimmed.is_empty() || is_openrouter_done_event(trimmed) {
        return None;
    }
    let event_name = trimmed
        .lines()
        .find_map(|line| line.strip_prefix("event:"))?
        .trim()
        .to_string();
    let data = trimmed
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    let json = serde_json::from_str::<Value>(&data).ok()?;
    Some((event_name, json))
}

fn collect_openrouter_web_search_results(data: &Value, results: &mut Vec<Value>) {
    let mut citations = Vec::new();
    if let Some(citation) = data.get("delta").and_then(|delta| delta.get("citation")) {
        citations.push(citation);
    }
    if let Some(items) = data
        .get("delta")
        .and_then(|delta| delta.get("citations"))
        .and_then(Value::as_array)
    {
        citations.extend(items);
    }
    if let Some(items) = data.get("citations").and_then(Value::as_array) {
        citations.extend(items);
    }
    for citation in citations {
        let Some(url) = citation.get("url").and_then(Value::as_str) else {
            continue;
        };
        let title = citation.get("title").and_then(Value::as_str).unwrap_or(url);
        if results.iter().any(|result| {
            result.get("url").and_then(Value::as_str) == Some(url)
                && result.get("title").and_then(Value::as_str) == Some(title)
        }) {
            continue;
        }
        let mut result = serde_json::json!({
            "type": "web_search_result",
            "title": title,
            "url": url
        });
        if let Some(page_age) = citation.get("page_age").cloned()
            && let Some(obj) = result.as_object_mut()
        {
            obj.insert("page_age".to_string(), page_age);
        }
        results.push(result);
    }
}

fn rewrite_word_message_body(body: &Bytes) -> Option<Vec<u8>> {
    let mut json = serde_json::from_slice::<Value>(body).ok()?;
    normalize_word_message_id(&mut json);
    let content = json.get_mut("content")?.as_array_mut()?;
    content.retain(|item| {
        item.get("type")
            .and_then(Value::as_str)
            .is_none_or(|kind| !kind.contains("thinking"))
    });
    serde_json::to_vec(&json).ok()
}

fn rewrite_word_message_stream(body: &Bytes) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(body).ok()?;
    let normalized = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(normalized.len());
    for event in normalized.split("\n\n") {
        let trimmed = event.trim();
        if trimmed.is_empty()
            || is_openrouter_done_event(trimmed)
            || is_thinking_stream_event(trimmed)
        {
            continue;
        }
        out.push_str(trimmed);
        out.push_str("\n\n");
    }
    Some(out.into_bytes())
}

fn synthesize_word_message_stream(body: &Bytes, fallback_model: &str) -> Option<Vec<u8>> {
    let mut message = serde_json::from_slice::<Value>(body).ok()?;
    normalize_word_message_id(&mut message);
    let content = message.get_mut("content")?.as_array_mut()?;
    let blocks = content
        .iter()
        .filter(|item| {
            item.get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "text" || kind == "tool_use")
        })
        .cloned()
        .collect::<Vec<_>>();
    let input_tokens = message
        .get("usage")
        .and_then(|usage| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation_input_tokens = message
        .get("usage")
        .and_then(|usage| usage.get("cache_creation_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read_input_tokens = message
        .get("usage")
        .and_then(|usage| usage.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation = message
        .get("usage")
        .and_then(|usage| usage.get("cache_creation"))
        .cloned()
        .unwrap_or(Value::Null);
    let output_tokens = message
        .get("usage")
        .and_then(|usage| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let stop_reason = message
        .get("stop_reason")
        .cloned()
        .unwrap_or(Value::String("end_turn".to_string()));
    let stop_sequence = message.get("stop_sequence").cloned().unwrap_or(Value::Null);
    let stream_message = serde_json::json!({
        "id": message.get("id").cloned().unwrap_or_else(|| Value::String("msg_proxy".to_string())),
        "type": "message",
        "role": "assistant",
        "model": message.get("model").cloned().unwrap_or_else(|| Value::String(fallback_model.to_string())),
        "content": [],
        "stop_reason": null,
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "cache_creation_input_tokens": cache_creation_input_tokens,
            "cache_read_input_tokens": cache_read_input_tokens,
            "cache_creation": cache_creation,
            "output_tokens": 0
        }
    });

    let mut out = String::new();
    push_sse_event(
        &mut out,
        "message_start",
        serde_json::json!({
            "type": "message_start",
            "message": stream_message
        }),
    );
    for (index, block) in blocks.iter().enumerate() {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                push_sse_event(
                    &mut out,
                    "content_block_start",
                    serde_json::json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {
                            "type": "text",
                            "text": "",
                            "citations": []
                        }
                    }),
                );
                if let Some(text) = block.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    push_sse_event(
                        &mut out,
                        "content_block_delta",
                        serde_json::json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": {
                                "type": "text_delta",
                                "text": text
                            }
                        }),
                    );
                }
                push_sse_event(
                    &mut out,
                    "content_block_stop",
                    serde_json::json!({
                        "type": "content_block_stop",
                        "index": index
                    }),
                );
            }
            Some("tool_use") => {
                let tool_id = block
                    .get("id")
                    .cloned()
                    .unwrap_or_else(|| Value::String(format!("toolu_proxy_{index}")));
                let tool_name = block
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| Value::String("tool".to_string()));
                let tool_input = block
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                push_sse_event(
                    &mut out,
                    "content_block_start",
                    serde_json::json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {
                            "type": "tool_use",
                            "id": tool_id,
                            "name": tool_name,
                            "input": {}
                        }
                    }),
                );
                push_sse_event(
                    &mut out,
                    "content_block_delta",
                    serde_json::json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": tool_input.to_string()
                        }
                    }),
                );
                push_sse_event(
                    &mut out,
                    "content_block_stop",
                    serde_json::json!({
                        "type": "content_block_stop",
                        "index": index
                    }),
                );
            }
            _ => {}
        }
    }
    push_sse_event(
        &mut out,
        "message_delta",
        serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": stop_sequence
            },
            "usage": {
                "output_tokens": output_tokens
            }
        }),
    );
    push_sse_event(
        &mut out,
        "message_stop",
        serde_json::json!({
            "type": "message_stop"
        }),
    );
    Some(out.into_bytes())
}

fn push_sse_event(out: &mut String, event: &str, data: Value) {
    out.push_str("event: ");
    out.push_str(event);
    out.push('\n');
    out.push_str("data: ");
    out.push_str(&data.to_string());
    out.push_str("\n\n");
}

fn normalize_word_message_id(message: &mut Value) {
    let Some(id) = message.get("id").and_then(Value::as_str) else {
        return;
    };
    if id.starts_with("msg_") {
        return;
    }
    let suffix = id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    let normalized = if suffix.is_empty() {
        "msg_proxy".to_string()
    } else {
        format!("msg_{suffix}")
    };
    message["id"] = Value::String(normalized);
}

fn is_openrouter_done_event(event: &str) -> bool {
    event.lines().any(|line| line.trim() == "data: [DONE]")
}

fn sse_message_is_complete(buffer: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(buffer) else {
        return false;
    };
    let normalized = text.replace("\r\n", "\n");
    normalized.split("\n\n").any(|event| {
        is_openrouter_done_event(event)
            || parse_sse_event(event).is_some_and(|(_, data)| {
                data.get("type").and_then(Value::as_str) == Some("message_stop")
            })
    })
}

fn is_thinking_stream_event(event: &str) -> bool {
    event.lines().any(|line| {
        let Some(data) = line.strip_prefix("data:") else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            return false;
        };
        value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.contains("thinking"))
            || value
                .get("content_block")
                .and_then(|block| block.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("thinking"))
            || value
                .get("delta")
                .and_then(|delta| delta.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("thinking"))
    })
}

fn copy_request_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HeaderMap,
    route: &ResolvedRoute,
    strip_headers: &[String],
) -> reqwest::RequestBuilder {
    for (name, value) in headers.iter() {
        if !should_forward_request_header(name, strip_headers) || !route.should_forward_header(name)
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
}

fn should_forward_request_header(name: &HeaderName, strip_headers: &[String]) -> bool {
    !is_hop_by_hop_request_header(name)
        && !strip_headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case(name.as_str()))
}

fn is_dry_run(headers: &HeaderMap, server: &ServerConfig) -> bool {
    headers
        .get(&server.dry_run_header)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            server
                .dry_run_values
                .iter()
                .any(|expected| expected.eq_ignore_ascii_case(value))
        })
}

fn is_successful_event_stream(status: StatusCode, headers: &HeaderMap) -> bool {
    status.is_success()
        && headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
}

fn drain_complete_sse_events(buffer: &mut Vec<u8>) -> Vec<u8> {
    let mut forwarded = Vec::new();
    while let Some(end) = next_sse_event_end(buffer) {
        let event = buffer.drain(..end).collect::<Vec<_>>();
        let is_done = std::str::from_utf8(&event)
            .ok()
            .is_some_and(is_openrouter_done_event);
        if !is_done {
            forwarded.extend_from_slice(&event);
        }
    }
    forwarded
}

fn next_sse_event_end(buffer: &[u8]) -> Option<usize> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| index + 2);
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4);
    match (lf, crlf) {
        (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
        (Some(end), None) | (None, Some(end)) => Some(end),
        (None, None) => None,
    }
}

#[cfg(test)]
fn body_requests_stream(body: impl AsRef<[u8]>) -> bool {
    serde_json::from_slice::<Value>(body.as_ref())
        .ok()
        .and_then(|json| json.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn dry_run_response(
    route: &ResolvedRoute,
    backend: &BackendConfig,
    uri: &Uri,
    prepared: &prepare::Prepared,
) -> Result<Response> {
    let target = route.target.as_str();
    let original_model = &prepared.original_model;
    let outgoing_model = &route.model;
    let outgoing_json = &prepared.outgoing_metadata;
    let outgoing_fields = &prepared.outgoing_fields;
    let target_name = target;
    let payload = serde_json::json!({
        "rule_id": route.rule_id,
        "target": target_name,
        "upstream_url": upstream_url(backend, uri),
        "original_model": original_model,
        "outgoing_model": outgoing_model,
        "outgoing_provider": outgoing_json.get("provider").cloned().unwrap_or(Value::Null),
        "outgoing_stream": outgoing_json.get("stream").cloned().unwrap_or(Value::Null),
        "outgoing_speed": outgoing_json.get("speed").cloned().unwrap_or(Value::Null),
        "outgoing_reasoning": outgoing_json.get("reasoning").cloned().unwrap_or(Value::Null),
        "outgoing_reasoning_effort": outgoing_json.get("reasoning_effort").cloned().unwrap_or(Value::Null),
        "outgoing_fields": outgoing_fields,
        "response_cache_ttl_seconds": route.response_cache_ttl_seconds,
        "synthesize_anthropic_sse": route.synthesize_anthropic_sse,
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .context("failed to build dry-run response")
}

fn is_hop_by_hop_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "accept-encoding"
            | "connection"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_hop_by_hop_response_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "content-length"
            | "connection"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    !is_hop_by_hop_response_header(name) && !name.as_str().starts_with("access-control-")
}

fn log_upstream_response(
    target: &str,
    backend: &BackendConfig,
    word_gateway: bool,
    method: &Method,
    uri: &Uri,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) {
    let target_name = target;
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-");
    let cache_header = |name: Option<&String>| {
        name.and_then(|name| headers.get(name))
            .and_then(|value| value.to_str().ok())
            .unwrap_or("-")
    };
    let cache_status = cache_header(
        backend
            .cache
            .as_ref()
            .map(|cache| &cache.response_status_header),
    );
    let cache_age = cache_header(
        backend
            .cache
            .as_ref()
            .map(|cache| &cache.response_age_header),
    );
    let cache_ttl = cache_header(
        backend
            .cache
            .as_ref()
            .map(|cache| &cache.response_ttl_header),
    );
    eprintln!(
        "{method} {uri} word={word_gateway} <- {target_name} status={} content-type={content_type} cache={cache_status} cache-age={cache_age} cache-ttl={cache_ttl}",
        status.as_u16(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verifies_file_backed_processing_against_request_fixture() {
        let Some(path) = env::var_os("ROUTER_TEST_REQUEST") else {
            return;
        };
        let config_path =
            PathBuf::from(env::var_os("ROUTER_TEST_CONFIG").expect("ROUTER_TEST_CONFIG"));
        let config = load_router_runtime_config(&config_path).unwrap();
        let server = config.runtime.server.as_ref().unwrap();
        fs::create_dir_all(&server.body_processing.spool_directory).unwrap();
        let original: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        for case in &config.validation_cases {
            let mut request = original.clone();
            request["model"] = Value::String(case.model.clone());
            if let Some(speed) = &case.speed {
                request["speed"] = Value::String(speed.clone());
            }
            if let Some(content) = &case.last_user_content {
                request["messages"]
                    .as_array_mut()
                    .expect("request messages")
                    .push(serde_json::json!({"role": "user", "content": content}));
            }
            let headers = case
                .headers
                .iter()
                .map(|(name, value)| (name.parse::<HeaderName>().unwrap(), value.parse().unwrap()))
                .collect::<HeaderMap>();
            let route = config
                .resolve(
                    &case.context,
                    &headers,
                    &request,
                    case.settings_fast_mode,
                    case.settings_model.as_deref(),
                )
                .unwrap();
            let compatibility = config
                .runtime
                .clients
                .iter()
                .find(|client| client.id == case.context)
                .unwrap()
                .word_gateway;
            let bytes = serde_json::to_vec(&request).unwrap();
            let expected = apply_resolved_route(&bytes, &route, compatibility, &config).unwrap();
            let expected = if let Some(policy) = &config.runtime.backends[&route.target].web_search
            {
                bridge_openrouter_web_search_request(
                    &expected,
                    body_requests_stream(&bytes),
                    policy,
                )
                .0
            } else {
                expected
            };
            let input = body::JsonBody::receive(
                Body::from(bytes),
                &server.body_processing,
                server.max_request_bytes,
            )
            .await
            .unwrap();
            let settings = ClientSettings {
                fast_mode: case.settings_fast_mode,
                model: case.settings_model.clone(),
            };
            let prepared = prepare::prepare(
                input,
                &config,
                &case.context,
                &headers,
                &settings,
                compatibility,
                Protocol::Messages,
            )
            .unwrap();
            assert_eq!(prepared.route.rule_id, route.rule_id, "{}", case.name);
            let actual: Value =
                serde_json::from_slice(&prepared.body.bytes_for_verification().unwrap()).unwrap();
            let expected: Value = serde_json::from_slice(&expected).unwrap();
            assert!(
                actual == expected,
                "request transformation mismatch: {}",
                case.name
            );
        }
    }

    #[test]
    fn live_router_config_is_valid() {
        let bytes = env::var_os("ROUTER_TEST_CONFIG")
            .map(|path| fs::read(path).unwrap())
            .unwrap_or_else(|| include_bytes!("../examples/router.json").to_vec());
        RouterConfig::from_json(&bytes).expect("live router config must validate");
    }

    fn route_with_reasoning(policy: Value) -> ResolvedRoute {
        ResolvedRoute {
            rule_id: "sample".to_string(),
            target: "sample".to_string(),
            model: None,
            provider: None,
            reasoning_policy: Some(serde_json::from_value(policy).unwrap()),
            force_nonstream: false,
            payload_normalization: PayloadNormalization::default(),
            synthesize_anthropic_sse: false,
            response_cache_ttl_seconds: None,
            field_policy: Default::default(),
            header_policy: Default::default(),
        }
    }

    #[test]
    fn translates_nested_reasoning_without_model_knowledge() {
        let mut request = serde_json::json!({
            "thinking": {"type": "adaptive"}, "output_config": {"effort": "high"},
            "reasoning": {"budget_tokens": 1234}
        });
        let route = route_with_reasoning(serde_json::json!({
            "sources": ["/output_config/effort"], "destination": "/reasoning/effort", "remove": ["/output_config", "/thinking"]
        }));
        route.apply_reasoning_policy(&mut request).unwrap();
        assert_eq!(request["reasoning"]["effort"], "high");
        assert_eq!(request["reasoning"]["budget_tokens"], 1234);
        assert!(request.get("thinking").is_none());
        assert!(request.get("output_config").is_none());
    }

    #[test]
    fn translates_flat_reasoning_without_model_knowledge() {
        let mut request = serde_json::json!({"output_config": {"effort": "max"}});
        let route = route_with_reasoning(serde_json::json!({
            "sources": ["/output_config/effort"], "destination": "/reasoning_effort", "remove": ["/output_config"], "value_map": {"max": "high"}
        }));
        route.apply_reasoning_policy(&mut request).unwrap();
        assert_eq!(request["reasoning_effort"], "high");
        assert!(request.get("output_config").is_none());
    }

    #[test]
    fn derives_configured_client_settings() {
        let source: SettingsSource = serde_json::from_value(serde_json::json!({
            "path": "/tmp/unused-settings.json", "model_pointer": "/selection/model", "fast_mode_pointer": "/selection/fast"
        })).unwrap();
        assert_eq!(
            client_settings_from_json(
                br#"{"selection":{"fast":true,"model":"logical-model"}}"#,
                &source
            ),
            ClientSettings {
                fast_mode: true,
                model: Some("logical-model".to_string())
            }
        );
        assert_eq!(
            client_settings_from_json(br#"{"selection":{"fast":false}}"#, &source),
            ClientSettings::default()
        );
        assert_eq!(
            client_settings_from_json(b"not-json", &source),
            ClientSettings::default()
        );
    }

    #[test]
    fn explicit_launcher_model_overrides_shared_settings_selection() {
        let mut headers = HeaderMap::new();
        headers.insert("x-client", "1; selection=logical-model".parse().unwrap());
        assert_eq!(
            header_parameter(&headers, "x-client", "selection"),
            Some("logical-model")
        );
        let client: ClientConfig = serde_json::from_value(serde_json::json!({
            "id":"sample", "settings": {"path":"/nonexistent/router-test-settings.json", "model_pointer":"/model", "fast_mode_pointer":"/fast", "model_header":{"header":"x-client","parameter":"selection"}}
        })).unwrap();
        assert_eq!(
            configured_client_settings(&client, &headers)
                .model
                .as_deref(),
            Some("logical-model")
        );
    }

    #[test]
    fn filters_only_trailing_openrouter_done_event() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        assert!(!is_successful_event_stream(
            StatusCode::TOO_MANY_REQUESTS,
            &headers
        ));
        assert!(!is_successful_event_stream(StatusCode::OK, &headers));
        headers.insert(
            "content-type",
            "text/event-stream; charset=utf-8".parse().unwrap(),
        );
        assert!(is_successful_event_stream(StatusCode::OK, &headers));
        assert!(!is_successful_event_stream(
            StatusCode::TOO_MANY_REQUESTS,
            &headers
        ));
        let mut buffer =
            b"event: message_delta\ndata: {\"type\":\"message_delta\"}\n\ndata: [DONE]\n\n"
                .to_vec();
        let filtered = drain_complete_sse_events(&mut buffer);
        let text = String::from_utf8(filtered).unwrap();
        assert!(text.contains("message_delta"));
        assert!(!text.contains("[DONE]"));
    }

    #[test]
    fn bridges_web_search_for_arbitrary_openrouter_models() {
        let body = serde_json::json!({
            "model": "vendor/arbitrary-model",
            "stream": true,
            "max_tokens": 32000,
            "tools": [{"type": "web_search_20250305", "name": "web_search"}],
            "tool_choice": {"type": "tool", "name": "web_search"},
            "messages": [{"role": "user", "content": "search"}],
            "thinking": {"type": "adaptive"}
        });
        let (outgoing, count) = bridge_openrouter_web_search_request(
            body.to_string().as_bytes(),
            true,
            &WebSearchPolicy {
                max_output_tokens: 1234,
                reasoning_effort: "none".to_string(),
            },
        );
        let outgoing: Value = serde_json::from_slice(&outgoing).unwrap();
        assert_eq!(count, 1);
        assert_eq!(outgoing["tools"][0]["type"], "openrouter:web_search");
        assert_eq!(outgoing["tool_choice"]["type"], "any");
        assert_eq!(outgoing["max_tokens"], 1234);
        assert_eq!(outgoing["stream"], true);
    }

    #[test]
    fn synthesized_stream_preserves_cache_usage() {
        let body = Bytes::from(
            serde_json::json!({
                "id": "msg_test",
                "type": "message",
                "role": "assistant",
                "model": "vendor/model",
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 10,
                    "cache_creation_input_tokens": 20,
                    "cache_read_input_tokens": 30,
                    "cache_creation": {
                        "ephemeral_1h_input_tokens": 20,
                        "ephemeral_5m_input_tokens": 0
                    },
                    "output_tokens": 1
                }
            })
            .to_string(),
        );
        let stream = synthesize_word_message_stream(&body, "fallback/model").expect("valid stream");
        let text = String::from_utf8(stream).unwrap();
        assert!(text.contains("\"cache_creation_input_tokens\":20"));
        assert!(text.contains("\"cache_read_input_tokens\":30"));
        assert!(text.contains("\"ephemeral_1h_input_tokens\":20"));
    }
}
