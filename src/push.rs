//! RFC 8887 push only. File payload and conditional API writes stay on HTTPS.
//! Notifications are hints: reconnect always schedules a fresh remote check.
use crate::{auth::Authentication, runtime::Policy};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io,
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    client::IntoClientRequest, protocol::WebSocketConfig, stream::MaybeTlsStream, Message,
};
use url::Url;

pub struct Push {
    signal: Arc<Signal>,
}
#[derive(Default)]
struct Signal {
    changed: AtomicBool,
    stop: AtomicBool,
}
impl Drop for Push {
    fn drop(&mut self) {
        self.signal.stop.store(true, Ordering::Relaxed);
    }
}
impl Push {
    pub(crate) fn start(
        url: Url,
        authentication: Authentication,
        account: String,
        policy: Arc<Policy>,
    ) -> Arc<Self> {
        let signal = Arc::new(Signal::default());
        let running = signal.clone();
        thread::spawn(move || {
            let mut delay = 1;
            while !running.stop.load(Ordering::Relaxed) && !policy.cancel.load(Ordering::Relaxed) {
                if policy.power_paused() {
                    wait(&running, &policy, Duration::from_millis(250));
                    continue;
                }
                let connected = connection(&url, &authentication, &account, &policy, &running);
                if connected.is_ok() {
                    delay = 1;
                }
                if !running.stop.load(Ordering::Relaxed) && policy.check().is_ok() {
                    crate::logger::event(
                        crate::logger::Level::Warning,
                        220,
                        "push_reconnect",
                        None,
                        "Push channel unavailable; periodic checks continue while reconnecting",
                    );
                    wait(&running, &policy, Duration::from_secs(delay));
                    delay = (delay * 2).min(60);
                }
            }
        });
        Arc::new(Self { signal })
    }
    pub fn take_change(&self) -> bool {
        self.signal.changed.swap(false, Ordering::Relaxed)
    }
}

fn wait(signal: &Signal, policy: &Policy, duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until
        && !signal.stop.load(Ordering::Relaxed)
        && !policy.cancel.load(Ordering::Relaxed)
    {
        thread::sleep(
            until
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250)),
        );
    }
}

/// Only an explicitly advertised same-origin TLS endpoint receives credentials.
pub fn endpoint(origin: &Url, capability: &Value) -> Option<Url> {
    if capability["supportsPush"] != true {
        return None;
    }
    let url = Url::parse(capability["url"].as_str()?).ok()?;
    if url.scheme() != "wss"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if url.host_str() != origin.host_str()
        || url.port_or_known_default() != origin.port_or_known_default()
    {
        return None;
    }
    Some(url)
}

