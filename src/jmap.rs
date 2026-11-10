//! Wire adapter. FileNode is still a draft: Stalwart 0.16 uses nodeType/type,
//! Stalwart 1.0 uses @type/mediaType. Never emulate files through WebDAV.
use crate::auth::Authentication;
use crate::{config::Config, model::*};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{
    blocking::{Client, Response},
    header, StatusCode,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use url::Url;

pub trait Remote {
    fn downloader(&self) -> Option<Arc<dyn Downloader>> {
        None
    }
    fn uploader(&self) -> Option<Arc<dyn Uploader>> {
        None
    }
    fn upload_limits(&self) -> UploadLimits {
        UploadLimits::default()
    }
    fn create_trash(&mut self, parent: Option<&str>, state: &mut String) -> Result<Node>;
    fn account_id(&self) -> &str;
    fn current_state(&self) -> Result<String>;
    fn snapshot(&mut self, previous: Option<&Snapshot>) -> Result<Snapshot>;
    fn upload(&mut self, path: &Path, size: u64) -> Result<String>;
    fn download(&mut self, node: &Node, path: &Path) -> Result<()>;
    fn create(
        &mut self,
        name: &str,
        parent: Option<&str>,
        kind: Kind,
        blob: Option<&str>,
        size: u64,
        state: &mut String,
    ) -> Result<Node>;
    fn update(
        &mut self,
        node: &Node,
        name: &str,
        parent: Option<&str>,
        blob: Option<(&str, u64)>,
        state: &mut String,
    ) -> Result<Node>;
}
pub trait Uploader: Send + Sync {
    fn send_blob(&self, path: &Path, size: u64) -> Result<String>;
    fn policy(&self) -> Arc<crate::runtime::Policy>;
}

/// Immutable blob range transport used by the Windows Cloud Files callbacks.
/// The consumer may fail (including cancellation); callers must stop immediately.
pub trait Downloader: Send + Sync {
    fn read_range(
        &self,
        node: &Node,
        offset: u64,
        length: u64,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<()>;
}

#[derive(Clone, Copy, Default, serde::Serialize)]
pub struct UploadLimits {
    pub max_size_upload: Option<u64>,
    pub max_concurrent_upload: Option<usize>,
}
impl UploadLimits {
    pub fn effective_size(self, config: &Config) -> u64 {
        self.max_size_upload
            .unwrap_or(if config.max_file_bytes == 0 {
                1024 * 1024 * 1024
            } else {
                config.max_file_bytes
            })
            .min(config.file_limit())
    }
    pub fn effective_parallelism(self, config: &Config) -> usize {
        config
            .parallel_uploads
            .min(self.max_concurrent_upload.unwrap_or(1))
            .clamp(1, 16)
    }
}

#[derive(Clone)]
pub struct Jmap {
    push_url: Option<Url>,
    push: Option<Arc<crate::push::Push>>,
    limits: UploadLimits,
    parallel_uploads: usize,
    web_trash_url: Option<String>,
    client: Client,
    origin: Url,
    api: Url,
    upload_url: String,
    download_url: String,
    authentication: Authentication,
    pub account_id: String,
    modern: bool,
    cancel: Arc<AtomicBool>,
    max_bytes: u64,
    policy: Arc<crate::runtime::Policy>,
}

impl Jmap {
    pub fn connect(config: &Config, password: String, cancel: Arc<AtomicBool>) -> Result<Self> {
        Self::connect_inner(config, password, cancel, false)
    }
    // HTTP is admitted only for in-process protocol tests; never exposed in config or GUI.
    fn connect_inner(
        config: &Config,
        password: String,
        cancel: Arc<AtomicBool>,
        test_http: bool,
    ) -> Result<Self> {
        Self::connect_authorized_inner(
            config,
            Authentication::basic(&config.username, password),
            cancel,
            test_http,
        )
    }

    pub fn connect_authenticated(
        config: &Config,
        authentication: Authentication,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::connect_authorized_inner(config, authentication, cancel, false)
    }

    fn connect_authorized_inner(
        config: &Config,
        authentication: Authentication,
        cancel: Arc<AtomicBool>,
        test_http: bool,
    ) -> Result<Self> {
        let policy = crate::runtime::Policy::new(
            config,
            cancel.clone(),
            Arc::new(crate::runtime::PowerState::default()),
        );
        let origin = Url::parse(&config.server)?;
        if !test_http && origin.scheme() != "https" {
            bail!("HTTPS is required");
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("FileNode/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let mut discovery = origin.join("/.well-known/jmap")?;
        let mut session = None;
        for _ in 0..6 {
            same_origin(&origin, &discovery)?;
            let response = authentication
                .apply(client.get(discovery.clone()))?
                .send()?;
            authentication.observe(response.status())?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .context("Redirect has no location")?
                    .to_str()?;
                discovery = discovery.join(location)?;
            } else {
                session = Some(read_json(response)?);
                break;
            }
        }
        let session = session.context("Too many JMAP redirects")?;
        if session["capabilities"].get(FILE_CAPABILITY).is_none() {
            bail!("Server does not support JMAP FileNodes");
        }
        let accounts = session["accounts"]
            .as_object()
            .context("JMAP accounts are missing")?;
        let account_id = if let Some(id) = &config.account_id {
            id.clone()
        } else if let Some(id) = session["primaryAccounts"][FILE_CAPABILITY].as_str() {
            id.to_owned()
        } else {
            let candidates: Vec<_> = accounts
                .iter()
                .filter(|(_, a)| {
                    a["accountCapabilities"].get(FILE_CAPABILITY).is_some()
                        && a["isReadOnly"] != true
                })
                .collect();
            if candidates.len() != 1 {
                bail!("Multiple file accounts or none available: configure account_id explicitly");
            }
            candidates[0].0.clone()
        };
        let account = accounts
            .get(&account_id)
            .context("Selected JMAP account is missing")?;
        if account["accountCapabilities"]
            .get(FILE_CAPABILITY)
            .is_none()
            || account["isReadOnly"] == true
        {
            bail!("File account is not writable");
        }
        let modern = account["accountCapabilities"][FILE_CAPABILITY]["supportedNodeTypes"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "Directory"));
        let api = origin.join(session["apiUrl"].as_str().context("apiUrl is missing")?)?;
        same_origin(&origin, &api)?;
        let upload_url = session["uploadUrl"]
            .as_str()
            .context("uploadUrl is missing")?
            .to_owned();
        let download_url = session["downloadUrl"]
            .as_str()
            .context("downloadUrl is missing")?
            .to_owned();
        let this = Self {
            push_url: crate::push::endpoint(
                &origin,
                &session["capabilities"]["urn:ietf:params:jmap:websocket"],
            ),
            push: None,
            parallel_uploads: config.parallel_uploads,
            limits: UploadLimits {
                max_size_upload: session["capabilities"]["urn:ietf:params:jmap:core"]
                    ["maxSizeUpload"]
                    .as_u64(),
                max_concurrent_upload: session["capabilities"]["urn:ietf:params:jmap:core"]
                    ["maxConcurrentUpload"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n > 0),
            },
            web_trash_url: account["accountCapabilities"][FILE_CAPABILITY]["webTrashUrl"]
                .as_str()
                .and_then(|value| {
                    let url = origin.join(value).ok()?;
                    if url.scheme() != "https"
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return None;
                    }
                    Some(url.to_string())
                }),
            client,
            origin,
            api,
            upload_url,
            download_url,
            authentication,
            account_id,
            modern,
            cancel,
            max_bytes: config.file_limit(),
            policy,
        };
        this.expand(&this.upload_url, None)?;
        this.expand(&this.download_url, Some(("probe", "probe")))?;
        Ok(this)
    }

    pub fn set_policy(&mut self, policy: Arc<crate::runtime::Policy>) {
        self.policy = policy;
        self.start_push();
    }
    pub fn start_push(&mut self) {
        if self.push.is_none() {
            if let Some(url) = &self.push_url {
                self.push = Some(crate::push::Push::start(
                    url.clone(),
                    self.authentication.clone(),
                    self.account_id.clone(),
                    self.policy.clone(),
                ));
            }
        }
    }
    pub fn take_remote_change(&self) -> bool {
        self.push.as_ref().is_some_and(|push| push.take_change())
    }

    fn transfer_transport(&self) -> Self {
        let mut transport = self.clone();
        // Share the HTTP pool and token session, but never keep an obsolete
        // account push worker alive through a file-transfer callback.
        transport.push = None;
        transport.push_url = None;
        transport
    }

    fn transfer_timeout(&self, size: u64, upload: bool) -> Duration {
        let rate = if upload {
            &self.policy.upload
        } else {
            &self.policy.download
        }
        .load(Ordering::Relaxed);
        Duration::from_secs(
            size.checked_div(rate)
                .map(|seconds| {
                    seconds
                        .saturating_mul(if upload {
                            self.limits
                                .max_concurrent_upload
                                .unwrap_or(1)
                                .min(self.parallel_uploads)
                                .clamp(1, 16) as u64
                        } else {
                            1
                        })
                        .saturating_add(180)
                })
                .unwrap_or(120),
        )
    }

    fn expand(&self, template: &str, blob: Option<(&str, &str)>) -> Result<Url> {
        fn encoded(s: &str) -> String {
            url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
        }
        let (blob, name) = blob.unwrap_or(("", ""));
        let url = template
            .replace("{accountId}", &encoded(&self.account_id))
            .replace("{blobId}", &encoded(blob))
            .replace("{name}", &encoded(name))
            .replace("{type}", "application%2Foctet-stream");
        if url.contains(['{', '}']) {
            bail!("Unsupported JMAP URL template");
        }
        let url = self.origin.join(&url)?;
        same_origin(&self.origin, &url)?;
        Ok(url)
    }

    fn call(&self, method: &str, args: Value) -> Result<Value> {
        self.policy.check()?;
        if self.cancel.load(Ordering::Relaxed) {
            bail!("Synchronization paused");
        }
        let body = json!({"using":[CORE_CAPABILITY, FILE_CAPABILITY], "methodCalls":[[method,args,"drive"]]});
        let attempts = if method.ends_with("/set") { 1 } else { 3 };
        let mut last_error = None;
        for attempt in 0..attempts {
            let result = self
                .authentication
                .apply(self.client.post(self.api.clone()).json(&body))?
                .send();
            if let Ok(response) = &result {
                self.authentication.observe(response.status())?;
            }
            match result {
                Ok(response)
                    if response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error() =>
                {
                    last_error = Some(anyhow!(
                        "JMAP temporarily unavailable: {}",
                        response.status()
                    ));
                }
                Ok(response) => {
                    let result = read_json(response)?;
                    let call = result["methodResponses"]
                        .as_array()
                        .and_then(|a| a.first())
                        .context("JMAP methodResponses is missing")?;
                    if call[2] != "drive" {
                        bail!("Unexpected JMAP response ID");
                    }
                    if call[0] == "error" {
                        bail!(
                            "JMAP {method}: {}",
                            call[1]["type"].as_str().unwrap_or("unknown")
                        );
                    }
                    if call[0] != method {
                        bail!("Unexpected JMAP method");
                    }
                    return Ok(call[1].clone());
                }
                Err(error) => {
                    last_error = Some(error.into());
                }
            }
            if attempt + 1 < attempts {
                std::thread::sleep(Duration::from_secs(1 << attempt));
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("JMAP connection failed")))
    }

    fn get_nodes(&self, ids: &[String]) -> Result<Snapshot> {
        let result = self.call(
            "FileNode/get",
            json!({"accountId":self.account_id,"ids":ids}),
        )?;
        let list = result["list"]
            .as_array()
            .context("FileNode list is missing")?;
        if result["notFound"].as_array().is_some_and(|a| !a.is_empty()) {
            bail!("Remote files disappeared during snapshot; retry required");
        }
        let mut nodes = Vec::new();
        for raw in list {
            nodes.push(parse_node(raw)?);
        }
        Ok(Snapshot {
            state: result["state"]
                .as_str()
                .context("FileNode state is missing")?
                .into(),
            nodes,
        })
    }

    fn full_snapshot(&self) -> Result<Snapshot> {
        let mut snapshot = self.get_nodes(&[])?;
        let mut position = 0;
        let mut query_state = None;
        loop {
            let page = self.call("FileNode/query", json!({"accountId":self.account_id,"position":position,"limit":256,"calculateTotal":true}))?;
            let state = page["queryState"]
                .as_str()
                .context("queryState is missing")?;
            if query_state.as_deref().is_some_and(|old| old != state) {
                bail!("Remote listing changed during pagination");
            }
            query_state = Some(state.to_owned());
            let ids: Vec<String> = serde_json::from_value(page["ids"].clone())?;
            if ids.is_empty() {
                break;
            }
            let part = self.get_nodes(&ids)?;
            if part.state != snapshot.state {
                bail!("Inconsistent remote snapshot");
            }
            position += ids.len();
            snapshot.nodes.extend(part.nodes);
            if position > 100_000 {
                bail!("More than 100000 remote files; refusing partial synchronization");
            }
            if ids.len() < 256 {
                break;
            }
        }
        if self.get_nodes(&[])?.state != snapshot.state {
            bail!("Remote snapshot changed");
        }
        Ok(snapshot)
    }

    fn incremental(&self, previous: &Snapshot) -> Result<Snapshot> {
        let mut nodes: BTreeMap<_, _> = previous
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
        let mut state = previous.state.clone();
        for _ in 0..1000 {
            let change = self.call(
                "FileNode/changes",
                json!({"accountId":self.account_id,"sinceState":state,"maxChanges":256}),
            )?;
            let new_state = change["newState"]
                .as_str()
                .context("newState is missing")?
                .to_owned();
            for id in change["destroyed"]
                .as_array()
                .context("destroyed is missing")?
            {
                nodes.remove(id.as_str().context("Invalid file ID")?);
            }
            let mut ids = Vec::new();
            for field in ["created", "updated"] {
                for id in change[field].as_array().context("Change list is missing")? {
                    ids.push(id.as_str().context("Invalid file ID")?.to_owned());
                }
            }
            for chunk in ids.chunks(256) {
                for node in self.get_nodes(chunk)?.nodes {
                    nodes.insert(node.id.clone(), node);
                }
            }
            state = new_state;
            if nodes.len() > 100_000 {
                bail!("Too many remote entries");
            }
            if change["hasMoreChanges"] == false {
                if self.get_nodes(&[])?.state != state {
                    bail!("Changes occurred during incremental snapshot");
                }
                return Ok(Snapshot {
                    state,
                    nodes: nodes.into_values().collect(),
                });
            }
        }
        bail!("Change sequence is too long")
    }

    fn mutate(&self, body: Value, state: &mut String) -> Result<Value> {
        let mut body = body;
        body["accountId"] = json!(self.account_id);
        body["ifInState"] = json!(state);
        let result = self.call("FileNode/set", body)?;
        for field in ["notCreated", "notUpdated", "notDestroyed"] {
            if result[field].as_object().is_some_and(|m| !m.is_empty()) {
                let kinds: Vec<_> = result[field]
                    .as_object()
                    .unwrap()
                    .values()
                    .map(|e| e["type"].as_str().unwrap_or("unknown"))
                    .collect();
                bail!("JMAP {field}: {}", kinds.join(", "));
            }
        }
        *state = result["newState"]
            .as_str()
            .context("set newState is missing")?
            .into();
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_with_role(
        &mut self,
        name: &str,
        parent: Option<&str>,
        kind: Kind,
        blob: Option<&str>,
        size: u64,
        state: &mut String,
        role: Option<&str>,
    ) -> Result<Node> {
        let mut node = json!({"name":name,"parentId":parent});
        if let Some(role) = role {
            node["role"] = json!(role);
        }
        if self.modern {
            node["@type"] = json!(if kind == Kind::Directory {
                "Directory"
            } else {
                "File"
            });
        } else {
            node["nodeType"] = json!(if kind == Kind::Directory {
                "directory"
            } else {
                "file"
            });
        }
        if let Some(blob) = blob {
            node["blobId"] = json!(blob);
            node[if self.modern { "mediaType" } else { "type" }] =
                json!("application/octet-stream");
        }
        let result = self.mutate(json!({"create":{"drive":node}}), state)?;
        let id = result["created"]["drive"]["id"]
            .as_str()
            .context("Create response has no file ID")?;
        self.read_written_node(
            Node {
                id: id.into(),
                name: name.into(),
                parent_id: parent.map(str::to_owned),
                blob_id: blob.map(str::to_owned),
                kind,
                size,
                role: role.map(str::to_owned),
            },
            state,
        )
    }

    fn read_written_node(&self, expected: Node, state: &str) -> Result<Node> {
        // Upload IDs can be temporary. Stalwart returns a different linked blob ID
        // once content belongs to a FileNode. Persist the server's actual identity.
        let snapshot = self.get_nodes(std::slice::from_ref(&expected.id))?;
        if snapshot.state != state {
            bail!("Server changed while confirming the write; reconcile before retrying");
        }
        let [node] = snapshot.nodes.as_slice() else {
            bail!("Written FileNode was not returned exactly once");
        };
        if node.id != expected.id
            || node.name != expected.name
            || node.parent_id != expected.parent_id
            || node.kind != expected.kind
            || node.role != expected.role
            || (node.kind == Kind::File && node.size != expected.size)
        {
            bail!("Written FileNode does not match the requested operation");
        }
        Ok(node.clone())
    }
    pub fn web_trash_url(&self) -> Option<&str> {
        self.web_trash_url.as_deref()
    }
}

impl Uploader for Jmap {
    fn send_blob(&self, path: &Path, size: u64) -> Result<String> {
        self.upload_blob(path, size)
    }
    fn policy(&self) -> Arc<crate::runtime::Policy> {
        self.policy.clone()
    }
}
impl Jmap {
    fn upload_blob(&self, path: &Path, size: u64) -> Result<String> {
        if size
            > self
                .max_bytes
                .min(self.limits.max_size_upload.unwrap_or(self.max_bytes))
        {
            bail!("Upload exceeds the file size limit");
        }
        let file = File::open(path)?;
        let response = self
            .authentication
            .apply(
                self.client
                    .post(self.expand(&self.upload_url, None)?)
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .header(header::CONTENT_LENGTH, size)
                    .timeout(self.transfer_timeout(size, true))
                    .body(reqwest::blocking::Body::sized(
                        crate::runtime::LimitedReader::new(file, self.policy.clone()),
                        size,
                    )),
            )?
            .send()?;
        self.authentication.observe(response.status())?;
        let value = read_json(response)?;
        if value["accountId"] != self.account_id || value["size"].as_u64() != Some(size) {
            bail!("Upload acknowledgement does not match the file");
        }
        Ok(value["blobId"]
            .as_str()
            .context("Upload response has no blobId")?
            .into())
    }
}

impl Downloader for Jmap {
    fn read_range(
        &self,
        node: &Node,
        offset: u64,
        length: u64,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        let end = offset
            .checked_add(length)
            .context("Invalid cloud file range")?;
        if end > node.size || node.size > self.max_bytes {
            bail!("Cloud file range exceeds the file limit");
        }
        self.policy.check()?;
        if cancel.load(Ordering::Relaxed) {
            bail!("Cloud file request cancelled");
        }
        if length == 0 {
            return Ok(());
        }
        let blob = node.blob_id.as_deref().context("File has no blob")?;
        let request = self.authentication.apply(
            self.client
                .get(self.expand(&self.download_url, Some((blob, &node.name)))?)
                .header(header::ACCEPT_ENCODING, "identity")
                .header(header::RANGE, format!("bytes={offset}-{}", end - 1)),
        )?;
        let mut response = request
            .timeout(self.transfer_timeout(node.size, false))
            .send()?;
        self.authentication.observe(response.status())?;
        if response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|encoding| encoding != "identity")
        {
            bail!("Encoded cloud file responses cannot be mapped to byte ranges");
        }
        let mut discard = 0;
        if response.status() == StatusCode::PARTIAL_CONTENT {
            let expected = format!("bytes {offset}-{}/{}", end - 1, node.size);
            if response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                != Some(expected.as_str())
            {
                bail!("Server returned a different cloud file range");
            }
            if response.content_length().is_some_and(|size| size != length) {
                bail!("Invalid range response size");
            }
        } else if response.status() == StatusCode::OK {
            // A compliant JMAP blob server need not implement byte ranges. Keep
            // memory bounded, account for the skipped bytes, and never expose
            // a prefix at the wrong offset.
            if response
                .content_length()
                .is_some_and(|size| size != node.size)
            {
                bail!("Blob size changed");
            }
            discard = offset;
        } else {
            bail!("Cloud file download HTTP {}", response.status());
        }
        let mut budget = crate::runtime::Budget::new(self.policy.clone(), false);
        let mut buffer = [0u8; 65536];
        let mut remaining = length;
        while discard > 0 || remaining > 0 {
            self.policy.check()?;
            if cancel.load(Ordering::Relaxed) {
                bail!("Cloud file request cancelled");
            }
            let wanted =
                if discard > 0 { discard } else { remaining }.min(buffer.len() as u64) as usize;
            let count = response.read(&mut buffer[..wanted])?;
            if count == 0 {
                bail!("Incomplete cloud file download; retry when connected");
            }
            budget.reserve(count)?;
            if discard > 0 {
                discard -= count as u64;
            } else {
                consume(&buffer[..count])?;
                remaining -= count as u64;
            }
        }
        Ok(())
    }
}

impl Remote for Jmap {
    fn downloader(&self) -> Option<Arc<dyn Downloader>> {
        Some(Arc::new(self.transfer_transport()))
    }
    fn uploader(&self) -> Option<Arc<dyn Uploader>> {
        Some(Arc::new(self.transfer_transport()))
    }
    fn upload_limits(&self) -> UploadLimits {
        self.limits
    }
    fn account_id(&self) -> &str {
        &self.account_id
    }
    fn current_state(&self) -> Result<String> {
        Ok(self.get_nodes(&[])?.state)
    }
    fn snapshot(&mut self, previous: Option<&Snapshot>) -> Result<Snapshot> {
        if let Some(previous) = previous {
            match self.incremental(previous) {
                Ok(snapshot) => return Ok(snapshot),
                Err(e)
                    if e.to_string().contains("cannotCalculateChanges")
                        || e.to_string().contains("unknownMethod") => {}
                Err(e) => return Err(e),
            }
        }
        self.full_snapshot()
    }

    fn upload(&mut self, path: &Path, size: u64) -> Result<String> {
        self.upload_blob(path, size)
    }

    fn download(&mut self, node: &Node, path: &Path) -> Result<()> {
        if node.size > self.max_bytes {
            bail!("Download exceeds size limit: {}", node.name);
        }
        if let Ok(m) = std::fs::symlink_metadata(path) {
            if !m.is_file() || m.file_type().is_symlink() {
                bail!("Unsafe transfer file");
            }
        }
        let existing = path.metadata().map(|m| m.len()).unwrap_or(0);
        if existing > node.size {
            std::fs::remove_file(path)?;
        }
        let offset = if existing <= node.size { existing } else { 0 };
        // An empty file is still fetched once to validate access and create the staging file.
        if offset == node.size && offset > 0 {
            return Ok(());
        }
        let blob = node.blob_id.as_deref().context("File has no blob")?;
        let mut request = self.authentication.apply(
            self.client
                .get(self.expand(&self.download_url, Some((blob, &node.name)))?)
                .header(header::ACCEPT_ENCODING, "identity"),
        )?;
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
        }
        let mut response = request
            .timeout(self.transfer_timeout(node.size, false))
            .send()?;
        self.authentication.observe(response.status())?;
        let append = response.status() == StatusCode::PARTIAL_CONTENT && offset > 0;
        if append {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .context("Range response has no Content-Range")?
                .to_str()?;
            if !range.starts_with(&format!("bytes {offset}-"))
                || !range.ends_with(&format!("/{}", node.size))
            {
                bail!("Invalid download resume response");
            }
        } else if response.status() != StatusCode::OK {
            bail!("Download HTTP {}", response.status());
        }
        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(path)?;
        let mut total = if append { offset } else { 0 };
        let progress = crate::activity::Progress::current();
        progress.set(total);
        let mut buffer = [0u8; 16384];
        let mut budget = crate::runtime::Budget::new(self.policy.clone(), false);
        loop {
            self.policy.check()?;
            if self.cancel.load(Ordering::Relaxed) {
                bail!("Transfer paused");
            }
            let n = response.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > node.size || total > self.max_bytes {
                bail!("Download exceeds advertised size");
            }
            budget.reserve(n)?;
            output.write_all(&buffer[..n])?;
            progress.set(total);
        }
        output.sync_all()?;
        if total != node.size {
            bail!("Incomplete download; resume on the next run");
        }
        Ok(())
    }

    fn create_trash(&mut self, parent: Option<&str>, state: &mut String) -> Result<Node> {
        self.create_with_role(
            ".Trash",
            parent,
            Kind::Directory,
            None,
            0,
            state,
            if self.modern { Some("trash") } else { None },
        )
    }

    fn create(
        &mut self,
        name: &str,
        parent: Option<&str>,
        kind: Kind,
        blob: Option<&str>,
        size: u64,
        state: &mut String,
    ) -> Result<Node> {
        self.create_with_role(name, parent, kind, blob, size, state, None)
    }

    fn update(
        &mut self,
        node: &Node,
        name: &str,
        parent: Option<&str>,
        blob: Option<(&str, u64)>,
        state: &mut String,
    ) -> Result<Node> {
        let mut patch = json!({"name":name,"parentId":parent});
        if let Some((blob, _)) = blob {
            patch["blobId"] = json!(blob);
            patch["modified"] = Value::Null;
        }
        let result = self.mutate(json!({"update":{node.id.clone():patch}}), state)?;
        if result["updated"].get(&node.id).is_none() {
            bail!("Update was not acknowledged");
        }
        let mut updated = node.clone();
        updated.name = name.into();
        updated.parent_id = parent.map(str::to_owned);
        if let Some((blob, size)) = blob {
            updated.blob_id = Some(blob.into());
            updated.size = size;
        }
        self.read_written_node(updated, state)
    }
}

