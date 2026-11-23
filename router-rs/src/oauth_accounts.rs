use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use oauth2::{TokenResponse, basic::BasicTokenResponse};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    oauth::{self, Settings},
    routing::RouterConfig,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub backend: String,
}

#[derive(Default, Deserialize, Serialize)]
struct Preferences {
    preferred: Option<u32>,
    labels: BTreeMap<u32, String>,
}

pub(crate) struct Pool<'a> {
    pub settings: &'a Settings,
}

impl<'a> Pool<'a> {
    pub fn from_settings(settings: &'a Settings) -> Self {
        Self { settings }
    }

    pub fn new(config: &'a RouterConfig, source: &Source) -> Result<Self> {
        let settings = config
            .runtime
            .backends
            .get(&source.backend)
            .and_then(|backend| backend.auth.as_ref())
            .and_then(|auth| auth.oauth.as_ref())
            .context("账号服务缺少 OAuth 后端")?;
        Ok(Self { settings })
    }

    fn preferences_path(&self) -> PathBuf {
        Path::new(&self.settings.credential_file).with_extension("accounts.json")
    }

    fn preferences(&self) -> Result<Preferences> {
        match fs::read(self.preferences_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("账号设置文件格式有误"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Preferences::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn account_path(&self, id: u32) -> PathBuf {
        let primary = Path::new(&self.settings.credential_file);
        if id == 1 {
            primary.to_owned()
        } else {
            primary.with_extension(format!("account-{id}.json"))
        }
    }

    fn account(&self, id: u32) -> Settings {
        let mut settings = self.settings.clone();
        settings.credential_file = self.account_path(id).to_string_lossy().into_owned();
        settings
    }

    fn ids(&self) -> Result<Vec<u32>> {
        let primary = Path::new(&self.settings.credential_file);
        let stem = primary
            .file_stem()
            .and_then(|s| s.to_str())
            .context("OAuth 凭据文件名无效")?;
        let prefix = format!("{stem}.account-");
        let entries = match fs::read_dir(primary.parent().context("OAuth 凭据目录缺失")?) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            if Some(name.as_os_str()) == primary.file_name() {
                ids.push(1);
            } else if let Some(id) = name.to_str().and_then(|name| {
                name.strip_prefix(&prefix)?
                    .strip_suffix(".json")?
                    .parse::<u32>()
                    .ok()
            }) && id > 1
                && name
                    == self
                        .account_path(id)
                        .file_name()
                        .expect("account file name")
            {
                ids.push(id);
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    pub fn ordered(&self) -> Result<Vec<Settings>> {
        let preferred = self.preferences()?.preferred;
        let mut ids = self.ids()?;
        ids.sort_by_key(|id| (Some(*id) != preferred, *id));
        Ok(ids.into_iter().map(|id| self.account(id)).collect())
    }

    pub async fn available_token(&self) -> Result<String> {
        let mut error = None;
        for account in self.ordered()? {
            match oauth::access_token(&account).await {
                Ok(token) => return Ok(token),
                Err(failure) => error = Some(failure),
            }
        }
        Err(error.unwrap_or_else(|| anyhow::anyhow!("请在账号与额度中登录")))
    }

    pub fn selected(&self, id: &str) -> Result<Settings> {
        let id: u32 = id.parse().context("账号编号无效")?;
        ensure!(self.ids()?.contains(&id), "请刷新后选择已有账号");
        Ok(self.account(id))
    }

    pub fn public(&self) -> Result<Vec<Value>> {
        let preferences = self.preferences()?;
        let ids = self.ids()?;
        let preferred = preferences
            .preferred
            .filter(|id| ids.contains(id))
            .or(ids.first().copied());
        Ok(ids.into_iter().map(|id| {
            let settings = self.account(id);
            let identity = oauth::stored_identity(&settings);
            let status = oauth::credential_status(&settings);
            let connected = status["authenticated"] == true || status["can_refresh"] == true;
            json!({"id":id.to_string(),"label":preferences.labels.get(&id).cloned().unwrap_or_else(|| format!("账号 {id}")),"email":identity.and_then(|identity| identity.email),"preferred":Some(id) == preferred,"unavailable":!connected})
        }).collect())
    }

    pub fn update(&self, id: &str, label: Option<&str>) -> Result<()> {
        let id: u32 = id.parse().context("账号编号无效")?;
        ensure!(self.ids()?.contains(&id), "请刷新后选择已有账号");
        let mut preferences = self.preferences()?;
        if let Some(label) = label {
            preferences.labels.insert(id, label.to_owned());
        } else {
            preferences.preferred = Some(id);
        }
        let path = self.preferences_path();
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().context("账号设置目录缺失")?)?;
        serde_json::to_writer(&mut temporary, &preferences)?;
        temporary.flush()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn session_key(&self) -> String {
        format!("accounts:{}", self.settings.credential_file)
    }

    pub async fn login(&self) -> Result<Value> {
        let settings = self.settings.clone();
        oauth::start_login(self.session_key(), self.settings, move |token| async move {
            Pool {
                settings: &settings,
            }
            .register(token)
            .await
        })
        .await
    }

    async fn register(&self, token: BasicTokenResponse) -> Result<()> {
        let ids = self.ids()?;
        let identity = if let Some(url) = oauth::identity_endpoint(self.settings).await? {
            let identity = oauth::identify(&url, token.access_token().secret()).await?;
            for id in &ids {
                let account = self.account(*id);
                let current = match oauth::stored_identity(&account) {
                    Some(identity) => identity,
                    None => {
                        let current = oauth::access_token(&account)
                            .await
                            .context("请恢复已有账号的授权后再添加账号")?;
                        oauth::identify(&url, &current)
                            .await
                            .context("无法核对已有账号身份")?
                    }
                };
                if current.sub == identity.sub {
                    return oauth::save_login(&account, token, Some(identity)).await;
                }
            }
            Some(identity)
        } else {
            None
        };
        let id = (1..=u32::MAX)
            .find(|id| !ids.contains(id))
            .context("账号编号已用尽")?;
        oauth::save_login(&self.account(id), token, identity).await
    }
}

pub(crate) fn for_backend<'a>(config: &'a RouterConfig, backend: &str) -> Option<Pool<'a>> {
    config.management.as_ref()?;
    let settings = config
        .runtime
        .backends
        .get(backend)?
        .auth
        .as_ref()?
        .oauth
        .as_ref()?;
    Some(Pool { settings })
}
