use crate::i18n::{Context, Message, Text};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", content = "name", rename_all = "snake_case")]
pub enum Route {
    #[default]
    Inherit,
    Direct,
    Http(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub user: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub jump: String,
    #[serde(default)]
    pub route: Route,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

impl Profile {
    pub fn quick(host: String) -> Self {
        Self {
            name: host.clone(),
            host,
            user: String::new(),
            port: None,
            identity: String::new(),
            jump: String::new(),
            route: Route::Inherit,
            credential_id: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(id) = &self.credential_id {
            crate::credentials::validate_id(id)?;
        }
        ensure!(
            !self.name.trim().is_empty(),
            Message::plain(Text::ConnectionNameEmpty)
        );
        crate::ssh::validate_target(&self.host)?;
        crate::ssh::validate_user(&self.user)?;
        ensure!(self.port != Some(0), Message::plain(Text::PortRange));
        if !self.jump.is_empty() {
            for hop in self.jump.split(',') {
                crate::ssh::validate_jump(hop)?;
            }
        }
        ensure!(
            !self.identity.contains(['\n', '\r', '\0']),
            Message::plain(Text::IdentityInvalid)
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proxy {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub username: String,
}

impl Proxy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty(),
            Message::plain(Text::ProxyNameEmpty)
        );
        crate::proxy::endpoint(&self.url)?;
        ensure!(
            !self.username.contains([':', '\r', '\n', '\0']),
            Message::plain(Text::ProxyUserInvalid)
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: crate::i18n::Language,
    pub theme: String,
    pub scrollback: usize,
    pub default_proxy: Option<String>,
    pub proxies: Vec<Proxy>,
    pub connections: Vec<Profile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending_credential_cleanup: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: crate::i18n::Language::En,
            theme: "Tokyo Night".into(),
            scrollback: 10_000,
            default_proxy: None,
            proxies: Vec::new(),
            connections: Vec::new(),
            pending_credential_cleanup: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => {
                let cfg: Self = toml::from_str(&s).context(Message::plain(Text::ConfigToml))?;
                cfg.validate()?;
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context(Message::plain(Text::ReadConfig)),
        }
    }
    pub fn validate(&self) -> Result<()> {
        for id in &self.pending_credential_cleanup {
            crate::credentials::validate_id(id)?;
            ensure!(
                !self
                    .connections
                    .iter()
                    .any(|p| p.credential_id.as_ref() == Some(id)),
                Message::plain(Text::CredentialInvalid)
            );
        }
        ensure!(
            self.scrollback <= 100_000,
            Message::plain(Text::ScrollbackLimit)
        );
        let mut names = std::collections::HashSet::new();
        for p in &self.proxies {
            p.validate()?;
            ensure!(
                names.insert(&p.name),
                Message::arg(Text::DuplicateProxy, &p.name)
            );
        }
        if let Some(name) = &self.default_proxy {
            self.proxy(name)?;
        }
        names.clear();
        for p in &self.connections {
            p.validate()?;
            ensure!(
                names.insert(&p.name),
                Message::arg(Text::DuplicateConnection, &p.name)
            );
            if let Route::Http(name) = &p.route {
                self.proxy(name)?;
                ensure!(p.jump.is_empty(), Message::plain(Text::ProxyJumpConflict));
            }
        }
        Ok(())
    }
    pub fn proxy(&self, name: &str) -> Result<&Proxy> {
        self.proxies
            .iter()
            .find(|p| p.name == name)
            .with_context(|| Message::arg(Text::ProxyMissing, name))
    }
    pub fn route(&self, profile: &Profile) -> Result<Option<Proxy>> {
        let name = match &profile.route {
            Route::Inherit => self.default_proxy.as_ref(),
            Route::Direct => None,
            Route::Http(name) => Some(name),
        };
        name.map(|n| self.proxy(n).cloned()).transpose()
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        atomic_write(path, toml::to_string_pretty(self)?.as_bytes())
    }
}

pub fn default_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path).join("ssh-tui/config.toml"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context(Message::plain(Text::HomeMissing))?)
            .join(".config/ssh-tui/config.toml"),
    )
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).context(Message::plain(Text::CreateConfigDir))?;
    let tmp = dir.join(format!(
        ".ssh-tui-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.context(Message::plain(Text::SaveConfig))
}

pub fn route_text(route: &Route) -> String {
    match route {
        Route::Inherit => "inherit".into(),
        Route::Direct => "direct".into(),
        Route::Http(n) => n.clone(),
    }
}
pub fn parse_route(text: &str) -> Result<Route> {
    match text.trim() {
        "" | "inherit" => Ok(Route::Inherit),
        "direct" => Ok(Route::Direct),
        n if n.contains(['\r', '\n']) => bail!(Message::plain(Text::ProxyNameInvalid)),
        n => Ok(Route::Http(n.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_roundtrip_routes_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.proxies.push(Proxy {
            name: "local".into(),
            url: "http://127.0.0.1:7890".into(),
            username: "alice".into(),
        });
        c.default_proxy = Some("local".into());
        let mut p = Profile::quick("example.org".into());
        assert!(c.route(&p).unwrap().is_some());
        p.route = Route::Direct;
        assert!(c.route(&p).unwrap().is_none());
        c.connections.push(p);
        c.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.connections[0].route, Route::Direct);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!fs::read_to_string(path).unwrap().contains("password"));
    }
    #[test]
    fn invalid_config_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "password = 'secret'").unwrap();
        assert!(Config::load(&path).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "password = 'secret'");
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;
    use crate::i18n::Language;
    #[test]
    fn old_configs_default_to_english_and_language_is_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "theme = 'Tokyo Night'").unwrap();
        let mut config = Config::load(&path).unwrap();
        assert_eq!(config.language, Language::En);
        config.language = Language::Zh;
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().language, Language::Zh);
        fs::write(&path, "language = 'fr'").unwrap();
        assert!(Config::load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "language = 'fr'");
    }
}
