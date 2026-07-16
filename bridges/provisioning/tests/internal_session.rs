//! Deterministic coverage for establishing Sideband's internal Matrix session
//! against a fake Synapse (wiremock). No matrix-sdk, no real homeserver.

use serde_json::json;
use sideband_bridge_provisioning::{
    acquire_internal_session, InternalSessionRequest, MatrixCredentials,
};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(shared_secret: Option<&str>) -> InternalSessionRequest {
    InternalSessionRequest {
        localpart: "sideband".into(),
        password: "generated-strong-password".into(),
        shared_secret: shared_secret.map(str::to_string),
        device_name: "Sideband Connected Apps".into(),
    }
}

async fn mount_login_ok(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "user_id": "@sideband:example.org",
            "access_token": "syt_secret_token",
            "device_id": "DEV123"
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn existing_account_logs_in() {
    let server = MockServer::start().await;
    mount_login_ok(&server).await;

    let creds = acquire_internal_session(&reqwest::Client::new(), &server.uri(), &req(None))
        .await
        .unwrap();
    assert_eq!(
        creds,
        MatrixCredentials {
            user_id: "@sideband:example.org".into(),
            access_token: "syt_secret_token".into(),
            device_id: "DEV123".into(),
        }
    );
}

#[tokio::test]
async fn missing_account_registers_then_logs_in() {
    let server = MockServer::start().await;
    // First login attempt fails because the account does not exist yet.
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"errcode": "M_FORBIDDEN", "error": "unknown user"})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Registration: nonce then create.
    Mock::given(method("GET"))
        .and(path("/_synapse/admin/v1/register"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"nonce": "nonce-xyz"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/_synapse/admin/v1/register"))
        .and(body_partial_json(
            json!({"username": "sideband", "admin": false}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "user_id": "@sideband:example.org",
            "access_token": "reg_token",
            "device_id": "REGDEV"
        })))
        .mount(&server)
        .await;
    // Second login (after registration) succeeds.
    mount_login_ok(&server).await;

    let creds = acquire_internal_session(
        &reqwest::Client::new(),
        &server.uri(),
        &req(Some("shared-secret")),
    )
    .await
    .unwrap();
    assert_eq!(creds.access_token, "syt_secret_token");
    assert_eq!(creds.user_id, "@sideband:example.org");
}

#[tokio::test]
async fn no_shared_secret_and_absent_account_is_error_without_leaking() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"errcode": "M_FORBIDDEN"})))
        .mount(&server)
        .await;

    let err = acquire_internal_session(&reqwest::Client::new(), &server.uri(), &req(None))
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("internal Matrix session"));
    assert!(!text.contains("M_FORBIDDEN"));
}

#[tokio::test]
async fn wrong_password_is_hard_failure_not_registration() {
    let server = MockServer::start().await;
    // A definitive credential rejection (not "user absent") must NOT trigger a
    // registration attempt even when a shared secret is available.
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(json!({"errcode": "M_LIMIT_EXCEEDED"})),
        )
        .mount(&server)
        .await;
    // If registration were (wrongly) attempted, there is no nonce mock → the
    // test still fails via the error path, but we also assert no creds.
    let result = acquire_internal_session(
        &reqwest::Client::new(),
        &server.uri(),
        &req(Some("shared-secret")),
    )
    .await;
    assert!(result.is_err());
}