fn read_json(response: Response) -> Result<Value> {
    let status = response.status();
    if !status.is_success() {
        bail!("HTTP {status}; check credentials and server");
    }
    let mut body = Vec::new();
    response.take(16 * 1024 * 1024 + 1).read_to_end(&mut body)?;
    if body.len() > 16 * 1024 * 1024 {
        bail!("JMAP response exceeds size limit");
    }
    serde_json::from_slice(&body).context("Invalid JMAP JSON")
}

fn same_origin(base: &Url, target: &Url) -> Result<()> {
    if base.origin() != target.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        bail!("JMAP refers to a different origin; credentials will not be forwarded");
    }
    Ok(())
}

fn parse_node(raw: &Value) -> Result<Node> {
    let kind = match raw["@type"].as_str().or_else(|| raw["nodeType"].as_str()) {
        Some("File" | "file") => Kind::File,
        Some("Directory" | "directory") => Kind::Directory,
        _ => bail!("Unsupported FileNode type (links are not followed)"),
    };
    let node = Node {
        id: raw["id"].as_str().context("File ID is missing")?.into(),
        name: raw["name"].as_str().context("File name is missing")?.into(),
        parent_id: raw["parentId"].as_str().map(str::to_owned),
        blob_id: raw["blobId"].as_str().map(str::to_owned),
        role: raw["role"].as_str().map(str::to_owned),
        kind,
        size: raw["size"].as_u64().unwrap_or(0),
    };
    if kind == Kind::File && node.blob_id.is_none() {
        bail!("Remote file has no blobId");
    }
    Ok(node)
}

