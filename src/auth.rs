//! Public-client OAuth: system browser, PKCE S256, a temporary loopback listener,
//! and rotating tokens in the OS credential store. ID tokens are never used as API credentials.
use crate::config::Config;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const MAX_DOCUMENT: u64 = 128 * 1024;
pub const FILE_SCOPE: &str = "urn:ietf:params:oauth:scope:files";

#[derive(Debug)]
pub struct SignInRequired;
impl std::fmt::Display for SignInRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Browser sign-in required. Open account settings and sign in again.")
    }
}
impl std::error::Error for SignInRequired {}

#[derive(Clone)]
pub enum Authentication {
    Basic {
        username: String,
        password: Arc<Zeroizing<String>>,
    },
    OAuth(Arc<Mutex<TokenSession>>),
}
impl Authentication {
    #[cfg(test)]
    pub(crate) fn test_bearer(token: &str) -> Self {
        Self::OAuth(Arc::new(Mutex::new(TokenSession {
            client: http_client().unwrap(),
            credential_key: None,
            test_http: true,
            tokens: Tokens {
                issuer: "http://127.0.0.1/".into(),
                client_id: "test".into(),
                token_endpoint: "http://127.0.0.1/token".into(),
                resource: "test".into(),
                scopes: FILE_SCOPE.into(),
                access_token: token.into(),
                refresh_token: "test".into(),
                expires_at: u64::MAX,
            },
        })))
    }
    pub fn basic(username: &str, password: String) -> Self {
        Self::Basic {
            username: username.into(),
            password: Arc::new(Zeroizing::new(password)),
        }
    }
    pub fn apply(&self, request: RequestBuilder) -> Result<RequestBuilder> {
        match self {
            Self::Basic { username, password } => {
                Ok(request.basic_auth(username, Some(password.as_str())))
            }
            Self::OAuth(session) => Ok(request.bearer_auth(
                session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Token session unavailable"))?
                    .access_token()?
                    .as_str(),
            )),
        }
    }
    pub fn observe(&self, status: StatusCode) -> Result<()> {
        if status == StatusCode::UNAUTHORIZED {
            if let Self::OAuth(session) = self {
                let mut session = session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Token session unavailable"))?;
                session.tokens.expires_at = 0;
                session.save()?;
            }
        }
        Ok(())
    }
}

pub fn validate_issuer(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("Invalid OAuth issuer URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("OAuth issuer must use HTTPS without credentials, query or fragment");
    }
    Ok(url)
}
fn endpoint(issuer: &Url, raw: &str, test_http: bool) -> Result<Url> {
    let target = Url::parse(raw).context("Invalid OAuth endpoint")?;
    if (target.scheme() != "https" && !(test_http && target.scheme() == "http"))
        || target.origin() != issuer.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        bail!("OAuth endpoint must stay on the configured issuer origin");
    }
    Ok(target)
}
fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("NoriDrive/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
fn document(response: Response) -> Result<Value> {
    let status = response.status();
    let mut bytes = Zeroizing::new(Vec::new());
    response.take(MAX_DOCUMENT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DOCUMENT {
        bail!("OAuth response exceeds size limit");
    }
    let value: Value = serde_json::from_slice(&bytes).context("Invalid OAuth response")?;
    if !status.is_success() {
        if value["error"] == "invalid_grant" || value["error"] == "invalid_client" {
            return Err(SignInRequired.into());
        }
        // Provider error descriptions can contain personal data; do not log response bodies.
        bail!("OAuth endpoint returned HTTP {status}");
    }
    Ok(value)
}

#[derive(Clone, Deserialize)]
struct Metadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}
fn discover(client: &Client, issuer: &Url, scopes: &str, test_http: bool) -> Result<Metadata> {
    let suffix = issuer.path().trim_matches('/');
    let mut oauth_url = issuer.clone();
    oauth_url.set_path(&format!(
        "/.well-known/oauth-authorization-server{}{}",
        if suffix.is_empty() { "" } else { "/" },
        suffix
    ));
    let oidc_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.as_str().trim_end_matches('/')
    );
    let response = client.get(oauth_url).send()?;
    let value = if response.status() == StatusCode::NOT_FOUND
        || response.status() == StatusCode::METHOD_NOT_ALLOWED
    {
        document(client.get(oidc_url).send()?)?
    } else {
        document(response)?
    };
    let metadata: Metadata =
        serde_json::from_value(value).context("Incomplete OAuth discovery metadata")?;
    if metadata.issuer.trim_end_matches('/') != issuer.as_str().trim_end_matches('/') {
        bail!("OAuth discovery issuer mismatch");
    }
    endpoint(issuer, &metadata.authorization_endpoint, test_http)?;
    endpoint(issuer, &metadata.token_endpoint, test_http)?;
    if !metadata
        .code_challenge_methods_supported
        .iter()
        .any(|method| method == "S256")
    {
        bail!("OAuth provider does not advertise PKCE S256");
    }
    // Some OIDC providers (including authentik) list confidential authentication
    // methods here even for public applications. Always use PKCE without a secret;
    // the provider must accept the configured client as public at token exchange.
    if !metadata.scopes_supported.is_empty()
        && scopes
            .split_ascii_whitespace()
            .any(|scope| !metadata.scopes_supported.iter().any(|known| known == scope))
    {
        bail!("OAuth provider does not advertise the requested scopes. This Stalwart version may not support the files scope; update the server or configure a provider/client that grants file access. No broader permissions were requested.");
    }
    Ok(metadata)
}

