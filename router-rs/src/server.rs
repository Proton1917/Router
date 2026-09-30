use std::{collections::HashMap, net::SocketAddr, path::PathBuf};

use anyhow::{Result, bail};
use clap::Parser;
use serde::Deserialize;

#[derive(Debug, Parser)]
#[command(version, about = "Configuration-driven HTTP inference router")]
pub struct Arguments {
    #[arg(long, env = "ROUTER_CONFIG")]
    pub config: PathBuf,
    #[arg(long, env = "ROUTER_ADDR")]
    pub listen: Option<SocketAddr>,
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub health_path: String,
    pub max_request_bytes: usize,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub dry_run_header: String,
    pub dry_run_values: Vec<String>,
    pub endpoints: HashMap<String, Protocol>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Messages,
    Responses,
    ChatCompletions,
    Models,
    Passthrough,
}

impl ServerConfig {
    pub fn protocol_for(&self, path: &str) -> Option<Protocol> {
        self.endpoints.get(path).copied().or_else(|| {
            self.endpoints
                .iter()
                .filter(|(pattern, _)| path_matches(pattern, path))
                .max_by_key(|(pattern, _)| pattern.len())
                .map(|(_, protocol)| *protocol)
        })
    }

    pub fn validate(&self) -> Result<()> {
        if !self.health_path.starts_with('/') || self.health_path.contains('?') {
            bail!("server.health_path must be an absolute URL path");
        }
        if self.max_request_bytes == 0
            || self.connect_timeout_ms == 0
            || self.request_timeout_ms == 0
        {
            bail!("server request limits and timeouts must be positive");
        }
        self.dry_run_header.parse::<http::HeaderName>()?;
        if self.dry_run_values.is_empty() || self.dry_run_values.iter().any(String::is_empty) {
            bail!("server.dry_run_values must contain nonempty values");
        }
        if self.endpoints.is_empty() {
            bail!("server.endpoints must not be empty");
        }
        for path in self.endpoints.keys() {
            if !valid_path_pattern(path) || path == &self.health_path {
                bail!("server endpoint paths must be absolute and distinct from health_path");
            }
        }
        Ok(())
    }
}

pub fn valid_path_pattern(pattern: &str) -> bool {
    (pattern == "*" || pattern.starts_with('/'))
        && !pattern.contains(['?', '#'])
        && !pattern.strip_suffix('*').unwrap_or(pattern).contains('*')
}

pub fn path_matches(pattern: &str, path: &str) -> bool {
    pattern == path
        || pattern
            .strip_suffix('*')
            .is_some_and(|prefix| path.starts_with(prefix))
}
