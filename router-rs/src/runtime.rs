use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub server: Option<crate::server::ServerConfig>,
    pub backends: HashMap<String, BackendConfig>,
    pub clients: Vec<ClientConfig>,
    pub selection: SelectionPolicy,
    pub reasoning_policies: HashMap<String, ReasoningPolicy>,
    pub strip_request_headers: Vec<String>,
    pub cors: CorsPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    pub base_url: String,
    #[serde(default)]
    pub path_rewrites: HashMap<String, String>,
    #[serde(default)]
    pub provider_fields: HashMap<String, Value>,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub web_search: Option<WebSearchPolicy>,
    #[serde(default)]
    pub filter_sse_done: bool,
    #[serde(default)]
    pub rewrite_model_catalog: bool,
    #[serde(default)]
    pub cache: Option<CachePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub token_file: Option<String>,
    #[serde(default)]
    pub token_command: Vec<String>,
    #[serde(default)]
    pub error_hint: Option<String>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchPolicy {
    pub max_output_tokens: u64,
    pub reasoning_effort: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachePolicy {
    pub paths: Vec<String>,
    pub max_ttl_seconds: u32,
    pub request_enable_header: String,
    pub request_enable_value: String,
    pub request_ttl_header: String,
    pub response_status_header: String,
    pub response_age_header: String,
    pub response_ttl_header: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderPredicate {
    pub name: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub before_delimiter: Option<String>,
}

impl HeaderPredicate {
    pub fn matches(&self, headers: &HeaderMap) -> bool {
        let Some(raw) = headers
            .get(&self.name)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        let actual = self.before_delimiter.as_deref().map_or(raw, |delimiter| {
            raw.split_once(delimiter)
                .map_or(raw, |(first, _)| first)
                .trim()
        });
        (self.values.is_empty() && self.contains.is_empty())
            || self
                .values
                .iter()
                .any(|value| value.eq_ignore_ascii_case(actual))
            || self.contains.iter().any(|value| actual.contains(value))
    }

    pub fn validate(&self) -> Result<()> {
        validate_header(&self.name)?;
        if self.before_delimiter.as_ref().is_some_and(String::is_empty)
            || self.contains.iter().any(String::is_empty)
        {
            bail!("header match delimiters and substrings must not be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub id: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub headers: Vec<HeaderPredicate>,
    #[serde(default)]
    pub any_headers: Vec<HeaderPredicate>,
    #[serde(default)]
    pub absent_headers: Vec<String>,
    #[serde(default)]
    pub settings: Option<SettingsSource>,
    #[serde(default)]
    pub response_errors: Option<ResponseErrorPolicy>,
    #[serde(default, rename = "message_compatibility", alias = "word_gateway")]
    pub word_gateway: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseErrorPolicy {
    pub request_headers: Vec<HeaderPredicate>,
    pub status_map: HashMap<u16, u16>,
    pub original_status_header: String,
}

impl ResponseErrorPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.request_headers.is_empty() || self.status_map.is_empty() {
            bail!("response error policy requires request predicates and status mappings");
        }
        for predicate in &self.request_headers {
            predicate.validate()?;
        }
        validate_header(&self.original_status_header)?;
        if !self
            .original_status_header
            .to_ascii_lowercase()
            .starts_with("x-")
        {
            bail!("original status must use a custom x- response header");
        }
        for (&source, &target) in &self.status_map {
            if !(400..=599).contains(&source) || !(400..=599).contains(&target) || source == target
            {
                bail!("response status mappings must preserve errors and change the status");
            }
        }
        Ok(())
    }

    pub fn apply(
        &self,
        status: http::StatusCode,
        request_headers: &HeaderMap,
        response_headers: &mut HeaderMap,
    ) -> Result<http::StatusCode> {
        if !self
            .request_headers
            .iter()
            .all(|predicate| predicate.matches(request_headers))
        {
            return Ok(status);
        }
        let Some(&mapped) = self.status_map.get(&status.as_u16()) else {
            return Ok(status);
        };
        response_headers.insert(
            self.original_status_header.parse::<http::HeaderName>()?,
            http::HeaderValue::from_str(status.as_str())?,
        );
        Ok(http::StatusCode::from_u16(mapped)?)
    }
}

impl ClientConfig {
    pub fn is_catch_all(&self) -> bool {
        self.paths.is_empty()
            && self.headers.is_empty()
            && self.any_headers.is_empty()
            && self.absent_headers.is_empty()
    }

    pub fn matches(&self, headers: &HeaderMap, path: &str) -> bool {
        (self.paths.is_empty()
            || self
                .paths
                .iter()
                .any(|candidate| crate::server::path_matches(candidate, path)))
            && self
                .headers
                .iter()
                .all(|predicate| predicate.matches(headers))
            && (self.any_headers.is_empty()
                || self
                    .any_headers
                    .iter()
                    .any(|predicate| predicate.matches(headers)))
            && self
                .absent_headers
                .iter()
                .all(|name| !headers.contains_key(name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSource {
    pub path: String,
    #[serde(default)]
    pub path_env: Vec<String>,
    pub model_pointer: String,
    pub fast_mode_pointer: String,
    #[serde(default)]
    pub model_header: Option<HeaderParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderParameter {
    pub header: String,
    pub parameter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionPolicy {
    pub model_pointer: String,
    pub speed_pointer: String,
    pub fast_value: String,
    #[serde(default)]
    pub speed_sources: Vec<SpeedSource>,
    pub ignored_model_suffixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedSource {
    pub pointer: String,
    pub fast_values: Vec<String>,
    #[serde(default)]
    pub contexts: Vec<String>,
}

impl SpeedSource {
    pub fn applies_to(&self, context: &str) -> bool {
        self.contexts.is_empty() || self.contexts.iter().any(|id| id == context)
    }
}

impl SelectionPolicy {
    pub fn requested_fast(&self, context: &str, body: &Value) -> Option<bool> {
        body.pointer(&self.speed_pointer)
            .and_then(Value::as_str)
            .map(|speed| speed.eq_ignore_ascii_case(&self.fast_value))
            .or_else(|| {
                self.speed_sources.iter().find_map(|source| {
                    if !source.applies_to(context) {
                        return None;
                    }
                    body.pointer(&source.pointer)
                        .and_then(Value::as_str)
                        .map(|speed| {
                            source
                                .fast_values
                                .iter()
                                .any(|value| speed.eq_ignore_ascii_case(value))
                        })
                })
            })
    }

    pub fn model_ids_match(&self, left: &str, right: &str) -> bool {
        let normalize = |value: &str| {
            let value = value.to_ascii_lowercase();
            for suffix in &self.ignored_model_suffixes {
                if let Some(stripped) = value.strip_suffix(&suffix.to_ascii_lowercase()) {
                    return stripped.to_string();
                }
            }
            value
        };
        normalize(left) == normalize(right)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningPolicy {
    pub sources: Vec<String>,
    pub destination: String,
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub value_map: HashMap<String, String>,
    #[serde(default)]
    pub fallback: Option<ValueFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueFallback {
    pub path: String,
    pub equals: Value,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadNormalization {
    #[serde(default)]
    pub message_role_map: HashMap<String, String>,
    #[serde(default)]
    pub tool_defaults: Option<ToolDefaults>,
    #[serde(default)]
    pub normalize_object_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefaults {
    pub description_template: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorsPolicy {
    pub allow_origin: String,
    pub reflect_request_origin: bool,
    pub allow_methods: String,
    pub allow_headers: String,
    pub expose_headers: String,
    pub allow_private_network: bool,
    pub max_age_seconds: u32,
}

impl RuntimeConfig {
    pub fn resolve_paths(&mut self, directory: &std::path::Path) {
        if let Some(server) = &mut self.server
            && !server.body_processing.spool_directory.is_absolute()
        {
            server.body_processing.spool_directory =
                directory.join(&server.body_processing.spool_directory);
        }
        let resolve = |value: &mut String| {
            if !std::path::Path::new(value).is_absolute() {
                *value = directory.join(&*value).to_string_lossy().into_owned();
            }
        };
        for backend in self.backends.values_mut() {
            if let Some(auth) = &mut backend.auth {
                if let Some(path) = &mut auth.token_file {
                    resolve(path);
                }
                if let Some(program) = auth.token_command.first_mut() {
                    resolve(program);
                }
            }
        }
        for client in &mut self.clients {
            if let Some(settings) = &mut client.settings {
                resolve(&mut settings.path);
            }
        }
    }

    pub fn client_for(&self, headers: &HeaderMap, path: &str) -> Result<&ClientConfig> {
        self.clients
            .iter()
            .find(|client| client.matches(headers, path))
            .context("no client matched")
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(server) = &self.server {
            server.validate()?;
        }
        if self.backends.is_empty() || self.clients.is_empty() {
            bail!("runtime backends and clients must not be empty");
        }
        for (id, backend) in &self.backends {
            validate_id(id)?;
            for pointer in backend.provider_fields.keys() {
                validate_pointer(pointer)?;
            }
            for (source, target) in &backend.path_rewrites {
                if [source, target]
                    .iter()
                    .any(|path| !path.starts_with('/') || path.contains('?') || path.contains('#'))
                {
                    bail!("backend {id} path rewrites require absolute URL paths");
                }
            }
            let url = reqwest::Url::parse(&backend.base_url).context("invalid backend base URL")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                bail!(
                    "backend {id} needs an HTTP(S) base URL without credentials, query or fragment"
                );
            }
            if let Some(auth) = &backend.auth {
                let sources = usize::from(auth.token_env.is_some())
                    + usize::from(auth.token_file.is_some())
                    + usize::from(!auth.token_command.is_empty());
                if sources != 1 || auth.headers.is_empty() {
                    bail!("backend {id} auth needs exactly one token source and nonempty headers");
                }
                if let Some(name) = &auth.token_env {
                    validate_env_name(name)?;
                }
                if auth
                    .token_file
                    .as_ref()
                    .is_some_and(|path| path.trim().is_empty())
                {
                    bail!("backend {id} token file path must not be empty");
                }
                if let Some(program) = auth.token_command.first()
                    && program.trim().is_empty()
                {
                    bail!("backend {id} token command must name an executable");
                }
                for (name, value) in &auth.headers {
                    validate_header(name)?;
                    if value.matches("{token}").count() != 1 {
                        bail!("backend {id} auth header must contain one {{token}}");
                    }
                    validate_header_value(&value.replace("{token}", "placeholder"))?;
                }
            }
            if let Some(search) = &backend.web_search
                && (search.max_output_tokens == 0 || search.reasoning_effort.trim().is_empty())
            {
                bail!("backend {id} has invalid web search policy");
            }
            if let Some(cache) = &backend.cache {
                if cache.max_ttl_seconds == 0
                    || cache.paths.is_empty()
                    || cache
                        .paths
                        .iter()
                        .any(|path| !path.starts_with('/') || path.contains('?'))
                {
                    bail!("backend {id} has invalid cache limits or paths");
                }
                for name in [
                    &cache.request_enable_header,
                    &cache.request_ttl_header,
                    &cache.response_status_header,
                    &cache.response_age_header,
                    &cache.response_ttl_header,
                ] {
                    validate_header(name)?;
                }
                validate_header_value(&cache.request_enable_value)?;
            }
        }
        let mut ids = HashSet::new();
        for (index, client) in self.clients.iter().enumerate() {
            validate_id(&client.id)?;
            if client
                .paths
                .iter()
                .any(|path| !crate::server::valid_path_pattern(path))
            {
                bail!("client {} has invalid paths", client.id);
            }
            if !ids.insert(&client.id) {
                bail!("duplicate client id {}", client.id);
            }
            if client.is_catch_all() && index + 1 != self.clients.len() {
                bail!("only the final client may be a catch-all");
            }
            for predicate in client.headers.iter().chain(&client.any_headers) {
                predicate.validate()?;
            }
            for name in &client.absent_headers {
                validate_header(name)?;
            }
            if let Some(policy) = &client.response_errors {
                policy.validate()?;
            }
            if let Some(settings) = &client.settings {
                if settings.path.trim().is_empty() {
                    bail!("settings path must not be empty");
                }
                for name in &settings.path_env {
                    validate_env_name(name)?;
                }
                validate_pointer(&settings.model_pointer)?;
                validate_pointer(&settings.fast_mode_pointer)?;
                if let Some(header) = &settings.model_header {
                    validate_header(&header.header)?;
                    if header.parameter.is_empty() {
                        bail!("model header parameter must not be empty");
                    }
                }
            }
        }
        if !self.clients.last().is_some_and(ClientConfig::is_catch_all) {
            bail!("final client must be a catch-all");
        }
        validate_pointer(&self.selection.model_pointer)?;
        validate_pointer(&self.selection.speed_pointer)?;
        for source in &self.selection.speed_sources {
            validate_pointer(&source.pointer)?;
            if source.fast_values.is_empty() || source.fast_values.iter().any(String::is_empty) {
                bail!("speed source must declare nonempty fast values");
            }
            for context in &source.contexts {
                if !self.clients.iter().any(|client| &client.id == context) {
                    bail!("speed source references unknown client context {context}");
                }
            }
        }
        if self.selection.fast_value.is_empty()
            || self
                .selection
                .ignored_model_suffixes
                .iter()
                .any(String::is_empty)
        {
            bail!("selection values and suffixes must not be empty");
        }
        for (id, policy) in &self.reasoning_policies {
            validate_id(id)?;
            if policy.sources.is_empty() {
                bail!("reasoning policy {id} has no source");
            }
            for path in policy
                .sources
                .iter()
                .chain(&policy.remove)
                .chain(std::iter::once(&policy.destination))
            {
                validate_pointer(path)?;
            }
            if let Some(fallback) = &policy.fallback {
                validate_pointer(&fallback.path)?;
            }
        }
        for name in &self.strip_request_headers {
            validate_header(name)?;
        }
        for value in [
            &self.cors.allow_origin,
            &self.cors.allow_methods,
            &self.cors.allow_headers,
            &self.cors.expose_headers,
        ] {
            validate_header_value(value)?;
        }
        Ok(())
    }
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        bail!("invalid runtime identifier");
    }
    Ok(())
}

fn validate_env_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with(|c: char| c.is_ascii_digit())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        bail!("invalid environment variable name");
    }
    Ok(())
}

fn validate_header(value: &str) -> Result<()> {
    value
        .parse::<http::HeaderName>()
        .context("invalid configured header name")?;
    Ok(())
}

fn validate_header_value(value: &str) -> Result<()> {
    value
        .parse::<http::HeaderValue>()
        .context("invalid configured header value")?;
    Ok(())
}

pub(crate) fn validate_pointer(value: &str) -> Result<()> {
    if !value.starts_with('/')
        || value == "/"
        || value.split('/').skip(1).any(|part| {
            part.is_empty()
                || part == "*"
                || part.parse::<usize>().is_ok()
                || part.replace("~0", "").replace("~1", "").contains('~')
        })
    {
        bail!("runtime field path must be a non-root object JSON Pointer");
    }
    Ok(())
}
