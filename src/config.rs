use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    AppPassword,
    Browser,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: crate::i18n::Language,
    pub server: String,
    pub web_url: String,
    pub username: String,
    pub auth_mode: AuthMode,
    /// Empty means the JMAP server is also the authorization server.
    pub oauth_issuer: String,
    /// Empty enables public dynamic client registration when the provider supports it.
    pub oauth_client_id: String,
    pub oauth_scopes: String,
    /// Zero selects an ephemeral loopback port. Registered clients may need a fixed port.
    pub oauth_redirect_port: u16,
    pub account_id: Option<String>,
    pub local_folder: PathBuf,
    pub interval_seconds: u64,
    pub full_scan_seconds: u64,
    pub upload_limit_kib: u64,
    pub download_limit_kib: u64,
    pub file_settle_seconds: u64,
    pub pause_on_power_saver: bool,
    pub autostart: bool,
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
            auth_mode: AuthMode::AppPassword,
            oauth_issuer: String::new(),
            oauth_client_id: String::new(),
            oauth_scopes: "urn:ietf:params:oauth:scope:files offline_access".into(),
            oauth_redirect_port: 0,
            account_id: None,
            local_folder: home.join("NoriDrive"),
            interval_seconds: 30,
            full_scan_seconds: 900,
            upload_limit_kib: 0,
            download_limit_kib: 0,
            file_settle_seconds: 2,
            pause_on_power_saver: true,
            autostart: false,
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
        if self.auth_mode == AuthMode::Browser {
            crate::auth::validate_issuer(self.oauth_issuer_url())?;
            if self.oauth_scopes.trim().is_empty() {
                bail!("Explicit OAuth scopes are required; unscoped full-account access is not a fallback");
            }
            if self.oauth_client_id.len() > 4096 || self.oauth_scopes.len() > 4096 {
                bail!("OAuth configuration exceeds length limits");
            }
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
        let remote_folder = self.remote_folder_name()?;
        crate::filesystem::validate_name(remote_folder)?;
        if crate::filesystem::ignored(remote_folder) {
            bail!("The local folder name is reserved for internal storage");
        }
        if self.interval_seconds < 5 || self.interval_seconds > 86400 {
            bail!("Check interval must be between 5 and 86400 seconds");
        }
        if self.max_delete_percent > 100 || self.max_file_bytes == 0 {
            bail!("Invalid protection limits");
        }
        if self.full_scan_seconds < 60 || self.full_scan_seconds > 86400 {
            bail!("Full verification interval must be between 60 and 86400 seconds");
        }
        if self.upload_limit_kib > 1_000_000
            || self.download_limit_kib > 1_000_000
            || self.file_settle_seconds > 60
        {
            bail!("Invalid transfer limit or file settling interval");
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        toml::from_str(&std::fs::read_to_string(path)?).context("Read configuration")
    }

    pub fn remote_folder_name(&self) -> Result<&str> {
        self.local_folder
            .file_name()
            .and_then(|name| name.to_str())
            .context("Select a named local folder; a filesystem root cannot be synchronized")
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
        // Preserve the existing profile location across the NoriDrive rename.
        Ok(directories::ProjectDirs::from("de", "Noritec", "Drive")
            .context("User configuration directory is unavailable")?
            .config_dir()
            .join("config.toml"))
    }

    pub fn credential_key(&self) -> String {
        format!("{}|{}", self.server.trim_end_matches('/'), self.username)
    }

    pub fn oauth_issuer_url(&self) -> &str {
        if self.oauth_issuer.is_empty() {
            &self.server
        } else {
            &self.oauth_issuer
        }
    }

    pub fn oauth_credential_key(&self) -> String {
        use sha2::{Digest, Sha256};
        let identity = format!(
            "oauth|{}|{}|{}|{}|{}",
            self.server.trim_end_matches('/'),
            self.username,
            self.oauth_issuer_url(),
            self.oauth_client_id,
            self.oauth_scopes
        );
        format!("oauth|{}", hex::encode(Sha256::digest(identity.as_bytes())))
    }
}
