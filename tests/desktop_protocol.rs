//! Account-free desktop worker contract: no listening sockets and no live server.
#![cfg(feature = "desktop")]
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[test]
fn worker_versions_protocol_validates_settings_and_applies_power_policy() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile.toml");
    // Removed options are not part of the public configuration contract.
    std::fs::write(
        &profile,
        "auto_connect = false\nremote_folder = \"Legacy folder\"\nusername = \"\"\n",
    )
    .unwrap();
    let executable = std::env::var_os("FILENODE_TEST_CORE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_filenode").into());
    let mut child = Command::new(executable)
        .env("FILENODE_LOG", "off")
        .args(["--service", "--external-autostart", "--config"])
        .arg(&profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for command in [
            json!({"command":"configure","config":{"upload_limit_kib":1_000_001}}),
            json!({"command":"configure","config":{"language":"de","upload_limit_kib":128}}),
            json!({"command":"power","available":true,"saver":true,"sleeping":false}),
            json!({"command":"power","available":true,"saver":false,"sleeping":true}),
            json!({"command":"configure","config":{"language":"de","exclude_patterns":["*.tmp","Cache/**"],"parallel_uploads":3,"max_file_bytes":0}}),
            json!({"command":"configure","config":{"exclude_patterns":["["]}}),
            json!({"command":"quit"}),
        ] {
            writeln!(stdin, "{command}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events[0]["event"], "ready");
    assert_eq!(events[0]["protocol"], 4);
    assert!(events[0]["config"].get("auto_connect").is_none());
    assert!(events[0]["config"].get("remote_folder").is_none());
    assert!(events[0]["config"].get("local_folder").is_none());
    assert_eq!(events[0]["config"]["sync_folders"], json!([]));
    assert_eq!(events[1]["event"], "error");
    assert_eq!(events[2]["event"], "configured");
    assert_eq!(events[2]["config"]["upload_limit_kib"], 128);
    assert_eq!(events[3]["paused"], true);
    assert_eq!(events[4]["paused"], true);
    assert_eq!(events[5]["event"], "configured");
    assert_eq!(events[6]["event"], "error");
    let saved = std::fs::read_to_string(profile).unwrap();
    assert!(saved.contains("language = \"de\""));
    let fields: toml::Value = toml::from_str(&saved).unwrap();
    assert!(fields.get("password").is_none());
    assert!(fields.get("access_token").is_none());
    assert!(fields.get("refresh_token").is_none());
    assert!(fields.get("auto_connect").is_none());
    assert!(fields.get("remote_folder").is_none());
    assert!(fields.get("local_folder").is_none());
    assert_eq!(fields["exclude_patterns"].as_array().unwrap().len(), 2);
    assert_eq!(fields["parallel_uploads"].as_integer(), Some(3));
    assert_eq!(fields["max_file_bytes"].as_integer(), Some(0));
}

#[test]
fn editing_running_sync_validates_then_restarts_and_preserves_other_edits() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile.toml");
    let executable = std::env::var_os("FILENODE_TEST_CORE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_filenode").into());
    let mut child = Command::new(executable)
        .env("FILENODE_LOG", "off")
        .args(["--service", "--external-autostart", "--config"])
        .arg(&profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let event: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    let mut stdin = child.stdin.take().unwrap();
    let mut send = |value: Value| writeln!(stdin, "{value}").unwrap();
    let receive = || {
        receiver
            .recv_timeout(Duration::from_secs(15))
            .expect("Worker event timed out")
    };
    assert_eq!(receive()["event"], "ready");
    // Refused loopback connection: exercises the real running/retry worker without
    // a live account, a credential-store write, or any local/remote file mutation.
    send(json!({"command":"configure", "patch":{
        "server":"https://127.0.0.1:9", "username":"fixture@example.org"
    }, "connect":true, "password":"local-test-fixture"}));
    loop {
        let event = receive();
        if event["event"] == "retry" {
            break;
        }
        assert_ne!(event["event"], "error", "{event}");
    }
    send(json!({"command":"configure", "patch":{"exclude_patterns":["["]}}));
    assert_eq!(receive()["event"], "error");
    send(json!({"command":"configure", "patch":{"parallel_uploads":3,"propagate_deletes":false}}));
    let mut stopped = false;
    loop {
        let event = receive();
        if event["event"] == "running" && event["value"] == false {
            stopped = true;
        }
        if event["event"] == "configured" {
            assert!(
                stopped,
                "Old worker must stop before edited settings are published"
            );
            assert_eq!(event["config"]["parallel_uploads"], 3);
            assert_eq!(event["config"]["propagate_deletes"], false);
            break;
        }
        assert_ne!(event["event"], "error", "{event}");
    }
    let event = receive();
    // Reconnection might fail before running if no password is stored. Supplying
    // a password must never be required just to persist already valid settings.
    assert!(
        event["event"] == "account" || event["event"] == "error",
        "{event}"
    );
    send(json!({"command":"pause"}));
    send(json!({"command":"configure", "patch":{"download_limit_kib":64}}));
    send(json!({"command":"configure", "patch":{"upload_limit_kib":128}}));
    let mut configured = 0;
    loop {
        let event = receive();
        if event["event"] == "configured" {
            configured += 1;
            if configured == 2 {
                assert_eq!(event["config"]["download_limit_kib"], 64);
                assert_eq!(event["config"]["upload_limit_kib"], 128);
                break;
            }
        }
    }
    send(json!({"command":"quit"}));
    drop(stdin);
    let result = child.wait_with_output().unwrap();
    reader.join().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