/// None selects the account root: nodes with a null parentId are top-level entries.
pub fn scoped_tree(snapshot: &Snapshot, root_id: Option<&str>) -> Result<RemoteTree> {
    let all: HashMap<_, _> = snapshot.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    if all.len() != snapshot.nodes.len() {
        bail!("Duplicate file IDs in snapshot");
    }
    if root_id.is_some_and(|id| all.get(id).is_none_or(|n| n.kind != Kind::Directory)) {
        bail!("Remote sync folder disappeared; synchronization blocked");
    }
    if let Some(root) = root_id {
        let mut current = all.get(root).copied();
        let mut seen = HashSet::new();
        while let Some(node) = current {
            if !seen.insert(&node.id) {
                bail!("Remote root ancestry is cyclic");
            }
            if node.role.as_deref() == Some("trash")
                || (node.parent_id.is_none() && node.name == ".Trash")
            {
                bail!("Selected remote folder is in the trash; synchronization blocked");
            }
            current = node
                .parent_id
                .as_deref()
                .and_then(|parent| all.get(parent).copied());
        }
    }
    let mut result = RemoteTree::new();
    let mut names = HashSet::new();
    for node in &snapshot.nodes {
        if Some(node.id.as_str()) == root_id {
            continue;
        }
        let mut pieces = vec![node.name.as_str()];
        let mut current = node;
        let mut seen = HashSet::new();
        let in_scope = loop {
            if !seen.insert(&current.id) || seen.len() > 128 {
                bail!("Remote directory tree is cyclic or too deep");
            }
            if current.role.as_deref() == Some("trash")
                || (current.parent_id.is_none() && current.name == ".Trash")
            {
                break false;
            }
            match current.parent_id.as_deref() {
                Some(parent) if Some(parent) == root_id => break true,
                Some(parent) => {
                    current = all
                        .get(parent)
                        .filter(|n| n.kind == Kind::Directory)
                        .context("Remote parent directory is missing or is not a directory")?;
                    pieces.push(&current.name);
                }
                None => break root_id.is_none(),
            }
        };
        if !in_scope {
            continue;
        }
        pieces.reverse();
        let path = pieces.join("/");
        if crate::filesystem::ignored(&path) {
            continue;
        }
        for name in pieces {
            crate::filesystem::validate_name(name)?;
        }
        if path.len() > 240 {
            bail!("Remote path exceeds length limit");
        }
        if !names.insert(crate::filesystem::folded(&path)) {
            bail!("Remote name collision: {path}");
        }
        result.insert(path, node.clone());
    }
    Ok(result)
}

