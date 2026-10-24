use std::process::Command;

#[test]
fn relay_startup_redacts_the_key_and_writes_a_log_file() {
    let logs = tempfile::tempdir().unwrap();
    let key = "startup-redaction-fixture";
    let output = Command::new(env!("CARGO_BIN_EXE_hbbr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--key", key, "--port", "invalid"])
        .env("LOG_DIR", logs.path()).env("ALLOW_ANONYMOUS", "N").env("RUST_LOG", "info")
        .output().unwrap();
    assert!(!output.status.success());
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let mut count = 0;
    for entry in std::fs::read_dir(logs.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "log") {
            count += 1;
            text.push_str(&std::fs::read_to_string(path).unwrap());
        }
    }
    assert!(count > 0, "no service log file was created");
    assert!(text.contains("Access key configured (value redacted)"), "startup did not reach the key configuration path");
    assert!(!text.contains(key), "a configured key was written to logs");
}

#[test]
fn relay_startup_rejects_an_empty_key_by_default() {
    let logs = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hbbr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--key", "", "--port", "invalid"])
        .env("LOG_DIR", logs.path()).env("ALLOW_ANONYMOUS", "N").env("RUST_LOG", "info")
        .output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("KEY is required"));
}

#[test]
fn relay_startup_falls_back_to_stdout_when_log_dir_is_unwritable() {
    // A regular file cannot be used as the log directory.
    let not_a_dir = tempfile::NamedTempFile::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hbbr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--key", "startup-fallback-fixture", "--port", "invalid"])
        .env("LOG_DIR", not_a_dir.path()).env("ALLOW_ANONYMOUS", "N").env("RUST_LOG", "info")
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("using stdout only"), "missing fallback warning: {stdout}");
    assert!(stdout.contains("Access key configured (value redacted)"), "startup stopped at logging: {stdout}");
}

#[test]
fn relay_startup_rejects_invalid_trusted_proxies() {
    let logs = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hbbr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--key", "startup-proxy-fixture", "--port", "invalid"])
        .env("LOG_DIR", logs.path()).env("ALLOW_ANONYMOUS", "N").env("TRUSTED_PROXIES", "127.0.0.1/32,not-a-network")
        .output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid TRUSTED_PROXIES"));
}
