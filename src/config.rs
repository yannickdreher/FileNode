use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: crate::i18n::Language,
    pub server: String,
    pub web_url: String,
    pub username: String,
    pub account_id: Option<String>,
    pub local_folder: PathBuf,
    pub remote_folder: String,
    pub interval_seconds: u64,
    pub full_scan_seconds: u64,
    pub autostart: bool,
    pub auto_connect: bool,
    pub propagate_deletes: bool,
    pub max_delete_count: usize,
    pub max_delete_percent: usize,
    pub max_file_bytes: u64,
    pub device_name: String,
}

impl Default for Config {
    fn default() -> Self {
        let home = directories::UserDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            language: crate::i18n::Language::Auto,
            server: "https://jmap.example.com".into(),
            web_url: "https://mail.example.com".into(),
            username: String::new(),
            account_id: None,
            local_folder: home.join("Noritec Drive"),
            remote_folder: "Noritec Drive".into(),
            interval_seconds: 30,
            full_scan_seconds: 900,
            autostart: false,
            auto_connect: true,
            propagate_deletes: false,
            max_delete_count: 20,
            max_delete_percent: 25,
            max_file_bytes: 1024 * 1024 * 1024,
            device_name: std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "Desktop".into()),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        let url = Url::parse(&self.server).context("Invalid server URL")?;
        if url.scheme() != "https" || url.host_str().is_none() {
            bail!("The server must use HTTPS");
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Server URL must not contain credentials, query parameters or a fragment");
        }
        if self.username.trim().is_empty() {
            bail!("Username is required");
        }
        let web = Url::parse(&self.web_url).context("Invalid web client URL")?;
        if web.scheme() != "https"
            || web.host_str().is_none()
            || !web.username().is_empty()
            || web.password().is_some()
        {
            bail!("Web client URL must use HTTPS without embedded credentials");
        }
        if !self.local_folder.is_absolute() || !self.local_folder.is_dir() {
            bail!("Local folder must exist and use an absolute path");
        }
        crate::filesystem::validate_name(&self.remote_folder)?;
        if self.interval_seconds < 5 || self.interval_seconds > 86400 {
            bail!("Check interval must be between 5 and 86400 seconds");
        }
        if self.max_delete_percent > 100 || self.max_file_bytes == 0 {
            bail!("Invalid protection limits");
        }
        if self.full_scan_seconds < 60 || self.full_scan_seconds > 86400 {
            bail!("Full verification interval must be between 60 and 86400 seconds");
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        toml::from_str(&std::fs::read_to_string(path)?).context("Read configuration")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("Configuration path has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    pub fn default_path() -> Result<PathBuf> {
        Ok(directories::ProjectDirs::from("de", "Noritec", "Drive")
            .context("User configuration directory is unavailable")?
            .config_dir()
            .join("config.toml"))
    }

    pub fn credential_key(&self) -> String {
        format!("{}|{}", self.server.trim_end_matches('/'), self.username)
    }
}
