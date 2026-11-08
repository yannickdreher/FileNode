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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncFolder {
    pub local_folder: PathBuf,
    #[serde(default)]
    pub remote_path: String,
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
    /// Internal engine root; desktop profiles select roots through sync_folders.
    #[serde(skip)]
    pub local_folder: PathBuf,
    /// None is used only by an individual engine; profiles use an explicit list.
    pub sync_folders: Option<Vec<SyncFolder>>,
    /// Resolved only inside a per-folder engine configuration.
    #[serde(skip)]
    pub resolved_remote_path: String,
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
            local_folder: home.join("FileNode"),
            sync_folders: None,
            resolved_remote_path: String::new(),
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
        self.validate_folders()?;
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
        let mut config: Self =
            toml::from_str(&std::fs::read_to_string(path)?).context("Read configuration")?;
        config.resolve_folder_paths()?;
        Ok(config)
    }

    pub fn resolve_folder_paths(&mut self) -> Result<()> {
        let home = directories::UserDirs::new()
            .context("User home directory unavailable")?
            .home_dir()
            .to_path_buf();
        if self.sync_folders.is_none() {
            self.sync_folders = Some(Vec::new());
        }
        if let Some(folders) = &mut self.sync_folders {
            for folder in folders {
                if folder.remote_path.is_empty() {
                    folder.remote_path = Self::derive_remote_path(&folder.local_folder, &home)?;
                }
            }
        }
        Ok(())
    }

    pub fn normalize_folders(&mut self, previous: &Self) -> Result<()> {
        let home = directories::UserDirs::new()
            .context("User home directory unavailable")?
            .home_dir()
            .to_path_buf();
        if let Some(folders) = &mut self.sync_folders {
            for folder in folders.iter_mut() {
                let canonical = folder
                    .local_folder
                    .canonicalize()
                    .unwrap_or_else(|_| folder.local_folder.clone());
                let existing = previous.sync_folders.as_ref().and_then(|items| {
                    items.iter().find(|item| {
                        item.local_folder
                            .canonicalize()
                            .unwrap_or_else(|_| item.local_folder.clone())
                            == canonical
                    })
                });
                folder.remote_path = match existing {
                    Some(existing) => existing.remote_path.clone(),
                    None => {
                        if !folder.local_folder.is_dir() {
                            bail!("Selected folder is unavailable");
                        }
                        Self::derive_remote_path(&canonical, &home)?
                    }
                };
            }
            self.local_folder = folders
                .first()
                .map(|folder| folder.local_folder.clone())
                .unwrap_or_default();
        }
        if self.sync_folders.as_ref().is_some_and(Vec::is_empty) {
            return Ok(());
        }
        self.validate_folders()
    }

    pub fn derive_remote_path(local: &Path, home: &Path) -> Result<String> {
        let local = local.canonicalize().unwrap_or_else(|_| local.to_path_buf());
        let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
        let relative = local
            .strip_prefix(&home)
            .ok()
            .filter(|path| !path.as_os_str().is_empty());
        let path = relative
            .map(Path::to_path_buf)
            .or_else(|| local.file_name().map(PathBuf::from))
            .context("Select a named folder, not a filesystem root")?;
        let components = path
            .components()
            .map(|part| {
                part.as_os_str()
                    .to_str()
                    .context("Folder name must be Unicode")
            })
            .collect::<Result<Vec<_>>>()?;
        let remote = components.join("/");
        Self::validate_remote_path(&remote)?;
        if local == home {
            bail!(
                "Select folders inside your home directory rather than the entire home directory"
            );
        }
        Ok(remote)
    }

    pub fn validate_remote_path(path: &str) -> Result<()> {
        if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
            bail!("Invalid remote folder path");
        }
        for name in path.split('/') {
            crate::filesystem::validate_name(name)?;
            if crate::filesystem::ignored(name) {
                bail!("Folder name is reserved for internal storage");
            }
        }
        Ok(())
    }

    pub fn folder_configs(&self) -> Result<Vec<Self>> {
        let Some(folders) = &self.sync_folders else {
            return Ok(vec![self.clone()]);
        };
        folders
            .iter()
            .map(|folder| {
                let mut config = self.clone();
                config.local_folder = folder.local_folder.clone();
                config.resolved_remote_path = folder.remote_path.clone();
                config.sync_folders = None;
                Ok(config)
            })
            .collect()
    }

    pub fn validate_folders(&self) -> Result<()> {
        let configs = self.folder_configs()?;
        if configs.is_empty() {
            bail!("Select at least one folder to synchronize");
        }
        if configs.len() > 64 {
            bail!("A profile supports at most 64 sync folders");
        }
        for (index, config) in configs.iter().enumerate() {
            if !config.local_folder.is_absolute() {
                bail!("Local folders must use absolute paths");
            }
            if config.local_folder.file_name().is_none() {
                bail!("A filesystem root cannot be synchronized");
            }
            // A disconnected drive is deferred by its engine; it must not stop other roots.
            if self.sync_folders.is_none() && !config.local_folder.is_dir() {
                bail!("Local folder must exist and use an absolute path");
            }
            Self::validate_remote_path(&config.remote_folder_path()?)?;
            let local = config
                .local_folder
                .canonicalize()
                .unwrap_or_else(|_| config.local_folder.clone());
            let remote = crate::filesystem::folded(&config.remote_folder_path()?);
            for other in &configs[..index] {
                let other_local = other
                    .local_folder
                    .canonicalize()
                    .unwrap_or_else(|_| other.local_folder.clone());
                let other_remote = crate::filesystem::folded(&other.remote_folder_path()?);
                if local.starts_with(&other_local) || other_local.starts_with(&local) {
                    bail!("Sync folders must not overlap or select the same local directory");
                }
                if remote == other_remote
                    || remote.starts_with(&(other_remote.clone() + "/"))
                    || other_remote.starts_with(&(remote.clone() + "/"))
                {
                    bail!("Sync folders must not overlap on the server");
                }
            }
        }
        Ok(())
    }

    pub fn remote_folder_path(&self) -> Result<String> {
        if self.resolved_remote_path.is_empty() {
            Ok(self.remote_folder_name()?.to_owned())
        } else {
            Ok(self.resolved_remote_path.clone())
        }
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
        // FileNode uses its own application profile namespace.
        Ok(directories::ProjectDirs::from("", "", "FileNode")
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