#[derive(Serialize, Deserialize)]
struct Tokens {
    issuer: String,
    client_id: String,
    token_endpoint: String,
    resource: String,
    scopes: String,
    #[serde(skip)]
    access_token: String,
    refresh_token: String,
    #[serde(skip)]
    expires_at: u64,
}
impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}
pub struct TokenSession {
    client: Client,
    tokens: Tokens,
    credential_key: Option<String>,
    test_http: bool,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn take_secret(value: &mut Value, key: &str) -> Result<Zeroizing<String>> {
    match value[key].take() {
        Value::String(secret) if !secret.is_empty() && secret.len() < 64 * 1024 => {
            Ok(Zeroizing::new(secret))
        }
        _ => bail!("OAuth response lacks a valid {key}"),
    }
}
impl TokenSession {
    fn save(&self) -> Result<()> {
        if let Some(key) = &self.credential_key {
            let serialized = Zeroizing::new(serde_json::to_string(&self.tokens)?);
            save_credential(key, &serialized)?;
        }
        Ok(())
    }
    fn access_token(&mut self) -> Result<Zeroizing<String>> {
        if self.tokens.expires_at <= now().saturating_add(30) {
            crate::runtime::check()?;
            let issuer = Url::parse(&self.tokens.issuer)?;
            let target = endpoint(&issuer, &self.tokens.token_endpoint, self.test_http)?;
            let response = self
                .client
                .post(target)
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", self.tokens.client_id.as_str()),
                    ("refresh_token", self.tokens.refresh_token.as_str()),
                    ("resource", self.tokens.resource.as_str()),
                ])
                .send()?;
            let mut value = document(response)?;
            self.update(&mut value, false)?;
            self.save()?; // Persist rotation before using the new token.
        }
        Ok(Zeroizing::new(self.tokens.access_token.clone()))
    }
    fn update(&mut self, value: &mut Value, require_refresh: bool) -> Result<()> {
        if !value["token_type"]
            .as_str()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer"))
        {
            bail!("OAuth token type must be Bearer");
        }
        let expires = value["expires_in"]
            .as_u64()
            .filter(|expires| *expires > 30)
            .context("OAuth access token must declare a lifetime over 30 seconds")?;
        if let Some(granted) = value["scope"].as_str() {
            if self
                .tokens
                .scopes
                .split_ascii_whitespace()
                .filter(|scope| *scope != "offline_access")
                .any(|requested| {
                    !granted
                        .split_ascii_whitespace()
                        .any(|scope| scope == requested)
                })
            {
                bail!("OAuth server did not grant the requested scopes");
            }
        }
        let access = take_secret(value, "access_token")?;
        let refresh = if value["refresh_token"].is_string() {
            Some(take_secret(value, "refresh_token")?)
        } else if require_refresh {
            bail!("OAuth provider did not return a refresh token")
        } else {
            None
        };
        self.tokens.access_token.zeroize();
        self.tokens.access_token = access.to_string();
        if let Some(refresh) = refresh {
            self.tokens.refresh_token.zeroize();
            self.tokens.refresh_token = refresh.to_string();
        }
        self.tokens.expires_at = now().saturating_add(expires);
        // This is an OAuth resource client, not an ID-token consumer. Never decode or trust it.
        if let Some(Value::String(id_token)) = value.get_mut("id_token") {
            id_token.zeroize();
        }
        Ok(())
    }
}

