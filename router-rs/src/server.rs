use std::{collections::HashMap, net::SocketAddr, path::PathBuf};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use serde::Deserialize;

#[derive(Debug, Parser)]
#[command(version, about = "Configuration-driven HTTP inference router")]
pub struct Arguments {
    #[arg(long, env = "ROUTER_CONFIG", global = true)]
    pub config: Option<PathBuf>,
    #[arg(long, env = "ROUTER_ADDR", global = true)]
    pub listen: Option<SocketAddr>,
    #[arg(long)]
    pub check: bool,
    #[command(subcommand)]
    pub command: Option<RouterCommand>,
}

#[derive(Debug, Subcommand)]
pub enum RouterCommand {
    /// 启动转发服务
    Serve,
    /// 启动管理界面与转发服务，并打开浏览器
    Web {
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        restart: bool,
        #[arg(long, hide = true)]
        foreground: bool,
    },
    /// 查看服务和配置状态
    Status,
    /// 管理可自由命名的启动命令
    Commands {
        #[command(subcommand)]
        action: CommandAction,
    },
    /// 管理 API 后端
    Backends {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// 管理模型配置
    Models {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// 管理客户端启动方式
    Templates {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// 管理有序路由规则
    Routes {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// 查看、校验或应用完整配置
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// 将凭据保存到受保护的文件
    Credentials {
        #[command(subcommand)]
        action: CredentialAction,
    },
    /// 启动一个已配置的命令
    Run {
        name: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
    #[command(hide = true, name = "__exec")]
    Execute {
        program: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CommandAction {
    List,
    Get {
        name: String,
    },
    Add {
        name: String,
        #[arg(long)]
        template: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    Put {
        name: String,
        #[arg(long)]
        file: PathBuf,
    },
    Remove {
        name: String,
    },
    Install {
        name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ResourceAction {
    List,
    Get {
        id: String,
    },
    Put {
        id: String,
        #[arg(long)]
        file: PathBuf,
    },
    Remove {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    Show,
    Check,
    Apply {
        #[arg(long)]
        file: PathBuf,
    },
    /// 导入管理配置，保留现有路由和后端
    Management {
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum CredentialAction {
    Set {
        id: String,
        #[arg(long)]
        file: PathBuf,
    },
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
    pub body_processing: BodyProcessing,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyProcessing {
    pub spool_directory: PathBuf,
    pub io_buffer_bytes: usize,
    pub metadata_limit_bytes: usize,
    pub max_concurrent_requests: usize,
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
        if self.body_processing.io_buffer_bytes == 0
            || self.body_processing.metadata_limit_bytes == 0
            || self.body_processing.max_concurrent_requests == 0
            || self.body_processing.spool_directory.as_os_str().is_empty()
        {
            bail!("body_processing requires a directory and positive resource limits");
        }
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
