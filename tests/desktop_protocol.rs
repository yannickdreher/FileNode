//! Account-free desktop worker contract: no listening sockets and no live server.
#![cfg(feature = "desktop")]
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
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
    let saved = std::fs::read_to_string(profile).unwrap();
    assert!(saved.contains("language = \"de\""));
    let fields: toml::Value = toml::from_str(&saved).unwrap();
    assert!(fields.get("password").is_none());
    assert!(fields.get("access_token").is_none());
    assert!(fields.get("refresh_token").is_none());
    assert!(fields.get("auto_connect").is_none());
    assert!(fields.get("remote_folder").is_none());
    assert!(fields.get("local_folder").is_none());
}
