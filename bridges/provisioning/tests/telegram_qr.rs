//! Deterministic Telegram-QR vertical slice for the Bridge v2 provisioning
//! client, driven entirely against a fake Bridge v2 HTTP service (wiremock).
//! No matrix-sdk, no live provider, no Matrix credentials.

use std::collections::BTreeMap;

use serde_json::json;
use sideband_bridge_provisioning::{select_qr_flow, LoginSession, LoginUpdate, ProvisioningClient};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "internal-session-token";

fn client(server: &MockServer) -> ProvisioningClient {
    ProvisioningClient::new(server.uri(), TOKEN)
}

async fn mount_flows(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/_matrix/provision/v3/login/flows"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "flows": [
                {"id": "qr", "name": "QR", "description": "Scan a QR code"},
                {"id": "phone", "name": "Phone number", "description": "Phone + code"}
            ]
        })))
        .mount(server)
        .await;
}

async fn mount_start_qr(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/_matrix/provision/v3/login/start/qr"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login_id": "login-1",
            "type": "display_and_wait",
            "step_id": "qr-scan",
            "instructions": "Scan with Telegram",
            "display_and_wait": {"type": "qr", "data": "tg://login?token=abc"}
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn lists_flows_and_selects_qr() {
    let server = MockServer::start().await;
    mount_flows(&server).await;

    let flows = client(&server).list_flows().await.unwrap();
    assert_eq!(flows.len(), 2);
    assert_eq!(select_qr_flow(&flows).as_deref(), Some("qr"));
}

#[tokio::test]
async fn telegram_qr_scan_completes() {
    let server = MockServer::start().await;
    mount_flows(&server).await;
    mount_start_qr(&server).await;
    // The QR long-poll returns `complete` once the code is scanned.
    Mock::given(method("POST"))
        .and(path(
            "/_matrix/provision/v3/login/step/login-1/qr-scan/display_and_wait",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login_id": "login-1",
            "type": "complete",
            "step_id": "done",
            "complete": {"user_login_id": "42", "user_login_name": "Alice"}
        })))
        .mount(&server)
        .await;

    let (mut session, first) = LoginSession::begin(client(&server), select_qr_flow)
        .await
        .unwrap();
    assert_eq!(
        first,
        LoginUpdate::Qr {
            step_id: "qr-scan".into(),
            data: "tg://login?token=abc".into(),
            instructions: "Scan with Telegram".into(),
        }
    );
    assert!(session.is_display_and_wait());

    let done = session.wait().await.unwrap();
    assert_eq!(
        done,
        LoginUpdate::Success {
            name: "Alice".into()
        }
    );
}

#[tokio::test]
async fn telegram_qr_then_2fa_password_completes_without_leaking_secret() {
    let server = MockServer::start().await;
    mount_flows(&server).await;
    mount_start_qr(&server).await;
    // After scanning, Telegram asks for the 2FA password.
    Mock::given(method("POST"))
        .and(path(
            "/_matrix/provision/v3/login/step/login-1/qr-scan/display_and_wait",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login_id": "login-1",
            "type": "user_input",
            "step_id": "twofa",
            "instructions": "Enter your Telegram 2FA password",
            "user_input": {"fields": [
                {"type": "password", "id": "password", "name": "Password"}
            ]}
        })))
        .mount(&server)
        .await;
    // The password submit must carry exactly the field map, then complete.
    Mock::given(method("POST"))
        .and(path(
            "/_matrix/provision/v3/login/step/login-1/twofa/user_input",
        ))
        .and(body_json(json!({"password": "hunter2-2fa"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login_id": "login-1",
            "type": "complete",
            "step_id": "done",
            "complete": {"user_login_name": "Bob"}
        })))
        .mount(&server)
        .await;

    let (mut session, _qr) = LoginSession::begin(client(&server), select_qr_flow)
        .await
        .unwrap();

    let prompt = session.wait().await.unwrap();
    let LoginUpdate::Fields {
        step_type,
        fields,
        step_id,
        ..
    } = prompt
    else {
        panic!("expected a 2FA field prompt, got {prompt:?}");
    };
    assert_eq!(step_type, "user_input");
    assert_eq!(step_id, "twofa");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].id, "password");
    assert!(fields[0].secret, "password field must be marked secret");

    let mut values = BTreeMap::new();
    values.insert("password".to_string(), "hunter2-2fa".to_string());
    let done = session.submit(values).await.unwrap();
    assert_eq!(done, LoginUpdate::Success { name: "Bob".into() });

    // The terminal update must never echo the secret back to the UI layer.
    match done {
        LoginUpdate::Success { name } => assert!(!name.contains("hunter2")),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn missing_qr_flow_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_matrix/provision/v3/login/flows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "flows": [{"id": "phone", "name": "Phone number"}]
        })))
        .mount(&server)
        .await;

    let err = LoginSession::begin(client(&server), select_qr_flow)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no matching login flow"));
}

#[tokio::test]
async fn http_error_status_never_leaks_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_matrix/provision/v3/login/flows"))
        .respond_with(ResponseTemplate::new(500).set_body_string("secret-token-should-not-appear"))
        .mount(&server)
        .await;

    let err = client(&server).list_flows().await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("HTTP 500"));
    assert!(!text.contains("secret-token"));
}

#[tokio::test]
async fn unsupported_step_type_maps_to_error() {
    let server = MockServer::start().await;
    mount_flows(&server).await;
    Mock::given(method("POST"))
        .and(path("/_matrix/provision/v3/login/start/qr"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login_id": "login-1",
            "type": "webauthn",
            "step_id": "wa"
        })))
        .mount(&server)
        .await;

    let (_session, update) = LoginSession::begin(client(&server), select_qr_flow)
        .await
        .unwrap();
    assert_eq!(
        update,
        LoginUpdate::Error {
            message: "unsupported login step type: webauthn".into()
        }
    );
}
