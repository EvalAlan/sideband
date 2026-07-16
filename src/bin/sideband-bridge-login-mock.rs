//! Deterministic interactive-login connector used by the bridge integration test.
//! Not bundled with any client.

use std::io::{BufRead, Write};

fn emit(value: serde_json::Value) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{value}").expect("write mock connector output");
    handle.flush().expect("flush mock connector output");
}

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("read mock connector input");
        let msg: serde_json::Value = serde_json::from_str(&line).expect("parse core message");
        match msg.get("type").and_then(|value| value.as_str()) {
            Some("hello") => {
                assert_eq!(
                    msg.get("protocol").and_then(|value| value.as_u64()),
                    Some(2)
                );
                assert!(
                    msg.get("config")
                        .and_then(|value| value.get("homeserver"))
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| !value.is_empty()),
                    "core must supply the profile Matrix homeserver"
                );
                emit(serde_json::json!({"type": "ready"}));
                emit(serde_json::json!({"type": "status", "state": "login_required"}));
            }
            Some("login") => {
                let input_id = msg
                    .get("input_id")
                    .and_then(|value| value.as_i64())
                    .unwrap();
                emit(serde_json::json!({"type": "login_input_ack", "input_id": input_id}));
                emit(serde_json::json!({
                    "type": "login_prompt",
                    "step_id": "scan-1",
                    "kind": "qr",
                    "prompt": "Scan with the provider app",
                    "qr": "mock-login-payload"
                }));
            }
            Some("login_input") => {
                let input_id = msg
                    .get("input_id")
                    .and_then(|value| value.as_i64())
                    .unwrap();
                emit(serde_json::json!({"type": "login_input_ack", "input_id": input_id}));
                let step = msg.get("step_id").and_then(|value| value.as_str());
                let value = msg.get("value").and_then(|value| value.as_str());
                if step == Some("scan-1") && value == Some("scanned") {
                    emit(serde_json::json!({
                        "type": "login_prompt",
                        "step_id": "authorize-2",
                        "kind": "url",
                        "prompt": "Authorize in a browser",
                        "url": "https://example.invalid/login"
                    }));
                } else if step == Some("authorize-2") && value == Some("opened") {
                    emit(serde_json::json!({
                        "type": "login_prompt",
                        "step_id": "code-3",
                        "kind": "text_input",
                        "prompt": "Enter the login code"
                    }));
                } else if step == Some("code-3") && value == Some("123456") {
                    emit(serde_json::json!({
                        "type": "login_prompt",
                        "step_id": "password-4",
                        "kind": "password_input",
                        "prompt": "Enter the account password"
                    }));
                } else if step == Some("password-4") && value == Some("secret") {
                    emit(serde_json::json!({
                        "type": "login_prompt",
                        "step_id": "done",
                        "kind": "success",
                        "prompt": "Connected"
                    }));
                } else {
                    emit(serde_json::json!({
                        "type": "login_prompt",
                        "step_id": "failed",
                        "kind": "error",
                        "prompt": "Unexpected login input"
                    }));
                }
            }
            Some("shutdown") => break,
            _ => {}
        }
    }
}
