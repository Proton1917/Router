use anyhow::{Context, Result};
use http::HeaderMap;
use serde_json::Value;

use crate::{
    ClientSettings,
    body::{JsonBody, Patch, Replacement, pointer},
    routing::{ResolvedRoute, RouterConfig},
    server::Protocol,
};

pub struct Prepared {
    pub body: JsonBody,
    pub route: ResolvedRoute,
    pub requested_stream: bool,
    pub outgoing_stream: bool,
    pub search_tools: usize,
    pub original_model: String,
    pub outgoing_fields: Vec<String>,
    pub original_bytes: u64,
    pub outgoing_metadata: Value,
}

fn assign(patch: &mut Patch, path: &str, value: Value) {
    patch.set.insert(pointer(path), Replacement::Json(value));
}

fn rewrite(body: &mut JsonBody, patch: Patch) -> Result<()> {
    let mut patch = patch;
    let mut remove = Vec::new();
    for path in patch.remove {
        if path.iter().any(|p| p == "*") || body.at(&path)?.is_some() {
            remove.push(path);
        }
    }
    patch.remove = remove;
    patch.text.retain(|(path, _, _)| !path.is_empty());
    let mut text = Vec::new();
    for rule in patch.text {
        let prefix = rule
            .0
            .iter()
            .take_while(|token| *token != "*")
            .cloned()
            .collect::<Vec<_>>();
        if body.at(&prefix)?.is_some() {
            text.push(rule);
        }
    }
    patch.text = text;
    if patch.remove.is_empty()
        && patch.set.is_empty()
        && patch.text.is_empty()
        && patch.retain.is_none()
        && !patch.drop_null
        && !patch.canonical
        && patch.role_map.is_empty()
        && patch.tool_defaults.is_none()
        && !patch.object_required
    {
        return Ok(());
    }
    *body = body.rewrite(&patch)?;
    Ok(())
}