fn connection(
    url: &Url,
    authentication: &Authentication,
    account: &str,
    policy: &Policy,
    signal: &Signal,
) -> Result<()> {
    policy.check()?;
    let mut connected = None;
    for address in (
        url.host_str().context("Push host is missing")?,
        url.port_or_known_default()
            .context("Push port is missing")?,
    )
        .to_socket_addrs()?
        .take(8)
    {
        policy.check()?;
        if signal.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
            connected = Some(stream);
            break;
        }
    }
    let stream = connected.context("Push connection unavailable")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut request = url.as_str().into_client_request()?;
    let mut authorization =
        tungstenite::http::HeaderValue::from_str(authentication.authorization()?.as_str())?;
    authorization.set_sensitive(true);
    request.headers_mut().insert("Authorization", authorization);
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        tungstenite::http::HeaderValue::from_static("jmap"),
    );
    let config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(128 * 1024)
        .max_message_size(Some(64 * 1024))
        .max_frame_size(Some(64 * 1024));
    let (mut socket, response) =
        match tungstenite::client_tls_with_config(request, stream, Some(config), None) {
            Ok(value) => value,
            Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
                authentication
                    .observe(reqwest::StatusCode::from_u16(response.status().as_u16())?)?;
                bail!("Push handshake rejected");
            }
            Err(_) => bail!("Push handshake failed"),
        };
    if response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|h| h.to_str().ok())
        != Some("jmap")
    {
        bail!("Push subprotocol was not negotiated");
    }
    let stream = match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::Rustls(stream) => &mut stream.sock,
        _ => bail!("Unsupported push TLS stream"),
    };
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    socket.send(Message::Text(
        json!({"@type":"WebSocketPushEnable","dataTypes":["FileNode"],"pushState":null})
            .to_string()
            .into(),
    ))?;
    signal.changed.store(true, Ordering::Relaxed);
    let mut last_message = Instant::now();
    let mut last_ping = Instant::now();
    while !signal.stop.load(Ordering::Relaxed) && policy.check().is_ok() {
        match socket.read() {
            Ok(Message::Text(text)) => {
                last_message = Instant::now();
                if state_changed(&text, account) {
                    signal.changed.store(true, Ordering::Relaxed);
                }
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                last_message = Instant::now();
                socket.flush()?;
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => bail!("Unexpected binary push message"),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        if last_message.elapsed() > Duration::from_secs(90) {
            break;
        }
        if last_ping.elapsed() >= Duration::from_secs(30) {
            socket.send(Message::Ping(Vec::new().into()))?;
            last_ping = Instant::now();
        }
    }
    // Dropping the socket also cancels a stale connection after power changes.
    Ok(())
}

fn state_changed(text: &str, account: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| {
        value["@type"] == "StateChange" && value["changed"][account]["FileNode"].is_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback's error type.
    fn actual_push_subscribes_and_reconnect_triggers_remote_reconciliation() {
        use std::{net::TcpListener, sync::mpsc};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = Url::parse(&format!("ws://{}/push", listener.local_addr().unwrap())).unwrap();
        let (ready, notifications) = mpsc::channel();
        let (command, commands) = mpsc::channel();
        let server = thread::spawn(move || {
            for index in 0..2 {
                let deadline = Instant::now() + Duration::from_secs(6);
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline);
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut socket = tungstenite::accept_hdr(
                    stream,
                    |request: &tungstenite::handshake::server::Request,
                     mut response: tungstenite::handshake::server::Response| {
                        assert_eq!(request.headers()["Sec-WebSocket-Protocol"], "jmap");
                        assert_eq!(
                            request.headers()["Authorization"],
                            "Basic dXNlcjpwYXNzd29yZA=="
                        );
                        response.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            tungstenite::http::HeaderValue::from_static("jmap"),
                        );
                        Ok(response)
                    },
                )
                .unwrap();
                let subscribe: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(subscribe["@type"], "WebSocketPushEnable");
                assert_eq!(subscribe["dataTypes"], json!(["FileNode"]));
                ready.send(index).unwrap();
                commands.recv_timeout(Duration::from_secs(3)).unwrap();
                socket.send(Message::Text(json!({"@type":"StateChange","changed":{"account":{"FileNode":format!("state-{index}")}}}).to_string().into())).unwrap();
                socket.close(None).unwrap();
            }
        });
        let policy = Policy::new(
            &crate::config::Config::default(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Default::default()),
        );
        // Plaintext is confined to this private loopback fixture; production
        // discovery rejects ws:// before any credentials are obtained.
        let push = Push::start(
            url,
            Authentication::basic("user", "password".into()),
            "account".into(),
            policy,
        );
        fn changed(push: &Push) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !push.take_change() {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(
            notifications.recv_timeout(Duration::from_secs(3)).unwrap(),
            0
        );
        changed(&push); // Initial reconciliation, before a notification exists.
        command.send(()).unwrap();
        changed(&push);
        assert_eq!(
            notifications.recv_timeout(Duration::from_secs(4)).unwrap(),
            1
        );
        changed(&push); // Reconnect must reconcile even without another change.
        command.send(()).unwrap();
        server.join().unwrap();
        drop(push);
    }

    #[test]
    fn power_pause_prevents_push_network_connections() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let power = Arc::new(crate::runtime::PowerState::default());
        power.set(true, false);
        let policy = Policy::new(
            &crate::config::Config::default(),
            Arc::new(AtomicBool::new(false)),
            power,
        );
        let push = Push::start(
            Url::parse(&format!("ws://{}/", listener.local_addr().unwrap())).unwrap(),
            Authentication::basic("user", "unused".into()),
            "account".into(),
            policy,
        );
        thread::sleep(Duration::from_millis(100));
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
        drop(push);
    }
    #[test]
    fn discovery_rejects_cross_origin_cleartext_and_unadvertised_push() {
        let origin = Url::parse("https://files.example.org").unwrap();
        assert!(endpoint(
            &origin,
            &json!({"url":"wss://files.example.org/ws","supportsPush":true})
        )
        .is_some());
        for url in [
            "ws://files.example.org/ws",
            "wss://other.example.org/ws",
            "wss://files.example.org:8443/ws",
            "wss://user:secret@files.example.org/ws",
            "wss://files.example.org/ws?token=secret",
        ] {
            assert!(endpoint(&origin, &json!({"url":url,"supportsPush":true})).is_none());
        }
        assert!(endpoint(
            &origin,
            &json!({"url":"wss://files.example.org/ws","supportsPush":false})
        )
        .is_none());
        assert!(state_changed(
            r#"{"@type":"StateChange","changed":{"account":{"FileNode":"v2"}}}"#,
            "account"
        ));
        assert!(!state_changed(
            r#"{"@type":"StateChange","changed":{"other":{"FileNode":"v2"}}}"#,
            "account"
        ));
        assert!(!state_changed("invalid JSON", "account"));
    }
}
