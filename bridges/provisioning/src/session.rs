//! Acquiring Sideband's **internal Matrix session** with the backend homeserver.
//!
//! Connected Apps needs a Matrix account (`@sideband:<server>`) to sync bridged
//! rooms and to bearer-authenticate the bridge provisioning API. The user never
//! sees or supplies any of this: the app owns a generated password (and, for a
//! self-hosted dev stack, the Synapse shared registration secret), and this
//! module turns them into a session by **registering** the account the first
//! time (Synapse admin shared-secret registration) and **logging in** on every
//! subsequent run. Both are plain Matrix/Synapse HTTP, so they are exercised
//! deterministically against a fake homeserver without matrix-sdk.

use anyhow::{anyhow, Context, Result};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha1::Sha1;

/// The internal Matrix session Sideband uses to reach the backend + bridges.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MatrixCredentials {
    pub user_id: String,
    pub access_token: String,
    #[serde(default)]
    pub device_id: String,
}

/// App-owned inputs for establishing the internal session. Never surfaced to the
/// user; `password`/`shared_secret` are secrets and must not be logged.
#[derive(Debug, Clone)]
pub struct InternalSessionRequest {
    /// Matrix localpart for the internal account (e.g. `sideband`).
    pub localpart: String,
    /// App-generated password for the internal account.
    pub password: String,
    /// Synapse `registration_shared_secret`, when the app manages a local dev
    /// stack and may need to create the account. `None` = login only.
    pub shared_secret: Option<String>,
    /// `initial_device_display_name` for the login/registration.
    pub device_name: String,
}

#[derive(Debug, Deserialize)]
struct NonceResponse {
    nonce: String,
}

#[derive(Debug, Deserialize)]
struct MatrixError {
    #[serde(default)]
    errcode: String,
}

/// Compute the Synapse admin-register MAC: HMAC-SHA1 over
/// `nonce \0 user \0 password \0 (admin|notadmin)`, hex-encoded.
fn register_mac(
    shared_secret: &str,
    nonce: &str,
    user: &str,
    password: &str,
    admin: bool,
) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(shared_secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(nonce.as_bytes());
    mac.update(b"\x00");
    mac.update(user.as_bytes());
    mac.update(b"\x00");
    mac.update(password.as_bytes());
    mac.update(b"\x00");
    mac.update(if admin { b"admin" } else { b"notadmin" });
    hex::encode(mac.finalize().into_bytes())
}

/// Establish the internal Matrix session against `homeserver`.
///
/// Tries a password login first; if the account does not exist yet and a shared
/// secret is available, registers it (Synapse admin shared-secret flow) and logs
/// in. Returns the resulting [`MatrixCredentials`]. Error strings never include
/// response bodies (which can carry tokens).
pub async fn acquire_internal_session(
    http: &reqwest::Client,
    homeserver: &str,
    req: &InternalSessionRequest,
) -> Result<MatrixCredentials> {
    let base = homeserver.trim_end_matches('/');
    match login(http, base, req).await {
        Ok(creds) => Ok(creds),
        Err(LoginError::Unknown) if req.shared_secret.is_some() => {
            register(http, base, req).await?;
            login(http, base, req)
                .await
                .map_err(|_| anyhow!("internal Matrix login failed after registration"))
        }
        Err(_) => Err(anyhow!(
            "Sideband could not establish its internal Matrix session"
        )),
    }
}

enum LoginError {
    /// Credentials rejected because the account does not exist (registerable).
    Unknown,
    /// Any other failure (network, bad password, server error).
    Other,
}

async fn login(
    http: &reqwest::Client,
    base: &str,
    req: &InternalSessionRequest,
) -> std::result::Result<MatrixCredentials, LoginError> {
    let url = format!("{base}/_matrix/client/v3/login");
    let body = serde_json::json!({
        "type": "m.login.password",
        "identifier": {"type": "m.id.user", "user": req.localpart},
        "password": req.password,
        "initial_device_display_name": req.device_name,
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|_| LoginError::Other)?;
    if resp.status().is_success() {
        return resp
            .json::<MatrixCredentials>()
            .await
            .map_err(|_| LoginError::Other);
    }
    // M_FORBIDDEN / M_USER_NOT_FOUND / M_UNKNOWN → the account is (likely)
    // absent and can be registered; other statuses are hard failures.
    let errcode = resp
        .json::<MatrixError>()
        .await
        .map(|e| e.errcode)
        .unwrap_or_default();
    if matches!(
        errcode.as_str(),
        "M_FORBIDDEN" | "M_USER_NOT_FOUND" | "M_UNKNOWN" | ""
    ) {
        Err(LoginError::Unknown)
    } else {
        Err(LoginError::Other)
    }
}

async fn register(http: &reqwest::Client, base: &str, req: &InternalSessionRequest) -> Result<()> {
    let shared_secret = req
        .shared_secret
        .as_deref()
        .ok_or_else(|| anyhow!("no registration shared secret configured"))?;
    let url = format!("{base}/_synapse/admin/v1/register");
    let nonce: NonceResponse = http
        .get(&url)
        .send()
        .await
        .context("request registration nonce")?
        .json()
        .await
        .context("decode registration nonce")?;
    let mac = register_mac(
        shared_secret,
        &nonce.nonce,
        &req.localpart,
        &req.password,
        false,
    );
    let body = serde_json::json!({
        "nonce": nonce.nonce,
        "username": req.localpart,
        "password": req.password,
        "admin": false,
        "mac": mac,
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("register internal account")?;
    if resp.status().is_success() {
        return Ok(());
    }
    let errcode = resp
        .json::<MatrixError>()
        .await
        .map(|e| e.errcode)
        .unwrap_or_default();
    // A concurrent run may have created it; treat "already exists" as success.
    if errcode == "M_USER_IN_USE" {
        Ok(())
    } else {
        Err(anyhow!("internal Matrix registration failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_mac_matches_synapse_reference() {
        // Independently HMAC-SHA1("shhh", "nonce123\0sideband\0pw\0notadmin").
        let mac = register_mac("shhh", "nonce123", "sideband", "pw", false);
        let mut expected = Hmac::<Sha1>::new_from_slice(b"shhh").unwrap();
        expected.update(b"nonce123\x00sideband\x00pw\x00notadmin");
        assert_eq!(mac, hex::encode(expected.finalize().into_bytes()));
        assert_eq!(mac.len(), 40); // SHA1 = 20 bytes hex-encoded
    }
}