#[cfg(feature = "desktop")]
fn load_credential(key: &str) -> Result<Option<Zeroizing<String>>> {
    match keyring::Entry::new("de.noritec.drive", key)?.get_password() {
        Ok(secret) => Ok(Some(Zeroizing::new(secret))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).context("OAuth credential store unavailable"),
    }
}
#[cfg(not(feature = "desktop"))]
fn load_credential(_: &str) -> Result<Option<Zeroizing<String>>> {
    Ok(None)
}
#[cfg(feature = "desktop")]
fn save_credential(key: &str, secret: &str) -> Result<()> {
    keyring::Entry::new("de.noritec.drive", key)?
        .set_password(secret)
        .context("Cannot persist OAuth token rotation in OS credential store")
}
#[cfg(not(feature = "desktop"))]
fn save_credential(_: &str, _: &str) -> Result<()> {
    bail!("Browser authentication requires OS credential storage (desktop feature)")
}

fn random_url_secret() -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut())
        .map_err(|_| anyhow::anyhow!("Operating system random generator unavailable"))?;
    Ok(Zeroizing::new(URL_SAFE_NO_PAD.encode(*bytes)))
}
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn browser_authentication(
    config: &Config,
    interactive: bool,
    cancel: &AtomicBool,
    mut open: impl FnMut(&Url),
) -> Result<Authentication> {
    let issuer = validate_issuer(config.oauth_issuer_url())?;
    let client = http_client()?;
    let key = config.oauth_credential_key();
    if !interactive {
        if let Some(secret) = load_credential(&key)? {
            if secret.len() as u64 > MAX_DOCUMENT {
                bail!("Stored OAuth credential exceeds size limit");
            }
            let tokens: Tokens = serde_json::from_str(&secret)
                .context("Invalid stored OAuth session; sign in again")?;
            if tokens.issuer != issuer.as_str()
                || tokens.resource != config.server
                || tokens.scopes != config.oauth_scopes
                || (!config.oauth_client_id.is_empty()
                    && tokens.client_id != config.oauth_client_id)
            {
                bail!("Stored OAuth session does not match this profile");
            }
            endpoint(&issuer, &tokens.token_endpoint, false)?;
            return Ok(Authentication::OAuth(Arc::new(Mutex::new(TokenSession {
                client,
                tokens,
                credential_key: Some(key),
                test_http: false,
            }))));
        }
        return Err(SignInRequired.into());
    }
    let metadata = discover(&client, &issuer, &config.oauth_scopes, false)?;
    let mut session = authorize(config, &client, &issuer, metadata, cancel, &mut open, false)?;
    session.credential_key = Some(key);
    session.save()?;
    Ok(Authentication::OAuth(Arc::new(Mutex::new(session))))
}

fn authorize(
    config: &Config,
    client: &Client,
    issuer: &Url,
    metadata: Metadata,
    cancel: &AtomicBool,
    open: &mut impl FnMut(&Url),
    test_http: bool,
) -> Result<TokenSession> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, config.oauth_redirect_port))
        .context("OAuth callback port unavailable")?;
    listener.set_nonblocking(true)?;
    let redirect = format!(
        "http://127.0.0.1:{}/oauth/callback",
        listener.local_addr()?.port()
    );
    let client_id = if config.oauth_client_id.is_empty() {
        let target = endpoint(
            issuer,
            metadata
                .registration_endpoint
                .as_deref()
                .context("OAuth provider needs an administrator-registered public client_id")?,
            test_http,
        )?;
        let value = document(client.post(target).json(&json!({
            "client_name":"NoriDrive", "application_type":"native", "redirect_uris":[redirect],
            "token_endpoint_auth_method":"none", "grant_types":["authorization_code","refresh_token"],
            "response_types":["code"], "scope":config.oauth_scopes,
        })).send()?)?;
        value["client_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 4096)
            .context("OAuth registration returned no client_id")?
            .to_owned()
    } else {
        config.oauth_client_id.clone()
    };
    let verifier = random_url_secret()?;
    let state = random_url_secret()?;
    let mut authorization = endpoint(issuer, &metadata.authorization_endpoint, test_http)?;
    authorization.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", &client_id),
        ("redirect_uri", &redirect),
        ("scope", &config.oauth_scopes),
        ("resource", &config.server),
        ("state", state.as_str()),
        ("code_challenge", &pkce_challenge(&verifier)),
        ("code_challenge_method", "S256"),
        ("login_hint", &config.username),
    ]);
    open(&authorization);
    let code = wait_for_callback(
        listener,
        &redirect,
        &state,
        issuer,
        cancel,
        Duration::from_secs(600),
    )?;
    let target = endpoint(issuer, &metadata.token_endpoint, test_http)?;
    let mut value = document(
        client
            .post(target)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", &client_id),
                ("redirect_uri", &redirect),
                ("code", code.as_str()),
                ("code_verifier", verifier.as_str()),
                ("resource", &config.server),
            ])
            .send()?,
    )?;
    let mut session = TokenSession {
        client: client.clone(),
        credential_key: None,
        test_http,
        tokens: Tokens {
            issuer: issuer.as_str().into(),
            client_id,
            token_endpoint: metadata.token_endpoint,
            resource: config.server.clone(),
            scopes: config.oauth_scopes.clone(),
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
        },
    };
    session.update(&mut value, true)?;
    Ok(session)
}