pub fn prepare(
    mut body: JsonBody,
    config: &RouterConfig,
    context: &str,
    headers: &HeaderMap,
    settings: &ClientSettings,
    compatibility: bool,
    protocol: Protocol,
) -> Result<Prepared> {
    let original_bytes = body.len;
    let json_request = body.len > 0
        && (protocol != Protocol::Passthrough
            || headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("application/json")));
    let mut metadata = serde_json::json!({});
    if json_request {
        for path in [
            &config.runtime.selection.model_pointer,
            &config.runtime.selection.speed_pointer,
            "/stream",
        ] {
            if let Some(value) = body.value_at(path)? {
                crate::routing::set_json_pointer(&mut metadata, path, value)?;
            }
        }
    }
    let markers = config
        .routes
        .iter()
        .filter(|rule| {
            rule.matcher.contexts.is_empty() || rule.matcher.contexts.iter().any(|id| id == context)
        })
        .flat_map(|rule| rule.matcher.last_user_contains_all.clone())
        .collect::<Vec<_>>();
    let known_markers = if json_request {
        body.last_user_matches(&markers)?
    } else {
        Vec::new()
    };
    let route = config.resolve_with_markers(
        context,
        headers,
        &metadata,
        settings.fast_mode,
        settings.model.as_deref(),
        Some(&known_markers),
    )?;
    let requested_stream = metadata
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let original_model = metadata
        .pointer(&config.runtime.selection.model_pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut search_tools = 0;
    if json_request && protocol != Protocol::Passthrough {
        if let Some(policy) = &route.reasoning_policy {
            let mut effort = None;
            for path in &policy.sources {
                if let Some(value) = body
                    .value_at(path)?
                    .and_then(|v| v.as_str().map(str::to_owned))
                {
                    effort = Some(value);
                    break;
                }
            }
            if effort.is_none()
                && let Some(fallback) = &policy.fallback
                && body.value_at(&fallback.path)?.as_ref() == Some(&fallback.equals)
            {
                effort = Some(fallback.value.clone());
            }
            let mut patch = Patch {
                remove: policy.remove.iter().map(|p| pointer(p)).collect(),
                ..Default::default()
            };
            if let Some(effort) = effort {
                assign(
                    &mut patch,
                    &policy.destination,
                    Value::String(policy.value_map.get(&effort).unwrap_or(&effort).clone()),
                );
            }
            if !patch.remove.is_empty() || !patch.set.is_empty() {
                rewrite(&mut body, patch)?;
            }
        }
        let fields = &route.field_policy;
        if fields.retain_top_level.is_some() || !fields.remove.is_empty() {
            rewrite(
                &mut body,
                Patch {
                    retain: fields.retain_top_level.clone(),
                    remove: fields.remove.iter().map(|p| pointer(p)).collect(),
                    ..Default::default()
                },
            )?;
        }
        for (from, to) in &fields.rename {
            if let Some(source) = body.at(&pointer(from))? {
                let mut patch = Patch {
                    remove: vec![pointer(from)],
                    ..Default::default()
                };
                patch
                    .set
                    .insert(pointer(to), Replacement::Source(body.clone(), source));
                rewrite(&mut body, patch)?;
            }
        }
        for (path, value) in &fields.set {
            let mut patch = Patch::default();
            assign(&mut patch, path, value.clone());
            rewrite(&mut body, patch)?;
        }
        let mut patch = Patch {
            drop_null: fields.drop_null,
            ..Default::default()
        };
        for rule in &fields.replace_text_lines {
            for path in &rule.paths {
                patch.text.push((
                    pointer(path),
                    rule.prefix.clone(),
                    rule.replacement
                        .replace("{model}", route.model.as_deref().unwrap_or_default()),
                ));
            }
        }
        if patch.drop_null || !patch.text.is_empty() {
            rewrite(&mut body, patch)?;
        }
        let backend = config
            .runtime
            .backends
            .get(&route.target)
            .context("resolved backend is missing")?;
        let mut patch = Patch::default();
        if let Some(model) = &route.model
            && body.value_at("/model")?.as_ref().and_then(Value::as_str) != Some(model.as_str())
        {
            assign(&mut patch, "/model", Value::String(model.clone()));
        }
        if route.force_nonstream {
            assign(&mut patch, "/stream", Value::Bool(false));
        }
        if let Some(provider) = &route.provider {
            anyhow::ensure!(
                !backend.provider_fields.is_empty(),
                "selected provider requires backend.provider_fields"
            );
            for (path, template) in &backend.provider_fields {
                let mut value = template.clone();
                crate::render_provider_template(&mut value, provider);
                assign(&mut patch, path, value);
            }
        }
        if !patch.set.is_empty() {
            rewrite(&mut body, patch)?;
        }
        let normalization = &route.payload_normalization;
        if !normalization.message_role_map.is_empty()
            || normalization.tool_defaults.is_some()
            || normalization.normalize_object_required
        {
            rewrite(
                &mut body,
                Patch {
                    role_map: normalization.message_role_map.clone().into_iter().collect(),
                    tool_defaults: normalization.tool_defaults.clone(),
                    object_required: normalization.normalize_object_required,
                    ..Default::default()
                },
            )?;
        }
        if compatibility
            && protocol == Protocol::Messages
            && (route.force_nonstream
                || config.word_gateway.force_nonstream_when_requested && requested_stream)
        {
            let mut patch = Patch::default();
            assign(&mut patch, "/stream", Value::Bool(false));
            if config.word_gateway.remove_tools_when_nonstream {
                patch.remove = ["/tools", "/tool_choice", "/mcp_servers"]
                    .iter()
                    .map(|p| pointer(p))
                    .collect();
            }
            if let Some(range) = config.word_gateway.max_tokens_when_nonstream {
                let n = body
                    .value_at("/max_tokens")?
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    .clamp(range.min, range.max);
                assign(&mut patch, "/max_tokens", Value::from(n));
            }
            rewrite(&mut body, patch)?;
        }
        if protocol == Protocol::Messages
            && let Some(policy) = &backend.web_search
        {
            let mut patch = Patch::default();
            body.visit_array("/tools", |index, span| {
                if body.kind(span)? != struson::reader::ValueType::Object {
                    return Ok(());
                }
                let fields = body.fields(span)?;
                let Some(kind) = fields.get("type") else {
                    return Ok(());
                };
                if !body
                    .value(*kind)?
                    .as_str()
                    .is_some_and(|kind| kind.starts_with("web_search_"))
                {
                    return Ok(());
                }
                let mut holder = serde_json::json!({"tools": [body.value(span)?]});
                search_tools += crate::normalize_openrouter_web_search_tools(
                    holder.as_object_mut().expect("tools object"),
                );
                assign(
                    &mut patch,
                    &format!("/tools/{index}"),
                    holder["tools"][0].take(),
                );
                Ok(())
            })?;
            if search_tools > 0 {
                if let Some(mut choice) = body.value_at("/tool_choice")?
                    && choice.get("name").and_then(Value::as_str) == Some("web_search")
                    && let Some(object) = choice.as_object_mut()
                {
                    object.remove("name");
                    object.insert("type".to_owned(), Value::String("any".to_owned()));
                    assign(&mut patch, "/tool_choice", choice);
                }
                let n = body
                    .value_at("/max_tokens")?
                    .and_then(|v| v.as_u64())
                    .unwrap_or(policy.max_output_tokens)
                    .min(policy.max_output_tokens);
                assign(&mut patch, "/max_tokens", Value::from(n));
                assign(
                    &mut patch,
                    "/reasoning",
                    serde_json::json!({"effort": policy.reasoning_effort}),
                );
                if requested_stream {
                    assign(&mut patch, "/stream", Value::Bool(true));
                }
                rewrite(&mut body, patch)?;
            }
        }
        // 精确响应缓存使用规范的对象键顺序。
        if route.response_cache_ttl_seconds.is_some() {
            rewrite(
                &mut body,
                Patch {
                    canonical: true,
                    ..Default::default()
                },
            )?;
        }
    }
    let mut outgoing_metadata = serde_json::json!({});
    if json_request {
        for path in [
            "/stream",
            "/speed",
            "/provider",
            "/reasoning",
            "/reasoning_effort",
        ] {
            if let Some(value) = body.value_at(path)? {
                crate::routing::set_json_pointer(&mut outgoing_metadata, path, value)?;
            }
        }
    }
    let outgoing_stream = outgoing_metadata
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let outgoing_fields = if json_request {
        body.fields(body.root())?.into_keys().collect()
    } else {
        Vec::new()
    };
    Ok(Prepared {
        body,
        route,
        requested_stream,
        outgoing_stream,
        search_tools,
        original_model,
        outgoing_fields,
        original_bytes,
        outgoing_metadata,
    })
}