pub fn transfer_path(private: &Path, node: &Node) -> PathBuf {
    use sha2::{Digest, Sha256};
    let key = hex::encode(Sha256::digest(format!(
        "{}|{:?}|{}",
        node.id, node.blob_id, node.size
    )));
    private.join("transfers").join(format!("{key}.part"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::BufRead, net::TcpListener, thread};

    #[test]
    fn account_root_rejects_collisions_cycles_and_invalid_parents() {
        let directory = Node {
            id: "dir".into(),
            name: "docs".into(),
            parent_id: None,
            blob_id: None,
            role: None,
            kind: Kind::Directory,
            size: 0,
        };
        let file = Node {
            id: "file".into(),
            name: "a.txt".into(),
            parent_id: Some("dir".into()),
            blob_id: Some("blob".into()),
            role: None,
            kind: Kind::File,
            size: 1,
        };
        let mut snapshot = Snapshot {
            state: "1".into(),
            nodes: vec![directory, file],
        };
        let tree = scoped_tree(&snapshot, None).unwrap();
        assert!(tree.contains_key("docs/a.txt"));
        snapshot.nodes[0].parent_id = Some("missing".into());
        assert!(scoped_tree(&snapshot, None).is_err());
        snapshot.nodes[0].parent_id = Some("dir".into());
        assert!(scoped_tree(&snapshot, None).is_err());
        snapshot.nodes[0].parent_id = None;
        snapshot.nodes[0].kind = Kind::File;
        assert!(scoped_tree(&snapshot, None).is_err());
        snapshot.nodes[0].kind = Kind::Directory;
        let mut collision = snapshot.nodes[1].clone();
        collision.id = "other".into();
        collision.name = "A.TXT".into();
        snapshot.nodes.push(collision);
        assert!(scoped_tree(&snapshot, None).is_err());
    }

    struct Request {
        line: String,
        headers: String,
        body: Vec<u8>,
    }

    // A real HTTP boundary catches schema, authentication and streaming mistakes
    // that an in-memory Remote implementation cannot detect.
    fn server(
        count: usize,
        mut handler: impl FnMut(usize, Request) -> String + Send + 'static,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            for index in 0..count {
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "HTTP request {index} did not arrive"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    headers.push_str(&header.to_lowercase());
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let response = handler(
                    index,
                    Request {
                        line,
                        headers,
                        body,
                    },
                );
                socket.write_all(response.as_bytes()).unwrap();
            }
        });
        (address, worker)
    }

    fn response(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
            body.len()
        )
    }

    fn session(modern: bool) -> Value {
        json!({"capabilities": {FILE_CAPABILITY: {}},
            "accounts": {"account": {"isReadOnly":false,"accountCapabilities":{FILE_CAPABILITY: {
                "supportedNodeTypes": if modern { vec!["File", "Directory"] } else { vec!["file", "directory"] }
            }}}}, "primaryAccounts":{FILE_CAPABILITY:"account"},
            "apiUrl":"/jmap", "uploadUrl":"/upload/{accountId}/", "downloadUrl":"/download/{accountId}/{blobId}/{name}?type={type}"})
    }

    #[test]
    fn api_and_transfer_clones_reuse_one_http_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
            for index in 0..21 {
                let mut line = String::new();
                assert!(
                    reader.read_line(&mut line).unwrap() > 0,
                    "HTTP pool was discarded at request {index}"
                );
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let body = if index == 0 { session(true) } else {
                    json!({"methodResponses":[["FileNode/get",{"state":"1","list":[],"notFound":[]},"drive"]]})
                }.to_string();
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let config = Config {
            server: address,
            username: "test@example.org".into(),
            ..Default::default()
        };
        let remote = Jmap::connect_inner(
            &config,
            "fixture".into(),
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .unwrap();
        let transport = remote.transfer_transport();
        assert!(transport.push.is_none());
        assert!(transport.push_url.is_none());
        for _ in 0..10 {
            assert_eq!(remote.current_state().unwrap(), "1");
            assert_eq!(transport.current_state().unwrap(), "1");
        }
        worker.join().unwrap();
    }

    #[test]
    fn advertised_upload_limits_are_discovered_and_enforced_before_sending() {
        let (address, worker) = server(1, |_, _| {
            let mut value = session(true);
            value["capabilities"]["urn:ietf:params:jmap:core"] =
                json!({"maxSizeUpload":4,"maxConcurrentUpload":3});
            response("200 OK", "", &value.to_string())
        });
        let config = Config {
            server: address,
            username: "user".into(),
            parallel_uploads: 8,
            ..Default::default()
        };
        let mut remote = Jmap::connect_inner(
            &config,
            "test".into(),
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .unwrap();
        assert_eq!(remote.upload_limits().effective_size(&config), 4);
        assert_eq!(remote.upload_limits().effective_parallelism(&config), 3);
        // The path need not exist: rejection happens before opening or posting it.
        assert!(remote.upload(Path::new("unused"), 5).is_err());
        let mut lowered = config.clone();
        lowered.max_file_bytes = 2;
        lowered.parallel_uploads = 1;
        assert_eq!(remote.upload_limits().effective_size(&lowered), 2);
        assert_eq!(remote.upload_limits().effective_parallelism(&lowered), 1);
        worker.join().unwrap();
    }

    #[test]
    fn cloud_ranges_validate_offsets_and_support_servers_without_range_requests() {
        for partial in [true, false] {
            let (address, worker) = server(2, move |index, request| {
                if index == 0 {
                    return response("200 OK", "", &session(true).to_string());
                }
                assert!(request.headers.contains("range: bytes=8-13\r\n"));
                assert!(request.headers.contains("accept-encoding: identity\r\n"));
                if partial {
                    response(
                        "206 Partial Content",
                        "Content-Range: bytes 8-13/20\r\n",
                        "89abcd",
                    )
                } else {
                    response("200 OK", "", "0123456789abcdefghij")
                }
            });
            let config = Config {
                server: address,
                username: "user".into(),
                ..Config::default()
            };
            let remote = Jmap::connect_inner(
                &config,
                "test".into(),
                Arc::new(AtomicBool::new(false)),
                true,
            )
            .unwrap();
            let node = Node {
                id: "node".into(),
                name: "range.bin".into(),
                parent_id: None,
                blob_id: Some("blob".into()),
                kind: Kind::File,
                size: 20,
                role: None,
            };
            let mut bytes = Vec::new();
            remote
                .read_range(&node, 8, 6, &AtomicBool::new(false), &mut |part| {
                    bytes.extend_from_slice(part);
                    Ok(())
                })
                .unwrap();
            assert_eq!(bytes, b"89abcd");
            // Cancellation and invalid ranges fail before any further request.
            assert!(remote
                .read_range(&node, 0, 1, &AtomicBool::new(true), &mut |_| panic!(
                    "cancelled data leaked"
                ))
                .is_err());
            assert!(remote
                .read_range(&node, 19, 2, &AtomicBool::new(false), &mut |_| panic!(
                    "invalid range leaked"
                ))
                .is_err());
            worker.join().unwrap();
        }
    }

    #[test]
    fn cloud_ranges_reject_mismatched_server_content_before_exposing_bytes() {
        let (address, worker) = server(2, |index, _| {
            if index == 0 {
                return response("200 OK", "", &session(true).to_string());
            }
            response(
                "206 Partial Content",
                "Content-Range: bytes 0-5/20\r\n",
                "012345",
            )
        });
        let config = Config {
            server: address,
            username: "user".into(),
            ..Config::default()
        };
        let remote = Jmap::connect_inner(
            &config,
            "test".into(),
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .unwrap();
        let node = Node {
            id: "node".into(),
            name: "range.bin".into(),
            parent_id: None,
            blob_id: Some("blob".into()),
            kind: Kind::File,
            size: 20,
            role: None,
        };
        assert!(remote
            .read_range(&node, 8, 6, &AtomicBool::new(false), &mut |_| panic!(
                "wrong bytes exposed"
            ))
            .is_err());
        worker.join().unwrap();
    }

    #[test]
    fn disconnected_upload_retries_the_complete_snapshot_without_publishing_partial_content() {
        let (address, worker) = server(3, |index, request| {
            if index == 0 {
                return response("200 OK", "", &session(false).to_string());
            }
            assert_eq!(request.body, b"complete snapshot");
            if index == 1 {
                return String::new();
            } // Network drops before acknowledgment.
            response(
                "200 OK",
                "",
                &json!({"accountId":"account","blobId":"complete","size":17}).to_string(),
            )
        });
        let config = Config {
            server: address,
            username: "user".into(),
            ..Default::default()
        };
        let mut remote = Jmap::connect_inner(
            &config,
            "test".into(),
            Arc::new(AtomicBool::new(false)),
            true,
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("snapshot");
        std::fs::write(&file, b"complete snapshot").unwrap();
        assert!(remote.upload(&file, 17).is_err());
        assert_eq!(remote.upload(&file, 17).unwrap(), "complete");
        worker.join().unwrap();
    }

    #[test]
    fn server_trash_creation_supports_legacy_stalwart_and_directory_roles() {
        for modern in [false, true] {
            let (address, worker) = server(3, move |index, request| {
                if index == 0 {
                    return response("200 OK", "", &session(modern).to_string());
                }
                let request: Value = serde_json::from_slice(&request.body).unwrap();
                let method = request["methodCalls"][0][0].as_str().unwrap();
                let args = &request["methodCalls"][0][1];
                let result = if index == 1 {
                    assert_eq!(method, "FileNode/set");
                    assert_eq!(args["create"]["drive"]["name"], ".Trash");
                    assert_eq!(
                        args["create"]["drive"]["role"],
                        if modern { json!("trash") } else { Value::Null }
                    );
                    json!({"oldState":"0","newState":"1","created":{"drive":{"id":"trash"}}})
                } else {
                    assert_eq!(method, "FileNode/get");
                    let mut node = json!({"id":"trash","name":".Trash","parentId":null,"role":if modern { Some("trash") } else { None }});
                    node[if modern { "@type" } else { "nodeType" }] =
                        json!(if modern { "Directory" } else { "directory" });
                    json!({"state":"1","list":[node],"notFound":[]})
                };
                response(
                    "200 OK",
                    "",
                    &json!({"methodResponses":[[method,result,"drive"]]}).to_string(),
                )
            });
            let config = Config {
                server: address,
                username: "user@example.org".into(),
                ..Default::default()
            };
            let mut remote = Jmap::connect_inner(
                &config,
                "secret".into(),
                Arc::new(AtomicBool::new(false)),
                true,
            )
            .unwrap();
            let trash = remote.create_trash(None, &mut "0".into()).unwrap();
            assert_eq!(trash.name, ".Trash");
            assert_eq!(
                trash.role.as_deref(),
                if modern { Some("trash") } else { None }
            );
            worker.join().unwrap();
        }
    }

    #[test]
    fn own_upload_is_not_downloaded_when_the_server_returns_a_linked_blob_id() {
        use crate::{engine, store::Store};
        use std::sync::atomic::AtomicUsize;
        for modern in [false, true] {
            let folder = tempfile::tempdir().unwrap();
            let root_name = folder
                .path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            std::fs::write(folder.path().join("a.txt"), b"hello").unwrap();
            let downloads = Arc::new(AtomicUsize::new(0));
            let observed = downloads.clone();
            let mut created = false;
            let (address, worker) = server(11, move |_, request| {
                if request.line.starts_with("GET /.well-known/jmap ") {
                    return response("200 OK", "", &session(modern).to_string());
                }
                if request.line.starts_with("POST /upload/") {
                    return response(
                        "200 OK",
                        "",
                        &json!({"accountId":"account","blobId":"temporary-upload","size":5})
                            .to_string(),
                    );
                }
                if request.line.starts_with("GET /download/") {
                    observed.fetch_add(1, Ordering::Relaxed);
                    return response("200 OK", "", "hello");
                }
                let request: Value = serde_json::from_slice(&request.body).unwrap();
                let call = &request["methodCalls"][0];
                let method = call[0].as_str().unwrap();
                let args = &call[1];
                let state = if created { "2" } else { "1" };
                let result = match method {
                    "FileNode/query" => json!({"queryState":"q1","ids":["root"]}),
                    "FileNode/get" => {
                        let mut list = Vec::new();
                        for id in args["ids"].as_array().unwrap() {
                            let mut node = if id == "root" {
                                json!({"id":"root","name":root_name,"parentId":null})
                            } else {
                                assert!(created);
                                json!({"id":"file","name":"a.txt","parentId":"root","blobId":"linked-file-content","size":5})
                            };
                            node[if modern { "@type" } else { "nodeType" }] =
                                json!(match (modern, id == "root") {
                                    (true, true) => "Directory",
                                    (true, false) => "File",
                                    (false, true) => "directory",
                                    (false, false) => "file",
                                });
                            list.push(node);
                        }
                        json!({"state":state,"list":list,"notFound":[]})
                    }
                    "FileNode/set" => {
                        assert_eq!(args["create"]["drive"]["blobId"], "temporary-upload");
                        created = true;
                        json!({"oldState":"1","newState":"2","created":{"drive":{"id":"file"}}})
                    }
                    "FileNode/changes" => {
                        json!({"oldState":args["sinceState"],"newState":"2","hasMoreChanges":false,"created":["file"],"updated":[],"destroyed":[]})
                    }
                    _ => panic!("Unexpected method {method}"),
                };
                response(
                    "200 OK",
                    "",
                    &json!({"methodResponses":[[method,result,"drive"]]}).to_string(),
                )
            });
            let config = Config {
                server: address,
                username: "test@example.org".into(),
                local_folder: folder.path().into(),
                file_settle_seconds: 0,
                ..Config::default()
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let mut remote =
                Jmap::connect_inner(&config, "secret".into(), cancel.clone(), true).unwrap();
            let mut store = Store::open(&config).unwrap();
            // The public engine validates HTTPS. This isolated HTTP transport uses the
            // same profile identity but no production account or external endpoint.
            let mut engine_config = config.clone();
            engine_config.server = "https://jmap.example.com".into();
            let first = engine::sync(
                &engine_config,
                &mut store,
                &mut remote,
                &cancel,
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(first.uploaded, 1);
            let second = engine::sync(
                &engine_config,
                &mut store,
                &mut remote,
                &cancel,
                &mut |_| {},
            )
            .unwrap();
            worker.join().unwrap();
            assert_eq!(
                second.downloaded, 0,
                "an acknowledged upload must not become a download"
            );
            assert_eq!(downloads.load(Ordering::Relaxed), 0);
            assert_eq!(
                store.baselines().unwrap()["a.txt"].remote_blob.as_deref(),
                Some("linked-file-content")
            );
        }
    }

    #[test]
    fn http_round_trip_supports_both_schemas_conditional_writes_and_resumed_download() {
        for (modern, bearer) in [(false, false), (true, false), (false, true), (true, true)] {
            let (address, worker) = server(14, move |index, request| {
                assert!(request.headers.contains(if bearer {
                    "authorization: bearer test-token"
                } else {
                    "authorization: basic "
                }));
                if index == 0 {
                    assert!(request.line.starts_with("GET /.well-known/jmap "));
                    return response("200 OK", "", &session(modern).to_string());
                }
                if index == 5 {
                    assert!(request.line.starts_with("POST /upload/account/ "));
                    assert_eq!(request.body, b"hello");
                    return response(
                        "200 OK",
                        "",
                        &json!({"accountId":"account","blobId":"uploaded","size":5}).to_string(),
                    );
                }
                if index == 10 {
                    assert!(request.headers.contains("range: bytes=2-"));
                    assert!(request
                        .line
                        .starts_with("GET /download/account/linked-updated/renamed.txt?"));
                    return response(
                        "206 Partial Content",
                        "Content-Range: bytes 2-4/5\r\n",
                        "llo",
                    );
                }
                let request: Value = serde_json::from_slice(&request.body).unwrap();
                let call = &request["methodCalls"][0];
                let method = call[0].as_str().unwrap();
                let args = &call[1];
                assert_eq!(args["accountId"], "account");
                let mut file = json!({"id":"file","name":"a.txt","parentId":"root","blobId":"original","size":5});
                file[if modern { "@type" } else { "nodeType" }] =
                    json!(if modern { "File" } else { "file" });
                let mut root = json!({"id":"root","name":"FileNode","parentId":null});
                root[if modern { "@type" } else { "nodeType" }] =
                    json!(if modern { "Directory" } else { "directory" });
                let result = match index {
                    1 | 4 => {
                        assert_eq!(method, "FileNode/get");
                        assert_eq!(args["ids"], json!([]));
                        json!({"state":"1","list":[],"notFound":[]})
                    }
                    2 => {
                        assert_eq!(method, "FileNode/query");
                        json!({"queryState":"q1","ids":["root","file"]})
                    }
                    3 => {
                        assert_eq!(method, "FileNode/get");
                        json!({"state":"1","list":[root,file],"notFound":[]})
                    }
                    6 => {
                        assert_eq!(method, "FileNode/set");
                        assert_eq!(args["ifInState"], "1");
                        let node = &args["create"]["drive"];
                        assert_eq!(
                            node[if modern { "@type" } else { "nodeType" }],
                            if modern { "File" } else { "file" }
                        );
                        assert_eq!(
                            node[if modern { "mediaType" } else { "type" }],
                            "application/octet-stream"
                        );
                        assert_eq!(node["blobId"], "uploaded");
                        json!({"oldState":"1","newState":"2","created":{"drive":{"id":"new"}}})
                    }
                    8 => {
                        assert_eq!(method, "FileNode/set");
                        assert_eq!(args["ifInState"], "2");
                        assert_eq!(args["update"]["new"]["name"], "renamed.txt");
                        assert_eq!(args["update"]["new"]["blobId"], "uploaded");
                        json!({"oldState":"2","newState":"3","updated":{"new":null}})
                    }
                    11 => {
                        assert_eq!(method, "FileNode/changes");
                        assert_eq!(args["sinceState"], "1");
                        json!({"oldState":"1","newState":"3","hasMoreChanges":false,"created":["new"],"updated":[],"destroyed":["file"]})
                    }
                    7 | 9 | 12 => {
                        assert_eq!(method, "FileNode/get");
                        assert_eq!(args["ids"], json!(["new"]));
                        file["id"] = json!("new");
                        file["name"] = json!(if index == 7 { "new.txt" } else { "renamed.txt" });
                        file["blobId"] = json!(if index == 7 {
                            "linked-created"
                        } else {
                            "linked-updated"
                        });
                        json!({"state":if index == 7 { "2" } else { "3" },"list":[file],"notFound":[]})
                    }
                    13 => json!({"state":"3","list":[],"notFound":[]}),
                    _ => unreachable!(),
                };
                response(
                    "200 OK",
                    "",
                    &json!({"methodResponses":[[method,result,"drive"]]}).to_string(),
                )
            });
            let config = Config {
                server: address,
                username: "test@example.org".into(),
                ..Default::default()
            };
            let mut remote = Jmap::connect_authorized_inner(
                &config,
                if bearer {
                    Authentication::test_bearer("test-token")
                } else {
                    Authentication::basic(&config.username, "test-secret".into())
                },
                Arc::new(AtomicBool::new(false)),
                true,
            )
            .unwrap();
            let snapshot = remote.snapshot(None).unwrap();
            assert_eq!(snapshot.nodes.len(), 2);
            let folder = tempfile::tempdir().unwrap();
            let source = folder.path().join("source");
            std::fs::write(&source, b"hello").unwrap();
            let blob = remote.upload(&source, 5).unwrap();
            let mut state = snapshot.state.clone();
            let node = remote
                .create(
                    "new.txt",
                    Some("root"),
                    Kind::File,
                    Some(&blob),
                    5,
                    &mut state,
                )
                .unwrap();
            assert_eq!(node.blob_id.as_deref(), Some("linked-created"));
            let node = remote
                .update(
                    &node,
                    "renamed.txt",
                    Some("root"),
                    Some((&blob, 5)),
                    &mut state,
                )
                .unwrap();
            assert_eq!(node.blob_id.as_deref(), Some("linked-updated"));
            assert_eq!(state, "3");
            let staged = folder.path().join("resumed.part");
            std::fs::write(&staged, b"he").unwrap();
            remote.download(&node, &staged).unwrap();
            assert_eq!(std::fs::read(staged).unwrap(), b"hello");
            let updated = remote.snapshot(Some(&snapshot)).unwrap();
            assert_eq!(updated.nodes.len(), 2);
            assert!(updated
                .nodes
                .iter()
                .any(|n| n.id == "new" && n.name == "renamed.txt"));
            worker.join().unwrap();
        }
    }

    #[test]
    fn unsafe_discovery_urls_are_rejected_before_any_followup_request() {
        let (address, worker) = server(1, |_, _| {
            let mut session = session(false);
            session["apiUrl"] = json!("https://other.example/jmap");
            response("200 OK", "", &session.to_string())
        });
        let config = Config {
            server: address,
            ..Default::default()
        };
        assert!(Jmap::connect_inner(
            &config,
            "test-secret".into(),
            Arc::new(AtomicBool::new(false)),
            true
        )
        .is_err());
        worker.join().unwrap();
    }
    #[test]
    fn origins_do_not_leak_credentials() {
        let base = Url::parse("https://mail.example").unwrap();
        for url in [
            "http://mail.example/jmap",
            "https://evil.example/jmap",
            "https://user:pass@mail.example/jmap",
        ] {
            assert!(same_origin(&base, &Url::parse(url).unwrap()).is_err());
        }
    }
    #[test]
    fn supports_both_stalwart_schemas() {
        for raw in [
            json!({"id":"1","name":"test","nodeType":"file","blobId":"b","size":1}),
            json!({"id":"1","name":"test","@type":"File","blobId":"b","size":1}),
        ] {
            assert_eq!(parse_node(&raw).unwrap().kind, Kind::File);
        }
    }
}