fn callback_parameters(
    request: &str,
    redirect: &str,
    state: &str,
    issuer: &Url,
) -> Result<Zeroizing<String>> {
    let base = Url::parse(redirect)?;
    let mut lines = request.split("\r\n");
    let first: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    if first.len() != 3
        || first[0] != "GET"
        || !first[1].starts_with("/oauth/callback?")
        || first[2] != "HTTP/1.1"
    {
        bail!("Invalid OAuth callback request");
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .collect();
    let authority = format!(
        "127.0.0.1:{}",
        base.port().context("Callback port missing")?
    );
    if hosts.len() != 1 || hosts[0].1.trim() != authority {
        bail!("OAuth callback host mismatch");
    }
    let target = base.join(first[1])?;
    let mut params = std::collections::HashMap::new();
    for (key, value) in target.query_pairs() {
        if params
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            bail!("Duplicate OAuth callback parameter");
        }
    }
    if params.get("state").is_none_or(|value| value != state) {
        bail!("OAuth callback state mismatch");
    }
    if params
        .get("iss")
        .is_some_and(|value| Url::parse(value).ok().as_ref() != Some(issuer))
    {
        bail!("OAuth callback issuer mismatch");
    }
    if params.contains_key("error") {
        return Err(SignInRequired.into());
    }
    Ok(Zeroizing::new(
        params
            .remove("code")
            .filter(|code| !code.is_empty() && code.len() <= 4096)
            .context("OAuth callback has no code")?,
    ))
}
fn wait_for_callback(
    listener: TcpListener,
    redirect: &str,
    state: &str,
    issuer: &Url,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Zeroizing<String>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let result = read_callback(&mut stream)
                    .and_then(|request| callback_parameters(&request, redirect, state, issuer));
                let valid = result.is_ok();
                let declined = result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.downcast_ref::<SignInRequired>().is_some());
                let (status, body) = if valid {
                    ("200 OK", "NoriDrive: Browser response received. Return to the application to complete sign-in. / Browser-Antwort empfangen. Zurueck zu NoriDrive, um die Anmeldung abzuschliessen.")
                } else {
                    ("400 Bad Request", "NoriDrive: Sign-in not completed. Return to the application. / Anmeldung nicht abgeschlossen. Zur App zurueckkehren.")
                };
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                if valid || declined {
                    return result;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Browser sign-in timed out or was cancelled")
}
fn read_callback(stream: &mut TcpStream) -> Result<Zeroizing<String>> {
    // Winsock can inherit the listener's nonblocking mode on accepted sockets.
    // Blocking reads with a deadline are required to receive fragmented HTTP headers.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut data = Zeroizing::new(Vec::new());
    let mut buffer = [0u8; 1024];
    while data.len() <= 8192 && Instant::now() < deadline {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            bail!("Incomplete OAuth callback");
        }
        data.extend_from_slice(&buffer[..count]);
        if data.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(Zeroizing::new(String::from_utf8(data.to_vec())?));
        }
    }
    bail!("OAuth callback exceeds limits")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::BufRead, thread};

    #[test]
    fn fragmented_callback_headers_work_with_a_nonblocking_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(b"GET /oauth/callback").unwrap();
            thread::sleep(Duration::from_millis(100));
            stream
                .write_all(b"?code=example HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        assert!(read_callback(&mut stream)
            .unwrap()
            .starts_with("GET /oauth/callback?code=example HTTP/1.1\r\n"));
        client.join().unwrap();
    }

    // Disposable loopback HTTP is test-only. Production endpoints always require HTTPS.
    fn server(
        count: usize,
        handler: impl Fn(usize, &str, &str, &str, &str) -> (u16, Value) + Send + 'static,
    ) -> (Url, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let issuer = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let base = issuer.to_string();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            for index in 0..count {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "Missing OAuth test request {index}"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0;
                let mut headers = String::new();
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
                    headers.push_str(&header.to_ascii_lowercase());
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (status, value) = handler(
                    index,
                    &base,
                    &first,
                    &headers,
                    std::str::from_utf8(&body).unwrap(),
                );
                let body = value.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (issuer, worker)
    }
    fn metadata(base: &str) -> Value {
        json!({"issuer":base,"authorization_endpoint":format!("{base}authorize"),"token_endpoint":format!("{base}token"),
            "registration_endpoint":format!("{base}register"),"scopes_supported":[FILE_SCOPE,"offline_access"],
            "code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"]})
    }
    #[test]
    fn pkce_matches_rfc7636_and_callback_checks_state_host_issuer_and_duplicates() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let issuer = Url::parse("https://auth.example.org/").unwrap();
        let request = "GET /oauth/callback?code=secret-code&state=expected&iss=https%3A%2F%2Fauth.example.org HTTP/1.1\r\nHost: 127.0.0.1:54321\r\n\r\n";
        assert_eq!(
            callback_parameters(
                request,
                "http://127.0.0.1:54321/oauth/callback",
                "expected",
                &issuer
            )
            .unwrap()
            .as_str(),
            "secret-code"
        );
        for broken in [
            request.replace("state=expected", "state=attacker"),
            request.replace("state=expected", "state=expected&state=attacker"),
            request.replace("127.0.0.1:54321", "attacker.example"),
            request.replace("auth.example.org", "attacker.example"),
            request.replace("GET ", "POST "),
        ] {
            assert!(callback_parameters(
                &broken,
                "http://127.0.0.1:54321/oauth/callback",
                "expected",
                &issuer
            )
            .is_err());
        }
        assert_ne!(
            random_url_secret().unwrap().as_str(),
            random_url_secret().unwrap().as_str()
        );
        assert!(validate_issuer("http://auth.example.org").is_err());
        assert!(endpoint(&issuer, "https://attacker.example/token", false).is_err());
    }
    #[test]
    fn browser_flow_registers_pkce_exchanges_code_rotates_refresh_and_uses_bearer() {
        let challenge = Arc::new(Mutex::new(String::new()));
        let observed = challenge.clone();
        let (issuer, worker) = server(6, move |index, base, first, headers, body| {
            assert!(!headers.contains("authorization: basic"));
            match index {
                0 => {
                    assert!(first.contains("/.well-known/oauth-authorization-server"));
                    (200, metadata(base))
                }
                1 => {
                    assert!(first.starts_with("POST /register "));
                    let value: Value = serde_json::from_str(body).unwrap();
                    assert_eq!(value["client_name"], "NoriDrive");
                    assert_eq!(value["token_endpoint_auth_method"], "none");
                    assert!(value.get("client_secret").is_none());
                    (201, json!({"client_id":"registered-test-client"}))
                }
                2 => {
                    let form: std::collections::HashMap<_, _> =
                        url::form_urlencoded::parse(body.as_bytes())
                            .into_owned()
                            .collect();
                    assert_eq!(form["grant_type"], "authorization_code");
                    assert_eq!(form["code"], "test-code");
                    assert_eq!(
                        pkce_challenge(&form["code_verifier"]),
                        *observed.lock().unwrap()
                    );
                    (
                        200,
                        json!({"token_type":"Bearer","access_token":"first-access","refresh_token":"first-refresh","expires_in":3600,"scope":FILE_SCOPE}),
                    )
                }
                3 => {
                    assert!(body.contains("grant_type=refresh_token"));
                    assert!(body.contains("refresh_token=first-refresh"));
                    (
                        200,
                        json!({"token_type":"Bearer","access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600,"scope":FILE_SCOPE}),
                    )
                }
                4 => {
                    assert!(first.starts_with("GET /resource "));
                    assert!(headers.contains("authorization: bearer rotated-access"));
                    (200, json!({"ok":true}))
                }
                5 => {
                    assert!(body.contains("refresh_token=rotated-refresh"));
                    (
                        400,
                        json!({"error":"invalid_grant","error_description":"must not be logged"}),
                    )
                }
                _ => unreachable!(),
            }
        });
        let config = Config {
            server: issuer.to_string(),
            username: "test@example.org".into(),
            ..Default::default()
        };
        let client = http_client().unwrap();
        let discovered = discover(&client, &issuer, &config.oauth_scopes, true).unwrap();
        let callback = Arc::new(Mutex::new(None));
        let saved_callback = callback.clone();
        let mut session = authorize(&config, &client, &issuer, discovered, &AtomicBool::new(false), &mut |url| {
            let pairs: std::collections::HashMap<_,_> = url.query_pairs().into_owned().collect();
            assert_eq!(pairs["code_challenge_method"], "S256"); *challenge.lock().unwrap() = pairs["code_challenge"].clone();
            let redirect = Url::parse(&pairs["redirect_uri"]).unwrap(); let state = pairs["state"].clone();
            *saved_callback.lock().unwrap() = Some(thread::spawn(move || {
                // An unrelated first request must not consume the real callback.
                for state in ["wrong-state".to_owned(), state] {
                    let mut stream = TcpStream::connect(("127.0.0.1", redirect.port().unwrap())).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    write!(stream, "GET /oauth/callback?state={state}&code=test-code HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n", redirect.port().unwrap()).unwrap();
                    let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
                    assert!(response.starts_with(if state == "wrong-state" { "HTTP/1.1 400" } else { "HTTP/1.1 200" }));
                }
            }));
        }, true).unwrap();
        callback.lock().unwrap().take().unwrap().join().unwrap();
        let saved = Zeroizing::new(serde_json::to_string(&session.tokens).unwrap());
        assert!(!saved.contains("first-access"));
        assert!(saved.contains("first-refresh"));
        session.tokens.expires_at = 0;
        let authentication = Authentication::OAuth(Arc::new(Mutex::new(session)));
        assert!(authentication
            .apply(client.get(issuer.join("resource").unwrap()))
            .unwrap()
            .send()
            .unwrap()
            .status()
            .is_success());
        authentication.observe(StatusCode::UNAUTHORIZED).unwrap();
        let error = authentication
            .apply(client.get(issuer.join("resource").unwrap()))
            .err()
            .unwrap();
        assert!(error.downcast_ref::<SignInRequired>().is_some());
        worker.join().unwrap();
    }
    #[test]
    fn oidc_discovery_fallback_accepts_public_client_without_none_in_metadata() {
        let (issuer, worker) = server(2, |index, base, first, _, _| {
            if index == 0 {
                (404, json!({"error":"not_found"}))
            } else {
                assert!(first.contains("/.well-known/openid-configuration"));
                let mut value = metadata(base);
                value["token_endpoint_auth_methods_supported"] =
                    json!(["client_secret_post", "client_secret_basic"]);
                (200, value)
            }
        });
        discover(&http_client().unwrap(), &issuer, FILE_SCOPE, true).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn missing_files_scope_fails_before_registration_or_opening_browser() {
        let (issuer, worker) = server(1, |_, base, _, _, _| {
            let mut value = metadata(base);
            value["scopes_supported"] = json!(["openid", "offline_access"]);
            (200, value)
        });
        let error = discover(&http_client().unwrap(), &issuer, FILE_SCOPE, true)
            .err()
            .unwrap();
        assert!(error.to_string().contains("files scope"));
        worker.join().unwrap();
    }
    #[test]
    fn callback_wait_can_be_cancelled_and_listener_is_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let started = Instant::now();
        assert!(wait_for_callback(
            listener,
            &format!("http://127.0.0.1:{port}/oauth/callback"),
            "state",
            &Url::parse("https://auth.example.org/").unwrap(),
            &AtomicBool::new(true),
            Duration::from_secs(600)
        )
        .is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
    }
}
