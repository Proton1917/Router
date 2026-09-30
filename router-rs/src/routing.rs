use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

use crate::runtime::{
    HeaderPredicate, PayloadNormalization, ReasoningPolicy, RuntimeConfig, SelectionPolicy,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterConfig {
    pub schema_version: u32,
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub management: Option<crate::control::Management>,
    pub profiles: HashMap<String, ModelProfile>,
    pub routes: Vec<RouteRule>,
    #[serde(default, rename = "catalog", alias = "word_gateway")]
    pub word_gateway: CatalogConfig,
    #[serde(default)]
    pub validation_cases: Vec<ValidationCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    pub standard: Destination,
    #[serde(default)]
    pub fast: Option<Destination>,
    #[serde(default)]
    pub provider_override: Option<ProviderOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub target: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_template: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub reasoning_adapter: Option<String>,
    #[serde(default)]
    pub force_nonstream: bool,
    #[serde(default)]
    pub payload_normalization: PayloadNormalization,
    #[serde(default)]
    pub synthesize_anthropic_sse_for: Vec<String>,
    #[serde(default)]
    pub response_cache_ttl_seconds: Option<u32>,
    #[serde(default)]
    pub field_policy: FieldPolicy,
    #[serde(default)]
    pub header_policy: HeaderPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldPolicy {
    #[serde(default)]
    pub retain_top_level: Option<Vec<String>>,
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub rename: HashMap<String, String>,
    #[serde(default)]
    pub set: HashMap<String, Value>,
    #[serde(default)]
    pub replace_text_lines: Vec<TextLineReplacement>,
    #[serde(default)]
    pub drop_null: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextLineReplacement {
    pub paths: Vec<String>,
    pub prefix: String,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderPolicy {
    #[serde(default)]
    pub forward_only: Option<Vec<String>>,
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub remove_prefixes: Vec<String>,
    #[serde(default)]
    pub set: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderOverride {
    pub header: String,
    pub value_key: String,
    pub mode_key: String,
    pub mode_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRule {
    pub id: String,
    #[serde(rename = "match")]
    pub matcher: RouteMatch,
    pub profile: String,
    #[serde(default)]
    pub use_settings_fast_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteMatch {
    #[serde(default)]
    pub contexts: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_prefixes: Vec<String>,
    #[serde(default)]
    pub model_contains: Vec<String>,
    #[serde(default)]
    pub settings_models: Vec<String>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub headers: Vec<HeaderPredicate>,
    #[serde(default)]
    pub absent_headers: Vec<String>,
    #[serde(default)]
    pub last_user_contains_all: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogConfig {
    #[serde(default)]
    pub probe_model: String,
    #[serde(default)]
    pub models: Vec<WordModel>,
    #[serde(default)]
    pub model_defaults: serde_json::Map<String, Value>,
    #[serde(default)]
    pub force_nonstream_when_requested: bool,
    #[serde(default)]
    pub remove_tools_when_nonstream: bool,
    #[serde(default)]
    pub max_tokens_when_nonstream: Option<TokenRange>,
    #[serde(default)]
    pub response_model_prefix_to_strip: Option<String>,
    #[serde(default)]
    pub response_model_required_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordModel {
    pub id: String,
    pub name: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub supports_reasoning: bool,
    #[serde(default)]
    pub metadata: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRange {
    pub min: u64,
    pub max: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationCase {
    pub name: String,
    pub context: String,
    pub model: String,
    #[serde(default)]
    pub speed: Option<String>,
    #[serde(default)]
    pub settings_fast_mode: bool,
    #[serde(default)]
    pub settings_model: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub last_user_content: Option<String>,
    pub expected: ValidationExpected,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationExpected {
    pub rule_id: String,
    pub target: String,
    pub model: String,
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoute {
    pub rule_id: String,
    pub target: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub reasoning_policy: Option<ReasoningPolicy>,
    pub force_nonstream: bool,
    pub payload_normalization: PayloadNormalization,
    pub synthesize_anthropic_sse: bool,
    pub response_cache_ttl_seconds: Option<u32>,
    pub field_policy: FieldPolicy,
    pub header_policy: HeaderPolicy,
}

struct MatchBody<'a> {
    value: &'a Value,
    known_markers: Option<&'a [String]>,
}

impl RouterConfig {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let config: Self = serde_json::from_slice(bytes).context("config must be valid JSON")?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if let Some(management) = &self.management {
            management.validate()?;
        }
        if self.schema_version != 2 {
            bail!("schema_version must be 2");
        }
        self.runtime.validate()?;
        if self.profiles.is_empty() {
            bail!("profiles must not be empty");
        }
        if self.routes.is_empty() {
            bail!("routes must not be empty");
        }

        for (name, profile) in &self.profiles {
            validate_identifier(name, &format!("profiles.{name}"))?;
            validate_destination(&profile.standard, &format!("profiles.{name}.standard"))?;
            if let Some(fast) = &profile.fast {
                validate_destination(fast, &format!("profiles.{name}.fast"))?;
            }
            for destination in std::iter::once(&profile.standard).chain(profile.fast.iter()) {
                let backend = self
                    .runtime
                    .backends
                    .get(&destination.target)
                    .with_context(|| {
                        format!(
                            "profile {name} references missing backend {}",
                            destination.target
                        )
                    })?;
                if let Some(policy) = &destination.reasoning_adapter
                    && !self.runtime.reasoning_policies.contains_key(policy)
                {
                    bail!("profile {name} references missing reasoning policy {policy}");
                }
                if let Some(ttl) = destination.response_cache_ttl_seconds
                    && backend
                        .cache
                        .as_ref()
                        .is_none_or(|cache| ttl == 0 || ttl > cache.max_ttl_seconds)
                {
                    bail!("profile {name} cache TTL is unsupported by its backend policy");
                }
            }
            if let Some(provider_override) = &profile.provider_override {
                validate_header_name(
                    &provider_override.header,
                    &format!("profiles.{name}.provider_override.header"),
                )?;
                for (field, value) in [
                    ("value_key", &provider_override.value_key),
                    ("mode_key", &provider_override.mode_key),
                    ("mode_value", &provider_override.mode_value),
                ] {
                    if value.trim().is_empty() {
                        bail!("profiles.{name}.provider_override.{field} must not be empty");
                    }
                }
            }
        }

        let mut route_ids = HashSet::new();
        let mut referenced_profiles = HashSet::new();
        for (index, route) in self.routes.iter().enumerate() {
            validate_identifier(&route.id, &format!("routes[{index}].id"))?;
            if !route_ids.insert(route.id.to_ascii_lowercase()) {
                bail!("duplicate route id {}", route.id);
            }
            if !self.profiles.contains_key(&route.profile) {
                bail!(
                    "route {} references missing profile {}",
                    route.id,
                    route.profile
                );
            }
            for context in &route.matcher.contexts {
                if !self
                    .runtime
                    .clients
                    .iter()
                    .any(|client| &client.id == context)
                {
                    bail!("route {} references missing client {context}", route.id);
                }
            }
            referenced_profiles.insert(route.profile.as_str());
            validate_matcher(&route.matcher, &format!("routes[{index}].match"))?;
        }
        if let Some(unused) = self
            .profiles
            .keys()
            .find(|profile| !referenced_profiles.contains(profile.as_str()))
        {
            bail!("profile {unused} is not referenced by any route");
        }

        if !self
            .routes
            .last()
            .is_some_and(|route| route.matcher.is_catch_all())
        {
            bail!("the final route must be a catch-all rule");
        }

        if self
            .runtime
            .clients
            .iter()
            .any(|client| client.word_gateway)
            && self.word_gateway.models.is_empty()
        {
            bail!("clients using message compatibility require a nonempty catalog");
        }
        if let Some(range) = self.word_gateway.max_tokens_when_nonstream
            && (range.min == 0 || range.max < range.min)
        {
            bail!("word_gateway.max_tokens_when_nonstream is invalid");
        }
        let mut word_ids = HashSet::new();
        for (index, model) in self.word_gateway.models.iter().enumerate() {
            validate_model_id(&model.id, &format!("word_gateway.models[{index}].id"))?;
            if !word_ids.insert(model.id.to_ascii_lowercase()) {
                bail!("duplicate Word model id {}", model.id);
            }
            if model.name.trim().is_empty()
                || model.context_window == 0
                || model.max_output_tokens == 0
            {
                bail!("word_gateway.models[{index}] has invalid metadata");
            }
        }
        if self.validation_cases.is_empty() {
            bail!("validation_cases must not be empty");
        }
        let mut case_names = HashSet::new();
        for (index, case) in self.validation_cases.iter().enumerate() {
            if !case_names.insert(case.name.to_ascii_lowercase()) {
                bail!("duplicate validation case name {}", case.name);
            }
            validate_model_id(&case.model, &format!("validation_cases[{index}].model"))?;
            if let Some(settings_model) = &case.settings_model {
                validate_model_id(
                    settings_model,
                    &format!("validation_cases[{index}].settings_model"),
                )?;
            }
            let mut headers = HeaderMap::new();
            for (name, value) in &case.headers {
                let name = name
                    .parse::<http::HeaderName>()
                    .with_context(|| format!("validation case {} has invalid header", case.name))?;
                let value = value.parse::<http::HeaderValue>().with_context(|| {
                    format!("validation case {} has invalid header value", case.name)
                })?;
                headers.insert(name, value);
            }
            let mut body = serde_json::json!({
                "model": case.model,
                "messages": [{
                    "role": "user",
                    "content": case.last_user_content.as_deref().unwrap_or("validation")
                }]
            });
            if let Some(speed) = &case.speed {
                body["speed"] = Value::String(speed.clone());
            }
            if !self
                .runtime
                .clients
                .iter()
                .any(|client| client.id == case.context)
            {
                bail!("validation case {} references missing client", case.name);
            }
            let resolved = self.resolve(
                &case.context,
                &headers,
                &body,
                case.settings_fast_mode,
                case.settings_model.as_deref(),
            )?;
            if resolved.rule_id != case.expected.rule_id
                || resolved.target != case.expected.target
                || resolved.model.as_deref() != Some(case.expected.model.as_str())
                || resolved.provider != case.expected.provider
            {
                bail!(
                    "validation case {} failed: got rule={} target={:?} model={:?} provider={:?}",
                    case.name,
                    resolved.rule_id,
                    resolved.target,
                    resolved.model,
                    resolved.provider
                );
            }
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        context: &str,
        headers: &HeaderMap,
        body: &Value,
        settings_fast_mode: bool,
        settings_model: Option<&str>,
    ) -> Result<ResolvedRoute> {
        self.resolve_with_markers(
            context,
            headers,
            body,
            settings_fast_mode,
            settings_model,
            None,
        )
    }

    pub fn resolve_with_markers(
        &self,
        context: &str,
        headers: &HeaderMap,
        body: &Value,
        settings_fast_mode: bool,
        settings_model: Option<&str>,
        known_markers: Option<&[String]>,
    ) -> Result<ResolvedRoute> {
        let model = body
            .pointer(&self.runtime.selection.model_pointer)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let explicit_fast = body
            .pointer(&self.runtime.selection.speed_pointer)
            .and_then(Value::as_str)
            .map(|speed| speed.eq_ignore_ascii_case(&self.runtime.selection.fast_value));
        let route = self
            .routes
            .iter()
            .find(|route| {
                route.matcher.matches(
                    context,
                    headers,
                    MatchBody {
                        value: body,
                        known_markers,
                    },
                    model,
                    settings_model,
                    &self.runtime.selection,
                )
            })
            .context("no route matched; configuration is missing a catch-all rule")?;
        let fast_mode = explicit_fast.unwrap_or(route.use_settings_fast_mode && settings_fast_mode);
        let profile = self
            .profiles
            .get(&route.profile)
            .with_context(|| format!("route {} references a missing profile", route.id))?;
        let destination = if fast_mode {
            profile.fast.as_ref().unwrap_or(&profile.standard)
        } else {
            &profile.standard
        };

        let mut provider = destination.provider.clone();
        if let Some(spec) = &profile.provider_override
            && let Some(overridden) = provider_override(headers, spec)
        {
            provider = Some(overridden);
        }
        let synthesize_anthropic_sse = provider.as_deref().is_some_and(|selected| {
            destination
                .synthesize_anthropic_sse_for
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(selected))
        });
        let outgoing_model = match (&destination.model, &destination.model_template) {
            (Some(model), None) => Some(model.clone()),
            (None, Some(template)) => Some(template.replace("{model}", model)),
            (None, None) => Some(model.to_string()),
            (Some(_), Some(_)) => unreachable!("validated destination"),
        };

        Ok(ResolvedRoute {
            rule_id: route.id.clone(),
            target: destination.target.clone(),
            model: outgoing_model,
            provider,
            reasoning_policy: destination
                .reasoning_adapter
                .as_ref()
                .and_then(|name| self.runtime.reasoning_policies.get(name))
                .cloned(),
            force_nonstream: destination.force_nonstream || synthesize_anthropic_sse,
            payload_normalization: destination.payload_normalization.clone(),
            synthesize_anthropic_sse,
            response_cache_ttl_seconds: destination.response_cache_ttl_seconds,
            field_policy: destination.field_policy.clone(),
            header_policy: destination.header_policy.clone(),
        })
    }
}

impl RouteMatch {
    fn is_catch_all(&self) -> bool {
        self.contexts.is_empty()
            && self.models.is_empty()
            && self.model_prefixes.is_empty()
            && self.model_contains.is_empty()
            && self.settings_models.is_empty()
            && self.stream.is_none()
            && self.headers.is_empty()
            && self.absent_headers.is_empty()
            && self.last_user_contains_all.is_empty()
    }

    fn matches(
        &self,
        context: &str,
        headers: &HeaderMap,
        payload: MatchBody<'_>,
        model: &str,
        settings_model: Option<&str>,
        selection: &SelectionPolicy,
    ) -> bool {
        let MatchBody {
            value: body,
            known_markers,
        } = payload;
        if !self.contexts.is_empty() && !self.contexts.iter().any(|candidate| candidate == context)
        {
            return false;
        }
        let has_model_condition = !self.models.is_empty()
            || !self.model_prefixes.is_empty()
            || !self.model_contains.is_empty();
        if has_model_condition
            && !self
                .models
                .iter()
                .any(|candidate| selection.model_ids_match(candidate, model))
            && !self
                .model_prefixes
                .iter()
                .any(|prefix| starts_with_ignore_ascii_case(model, prefix))
            && !self
                .model_contains
                .iter()
                .any(|needle| contains_ignore_ascii_case(model, needle))
        {
            return false;
        }
        if !self.settings_models.is_empty()
            && !settings_model.is_some_and(|actual| {
                self.settings_models
                    .iter()
                    .any(|expected| selection.model_ids_match(expected, actual))
            })
        {
            return false;
        }
        if let Some(expected_stream) = self.stream
            && body.get("stream").and_then(Value::as_bool).unwrap_or(false) != expected_stream
        {
            return false;
        }
        if self
            .headers
            .iter()
            .any(|predicate| !predicate.matches(headers))
        {
            return false;
        }
        if self
            .absent_headers
            .iter()
            .any(|name| headers.contains_key(name))
        {
            return false;
        }
        if !self.last_user_contains_all.is_empty() {
            if let Some(markers) = known_markers {
                return self
                    .last_user_contains_all
                    .iter()
                    .all(|marker| markers.contains(marker));
            }
            let Some(last_user_content) = last_user_content(body) else {
                return false;
            };
            if self
                .last_user_contains_all
                .iter()
                .any(|needle| !value_contains_text(last_user_content, needle))
            {
                return false;
            }
        }
        true
    }
}

impl ResolvedRoute {
    #[cfg(test)]
    pub fn apply_reasoning_policy(&self, body: &mut Value) -> Result<()> {
        let Some(policy) = &self.reasoning_policy else {
            return Ok(());
        };
        let effort = policy
            .sources
            .iter()
            .find_map(|path| {
                body.pointer(path)
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                policy.fallback.as_ref().and_then(|fallback| {
                    (body.pointer(&fallback.path) == Some(&fallback.equals))
                        .then(|| fallback.value.clone())
                })
            });
        for path in &policy.remove {
            take_json_pointer(body, path);
        }
        if let Some(effort) = effort {
            let mapped = policy.value_map.get(&effort).unwrap_or(&effort);
            set_json_pointer(body, &policy.destination, Value::String(mapped.clone()))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn apply_field_policy(&self, body: &mut Value) -> Result<()> {
        if let Some(retained) = &self.field_policy.retain_top_level {
            let retained = retained.iter().map(String::as_str).collect::<HashSet<_>>();
            let object = body
                .as_object_mut()
                .context("retain_top_level requires a JSON object request body")?;
            object.retain(|key, _| retained.contains(key.as_str()));
        }
        for pointer in &self.field_policy.remove {
            remove_json_path(body, pointer);
        }
        for (from, to) in &self.field_policy.rename {
            if let Some(value) = take_json_pointer(body, from) {
                set_json_pointer(body, to, value)?;
            }
        }
        for (pointer, value) in &self.field_policy.set {
            set_json_pointer(body, pointer, value.clone())?;
        }
        for rule in &self.field_policy.replace_text_lines {
            let replacement = rule
                .replacement
                .replace("{model}", self.model.as_deref().unwrap_or_default());
            for path in &rule.paths {
                let tokens = path
                    .split('/')
                    .skip(1)
                    .map(decode_json_pointer_token)
                    .collect::<Option<Vec<_>>>()
                    .context("invalid text replacement path")?;
                replace_text_lines_at_path(body, &tokens, &rule.prefix, &replacement);
            }
        }
        if self.field_policy.drop_null {
            drop_null_fields(body);
        }
        Ok(())
    }

    pub fn should_forward_header(&self, name: &http::HeaderName) -> bool {
        let name = name.as_str();
        if let Some(forward_only) = &self.header_policy.forward_only
            && !forward_only
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            return false;
        }
        !self
            .header_policy
            .remove
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(name))
            && !self
                .header_policy
                .remove_prefixes
                .iter()
                .any(|prefix| starts_with_ignore_ascii_case(name, prefix))
    }
}

fn validate_destination(destination: &Destination, field: &str) -> Result<()> {
    validate_identifier(&destination.target, &format!("{field}.target"))?;
    match (&destination.model, &destination.model_template) {
        (Some(model), None) => validate_model_id(model, &format!("{field}.model"))?,
        (None, Some(template)) if template.matches("{model}").count() == 1 => {}
        (None, Some(_)) => bail!("{field}.model_template must contain exactly one {{model}}"),
        (None, None) => {}
        (Some(_), Some(_)) => bail!("{field} cannot set both model and model_template"),
    }
    if let Some(provider) = &destination.provider {
        validate_provider(provider, &format!("{field}.provider"))?;
    }
    for (index, provider) in destination.synthesize_anthropic_sse_for.iter().enumerate() {
        validate_provider(
            provider,
            &format!("{field}.synthesize_anthropic_sse_for[{index}]"),
        )?;
    }
    for (from, to) in &destination.payload_normalization.message_role_map {
        if from.trim().is_empty() || to.trim().is_empty() {
            bail!("{field}.payload_normalization contains an empty role");
        }
    }
    if let Some(defaults) = &destination.payload_normalization.tool_defaults
        && (defaults.description_template.trim().is_empty() || !defaults.input_schema.is_object())
    {
        bail!("{field}.payload_normalization has invalid tool defaults");
    }
    validate_field_policy(&destination.field_policy, &format!("{field}.field_policy"))?;
    validate_header_policy(
        &destination.header_policy,
        &format!("{field}.header_policy"),
    )?;
    Ok(())
}

fn validate_field_policy(policy: &FieldPolicy, field: &str) -> Result<()> {
    if let Some(retained) = &policy.retain_top_level {
        if retained.is_empty() {
            bail!("{field}.retain_top_level must not be empty when set");
        }
        for (index, name) in retained.iter().enumerate() {
            if name.is_empty() || name.contains('/') {
                bail!("{field}.retain_top_level[{index}] must be a top-level field name");
            }
        }
    }
    for (index, pointer) in policy.remove.iter().enumerate() {
        validate_remove_path(pointer, &format!("{field}.remove[{index}]"))?;
    }
    for (from, to) in &policy.rename {
        validate_json_pointer(from, &format!("{field}.rename source"))?;
        validate_json_pointer(to, &format!("{field}.rename destination"))?;
    }
    for pointer in policy.set.keys() {
        validate_json_pointer(pointer, &format!("{field}.set key"))?;
    }
    for (index, rule) in policy.replace_text_lines.iter().enumerate() {
        let field = format!("{field}.replace_text_lines[{index}]");
        if rule.paths.is_empty() {
            bail!("{field}.paths must not be empty");
        }
        for (index, path) in rule.paths.iter().enumerate() {
            validate_remove_path(path, &format!("{field}.paths[{index}]"))?;
        }
        if rule.prefix.trim().is_empty() || rule.prefix.contains(['\r', '\n']) {
            bail!("{field}.prefix must be a nonempty single line");
        }
        if rule.replacement.trim().is_empty() || rule.replacement.contains(['\r', '\n']) {
            bail!("{field}.replacement must be a nonempty single line");
        }
    }
    Ok(())
}

fn validate_header_policy(policy: &HeaderPolicy, field: &str) -> Result<()> {
    if let Some(forward_only) = &policy.forward_only {
        for (index, header) in forward_only.iter().enumerate() {
            validate_header_name(header, &format!("{field}.forward_only[{index}]"))?;
        }
    }
    for (index, header) in policy.remove.iter().enumerate() {
        validate_header_name(header, &format!("{field}.remove[{index}]"))?;
    }
    for (index, prefix) in policy.remove_prefixes.iter().enumerate() {
        if prefix.trim().is_empty() {
            bail!("{field}.remove_prefixes[{index}] must not be empty");
        }
    }
    for header in policy.set.keys() {
        validate_header_name(header, &format!("{field}.set key"))?;
    }
    Ok(())
}

fn validate_json_pointer(pointer: &str, field: &str) -> Result<()> {
    if !pointer.starts_with('/') || pointer == "/" {
        bail!("{field} must be a non-root JSON Pointer");
    }
    if pointer
        .split('/')
        .skip(1)
        .any(|token| token.is_empty() || token.parse::<usize>().is_ok())
    {
        bail!("{field} must address object fields, not arrays");
    }
    Ok(())
}

fn validate_remove_path(pointer: &str, field: &str) -> Result<()> {
    if !pointer.starts_with('/') || pointer == "/" {
        bail!("{field} must be a non-root JSON Pointer or wildcard removal path");
    }
    let tokens = pointer.split('/').skip(1).collect::<Vec<_>>();
    if tokens.last().is_some_and(|token| *token == "*") {
        bail!("{field} must name a field to remove, not end in a wildcard");
    }
    for token in tokens {
        if token.is_empty() || (token != "*" && token.parse::<usize>().is_ok()) {
            bail!("{field} must address object fields and use * to traverse arrays");
        }
        if token != "*" && decode_json_pointer_token(token).is_none() {
            bail!("{field} contains an invalid JSON Pointer token");
        }
    }
    Ok(())
}

fn validate_matcher(matcher: &RouteMatch, field: &str) -> Result<()> {
    for (index, model) in matcher.models.iter().enumerate() {
        validate_model_id(model, &format!("{field}.models[{index}]"))?;
    }
    for (index, model) in matcher.settings_models.iter().enumerate() {
        validate_model_id(model, &format!("{field}.settings_models[{index}]"))?;
    }
    for (name, values) in [
        ("model_prefixes", &matcher.model_prefixes),
        ("model_contains", &matcher.model_contains),
        ("last_user_contains_all", &matcher.last_user_contains_all),
    ] {
        for (index, value) in values.iter().enumerate() {
            if value.is_empty() {
                bail!("{field}.{name}[{index}] must not be empty");
            }
        }
    }
    for (index, predicate) in matcher.headers.iter().enumerate() {
        predicate
            .validate()
            .with_context(|| format!("{field}.headers[{index}]"))?;
    }
    for (index, header) in matcher.absent_headers.iter().enumerate() {
        validate_header_name(header, &format!("{field}.absent_headers[{index}]"))?;
    }
    Ok(())
}

fn validate_identifier(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("{field} is not a valid identifier");
    }
    Ok(())
}

fn validate_model_id(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b'/' | b'@' | b':' | b'[' | b']')
        })
    {
        bail!("{field} is not a valid model ID");
    }
    Ok(())
}

fn validate_provider(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        bail!("{field} is not a valid provider slug");
    }
    Ok(())
}

fn validate_header_name(value: &str, field: &str) -> Result<()> {
    if value.parse::<http::HeaderName>().is_err() {
        bail!("{field} is not a valid HTTP header name");
    }
    Ok(())
}

fn provider_override(headers: &HeaderMap, spec: &ProviderOverride) -> Option<String> {
    let header = headers.get(&spec.header)?.to_str().ok()?;
    let mut parameters = HashMap::new();
    for part in header.split(';').skip(1) {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        parameters.insert(key.trim(), value.trim());
    }
    if !parameters
        .get(spec.mode_key.as_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(&spec.mode_value))
    {
        return None;
    }
    let provider = parameters.get(spec.value_key.as_str())?;
    validate_provider(provider, "provider override").ok()?;
    Some((*provider).to_string())
}

#[cfg(test)]
fn take_json_pointer(root: &mut Value, pointer: &str) -> Option<Value> {
    let (parent_pointer, leaf) = split_json_pointer(pointer)?;
    let parent = if parent_pointer.is_empty() {
        root
    } else {
        root.pointer_mut(&parent_pointer)?
    };
    parent
        .as_object_mut()?
        .remove(&decode_json_pointer_token(leaf)?)
}

#[cfg(test)]
fn remove_json_path(root: &mut Value, pointer: &str) {
    let Some(tokens) = pointer
        .split('/')
        .skip(1)
        .map(decode_json_pointer_token)
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    remove_json_path_tokens(root, &tokens);
}

#[cfg(test)]
fn remove_json_path_tokens(current: &mut Value, tokens: &[String]) {
    let Some((head, tail)) = tokens.split_first() else {
        return;
    };
    if tail.is_empty() {
        if let Value::Object(object) = current {
            object.remove(head);
        }
        return;
    }
    if head == "*" {
        match current {
            Value::Array(array) => {
                for value in array {
                    remove_json_path_tokens(value, tail);
                }
            }
            Value::Object(object) => {
                for value in object.values_mut() {
                    remove_json_path_tokens(value, tail);
                }
            }
            _ => {}
        }
    } else if let Value::Object(object) = current
        && let Some(value) = object.get_mut(head)
    {
        remove_json_path_tokens(value, tail);
    }
}

#[cfg(test)]
fn replace_text_lines_at_path(
    current: &mut Value,
    tokens: &[String],
    prefix: &str,
    replacement: &str,
) {
    let Some((head, tail)) = tokens.split_first() else {
        if let Value::String(text) = current {
            let mut rewritten = String::with_capacity(text.len());
            let mut changed = false;
            for line in text.split_inclusive('\n') {
                if line.starts_with(prefix) {
                    rewritten.push_str(replacement);
                    if line.ends_with("\r\n") {
                        rewritten.push_str("\r\n");
                    } else if line.ends_with('\n') {
                        rewritten.push('\n');
                    }
                    changed = true;
                } else {
                    rewritten.push_str(line);
                }
            }
            if changed {
                *text = rewritten;
            }
        }
        return;
    };
    if head == "*" {
        match current {
            Value::Array(array) => {
                for value in array {
                    replace_text_lines_at_path(value, tail, prefix, replacement);
                }
            }
            Value::Object(object) => {
                for value in object.values_mut() {
                    replace_text_lines_at_path(value, tail, prefix, replacement);
                }
            }
            _ => {}
        }
    } else if let Value::Object(object) = current
        && let Some(value) = object.get_mut(head)
    {
        replace_text_lines_at_path(value, tail, prefix, replacement);
    }
}

pub(crate) fn set_json_pointer(root: &mut Value, pointer: &str, value: Value) -> Result<()> {
    let (parent_pointer, leaf) =
        split_json_pointer(pointer).with_context(|| format!("invalid JSON Pointer {pointer}"))?;
    let mut current = root;
    if !parent_pointer.is_empty() {
        for token in parent_pointer.split('/').skip(1) {
            let token = decode_json_pointer_token(token)
                .with_context(|| format!("invalid JSON Pointer token in {pointer}"))?;
            let object = current
                .as_object_mut()
                .with_context(|| format!("JSON Pointer parent is not an object: {pointer}"))?;
            current = object
                .entry(token)
                .or_insert_with(|| Value::Object(Default::default()));
        }
    }
    let object = current
        .as_object_mut()
        .with_context(|| format!("JSON Pointer parent is not an object: {pointer}"))?;
    let leaf = decode_json_pointer_token(leaf)
        .with_context(|| format!("invalid JSON Pointer token in {pointer}"))?;
    object.insert(leaf, value);
    Ok(())
}

fn split_json_pointer(pointer: &str) -> Option<(String, &str)> {
    let index = pointer.rfind('/')?;
    let parent = pointer[..index].to_string();
    let leaf = &pointer[index + 1..];
    (!leaf.is_empty()).then_some((parent, leaf))
}

fn decode_json_pointer_token(token: &str) -> Option<String> {
    let mut decoded = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            decoded.push(ch);
            continue;
        }
        match chars.next()? {
            '0' => decoded.push('~'),
            '1' => decoded.push('/'),
            _ => return None,
        }
    }
    Some(decoded)
}

#[cfg(test)]
fn drop_null_fields(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                drop_null_fields(item);
            }
        }
        Value::Object(object) => {
            object.retain(|_, child| !child.is_null());
            for child in object.values_mut() {
                drop_null_fields(child);
            }
        }
        _ => {}
    }
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn contains_ignore_ascii_case(value: &str, needle: &str) -> bool {
    value
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn last_user_content(json: &Value) -> Option<&Value> {
    json.get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().rev().find_map(|message| {
                (message.get("role").and_then(Value::as_str) == Some("user"))
                    .then(|| message.get("content"))
                    .flatten()
            })
        })
}

fn value_contains_text(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items.iter().any(|item| value_contains_text(item, needle)),
        Value::Object(object) => object
            .values()
            .any(|child| value_contains_text(child, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
      "schema_version": 2,
      "runtime": {"backends": {"openrouter": {"base_url": "https://example.invalid/openrouter"}, "xai_oauth": {"base_url": "https://example.invalid/xai_oauth"}}, "clients": [{"id": "ccor", "headers": [{"name": "x-client", "values": ["1"]}], "settings": {"path": "/tmp/router-settings.json", "model_pointer": "/model", "fast_mode_pointer": "/fastMode"}}, {"id": "desktop"}], "selection": {"model_pointer": "/model", "speed_pointer": "/speed", "fast_value": "fast", "ignored_model_suffixes": ["[1m]"]}, "reasoning_policies": {"openrouter": {"sources": ["/output_config/effort", "/reasoning/effort", "/effort"], "destination": "/reasoning/effort", "remove": ["/output_config", "/thinking", "/effort"], "fallback": {"path": "/thinking/type", "equals": "disabled", "value": "none"}}}, "strip_request_headers": ["x-client"], "cors": {"allow_origin": "*", "reflect_request_origin": true, "allow_methods": "GET,POST,HEAD,OPTIONS", "allow_headers": "content-type", "expose_headers": "request-id", "allow_private_network": true, "max_age_seconds": 86400}},
      "profiles": {
        "default": {"standard": {"target": "openrouter"}},
        "switchable": {
          "standard": {
            "target": "openrouter",
            "model": "vendor/model-standard",
            "provider": "provider/standard",
            "reasoning_adapter": "openrouter",
            "field_policy": {
              "remove": ["/speed"],
              "rename": {"/vendor/old": "/vendor/current"},
              "set": {"/vendor/cache_scope": "stable"},
              "drop_null": true
            },
            "header_policy": {
              "remove": ["x-noisy"],
              "remove_prefixes": ["x-fallback-"],
              "set": {"x-vendor-mode": "stable"}
            }
          },
          "fast": {
            "target": "openrouter",
            "model": "vendor/model-fast"
          }
        },
        "single": {
          "standard": {"target": "xai_oauth", "model": "vendor/model-one"}
        }
      },
      "routes": [
        {
          "id": "selected-model-route",
          "match": {
            "contexts": ["ccor"],
            "models": ["client-fast-alias"],
            "settings_models": ["logical-model"]
          },
          "profile": "switchable",
          "use_settings_fast_mode": true
        },
        {
          "id": "switchable-route",
          "match": {"contexts": ["ccor"], "models": ["logical-model"]},
          "profile": "switchable"
        },
        {
          "id": "single-route",
          "match": {"contexts": ["ccor"], "models": ["single-model"]},
          "profile": "single"
        },
        {"id": "fallback", "match": {}, "profile": "default"}
      ],
      "word_gateway": {
        "probe_model": "probe-model",
        "models": [{
          "id": "probe-model",
          "name": "Probe",
          "context_window": 1000,
          "max_output_tokens": 100,
          "supports_reasoning": false
        }]
      },
      "validation_cases": [{
        "name": "switchable-standard",
        "context": "ccor",
        "model": "logical-model",
        "speed": "standard",
        "expected": {
          "rule_id": "switchable-route",
          "target": "openrouter",
          "model": "vendor/model-standard",
          "provider": "provider/standard"
        }
      }]
    }"#;

    #[test]
    fn each_profile_controls_its_own_fast_destination() {
        let config = RouterConfig::from_json(CONFIG.as_bytes()).expect("valid config");
        let headers = HeaderMap::new();
        let switchable = serde_json::json!({"model": "logical-model", "speed": "fast"});
        let fast = config
            .resolve("ccor", &headers, &switchable, true, None)
            .expect("fast route");
        assert_eq!(fast.model.as_deref(), Some("vendor/model-fast"));
        assert_eq!(fast.provider, None);

        let single = serde_json::json!({"model": "single-model"});
        let unchanged = config
            .resolve("ccor", &headers, &single, true, None)
            .expect("single route");
        assert_eq!(unchanged.model.as_deref(), Some("vendor/model-one"));
        assert_eq!(unchanged.target, "xai_oauth");
    }

    #[test]
    fn shared_fast_setting_is_opt_in_per_route() {
        let config = RouterConfig::from_json(CONFIG.as_bytes()).expect("valid config");
        let direct = serde_json::json!({"model": "logical-model"});
        let standard = config
            .resolve("ccor", &HeaderMap::new(), &direct, true, None)
            .expect("direct standard route");
        assert_eq!(standard.rule_id, "switchable-route");
        assert_eq!(standard.model.as_deref(), Some("vendor/model-standard"));

        let recovered = serde_json::json!({"model": "client-fast-alias"});
        let fast = config
            .resolve(
                "ccor",
                &HeaderMap::new(),
                &recovered,
                true,
                Some("logical-model"),
            )
            .expect("recovered fast route");
        assert_eq!(fast.rule_id, "selected-model-route");
        assert_eq!(fast.model.as_deref(), Some("vendor/model-fast"));
    }

    #[test]
    fn explicit_speed_overrides_settings() {
        let config = RouterConfig::from_json(CONFIG.as_bytes()).expect("valid config");
        let body = serde_json::json!({"model": "logical-model", "speed": "standard"});
        let route = config
            .resolve("ccor", &HeaderMap::new(), &body, true, None)
            .expect("route");
        assert_eq!(route.model.as_deref(), Some("vendor/model-standard"));
    }

    #[test]
    fn selected_settings_model_recovers_profile_from_client_alias() {
        let config = RouterConfig::from_json(CONFIG.as_bytes()).expect("valid config");
        let body = serde_json::json!({"model": "client-fast-alias"});
        let standard = config
            .resolve(
                "ccor",
                &HeaderMap::new(),
                &body,
                false,
                Some("logical-model[1m]"),
            )
            .expect("selected standard route");
        assert_eq!(standard.rule_id, "selected-model-route");
        assert_eq!(standard.model.as_deref(), Some("vendor/model-standard"));

        let fast = config
            .resolve(
                "ccor",
                &HeaderMap::new(),
                &body,
                true,
                Some("logical-model"),
            )
            .expect("selected fast route");
        assert_eq!(fast.rule_id, "selected-model-route");
        assert_eq!(fast.model.as_deref(), Some("vendor/model-fast"));
    }

    #[test]
    fn applies_profile_specific_field_and_header_policies() {
        let config = RouterConfig::from_json(CONFIG.as_bytes()).expect("valid config");
        let mut body = serde_json::json!({
            "model": "logical-model",
            "speed": "standard",
            "unused": null,
            "vendor": {"old": "value", "null_value": null}
        });
        let route = config
            .resolve("ccor", &HeaderMap::new(), &body, false, None)
            .expect("route");
        route.apply_field_policy(&mut body).expect("field policy");
        let mut text_route = route.clone();
        text_route.field_policy.replace_text_lines = vec![TextLineReplacement {
            paths: vec!["/system".to_string(), "/system/*/text".to_string()],
            prefix: "Identity: ".to_string(),
            replacement: "Identity: {model}".to_string(),
        }];
        let mut payload = serde_json::json!({
            "system": [{"type":"text","text":"Keep me.\r\nIdentity: old\r\nKeep this too.\n","cache_control":{"type":"ephemeral"}}],
            "messages":[{"role":"user","content":"Identity: user text must stay"}]
        });
        text_route.apply_field_policy(&mut payload).unwrap();
        assert_eq!(
            payload["system"][0]["text"],
            "Keep me.\r\nIdentity: vendor/model-standard\r\nKeep this too.\n"
        );
        assert_eq!(payload["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            payload["messages"][0]["content"],
            "Identity: user text must stay"
        );
        let before = payload.clone();
        text_route.apply_field_policy(&mut payload).unwrap();
        assert_eq!(payload, before);

        assert!(body.get("speed").is_none());
        assert!(body.get("unused").is_none());
        assert_eq!(body["vendor"]["current"], "value");
        assert_eq!(body["vendor"]["cache_scope"], "stable");
        assert!(body["vendor"].get("null_value").is_none());
        assert!(!route.should_forward_header(&"x-noisy".parse().unwrap()));
        assert!(!route.should_forward_header(&"x-fallback-token".parse().unwrap()));
        assert!(route.should_forward_header(&"anthropic-version".parse().unwrap()));
        assert_eq!(
            route
                .header_policy
                .set
                .get("x-vendor-mode")
                .map(String::as_str),
            Some("stable")
        );
    }

    #[test]
    fn removes_fields_through_wildcard_array_paths() {
        let mut body = serde_json::json!({
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "one", "is_error": false},
                        {"type": "text", "text": "keep"}
                    ]
                },
                {"role": "assistant", "content": "plain text"},
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "two", "is_error": true}
                    ]
                }
            ]
        });

        remove_json_path(&mut body, "/messages/*/content/*/is_error");

        assert!(body["messages"][0]["content"][0].get("is_error").is_none());
        assert_eq!(body["messages"][0]["content"][0]["tool_use_id"], "one");
        assert_eq!(body["messages"][0]["content"][1]["text"], "keep");
        assert!(body["messages"][2]["content"][0].get("is_error").is_none());
    }

    #[test]
    fn invalid_or_incomplete_configs_fail_closed() {
        let missing_catch_all = CONFIG.replace(
            ",\n        {\"id\": \"fallback\", \"match\": {}, \"profile\": \"default\"}",
            "",
        );
        assert!(RouterConfig::from_json(missing_catch_all.as_bytes()).is_err());
        assert!(RouterConfig::from_json(br#"{}"#).is_err());
        let original: Value = serde_json::from_str(CONFIG).unwrap();
        for pointer in [
            "/runtime/backends",
            "/runtime/clients",
            "/runtime/reasoning_policies",
        ] {
            let mut config = original.clone();
            if let Some(value) = config.pointer_mut(pointer).unwrap().as_object_mut() {
                value.clear();
            }
            if let Some(values) = config.pointer_mut(pointer).unwrap().as_array_mut() {
                values.clear();
            }
            assert!(RouterConfig::from_json(config.to_string().as_bytes()).is_err());
        }
        let mut config = original.clone();
        config["profiles"]["switchable"]["standard"]["target"] =
            Value::String("new-gateway".to_string());
        assert!(RouterConfig::from_json(config.to_string().as_bytes()).is_err());
        config["runtime"]["backends"]["new-gateway"] = serde_json::json!({
            "base_url": "https://example.invalid/new-prefix", "auth": {"token_file":"/tmp/credential", "headers":{"x-custom-auth":"Token {token}"}}
        });
        config["validation_cases"][0]["expected"]["target"] =
            Value::String("new-gateway".to_string());
        assert!(RouterConfig::from_json(config.to_string().as_bytes()).is_ok());
        config["runtime"]["backends"]["new-gateway"]["auth"]["token_env"] =
            Value::String("EXAMPLE_KEY".to_string());
        assert!(RouterConfig::from_json(config.to_string().as_bytes()).is_err());
    }
}
