//! Typed client for the mautrix **Bridge v2 provisioning v3** login API.
//!
//! Sideband never parses management-bot prose. Instead it drives the bridge's
//! machine-readable login flow: list flows, start one, then submit typed steps
//! (`display_and_wait` for QR/code, `user_input` for fields like a 2FA password,
//! `complete` when the provider account is linked). This crate is intentionally
//! free of matrix-sdk so the state machine is fast + deterministic to test.
//!
//! Routes (all under the homeserver base URL, bearer-authenticated with the
//! internal Matrix session's access token):
//!   * `GET  /_matrix/provision/v3/login/flows`
//!   * `POST /_matrix/provision/v3/login/start/{flowID}`
//!   * `POST /_matrix/provision/v3/login/step/{loginID}/{stepID}/{stepType}`

use std::collections::BTreeMap;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

mod session;
pub use session::{acquire_internal_session, InternalSessionRequest, MatrixCredentials};

/// Path prefix for the Bridge v2 provisioning v3 login API.
pub const PROVISION_LOGIN_BASE: &str = "/_matrix/provision/v3/login";

/// A login flow advertised by a bridge (e.g. Telegram's "QR" or "Phone number").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LoginFlow {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct FlowsResponse {
    #[serde(default)]
    flows: Vec<LoginFlow>,
}

/// A single input field within a `user_input` or `cookies` step.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawField {
    #[serde(rename = "type", default)]
    pub field_type: String,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub pattern: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DisplayAndWait {
    #[serde(rename = "type", default)]
    display_type: String,
    #[serde(default)]
    data: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UserInput {
    #[serde(default)]
    fields: Vec<RawField>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CompleteInfo {
    #[serde(default)]
    user_login_name: String,
}

/// The raw step object the bridge returns from `start` and `step`.
#[derive(Debug, Clone, Deserialize)]
struct RawStep {
    #[serde(default)]
    login_id: String,
    #[serde(rename = "type", default)]
    step_type: String,
    #[serde(default)]
    step_id: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    display_and_wait: Option<DisplayAndWait>,
    #[serde(default)]
    user_input: Option<UserInput>,
    #[serde(default)]
    complete: Option<CompleteInfo>,
}

/// A UI-facing field Sideband renders and collects a value for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginField {
    pub id: String,
    pub label: String,
    /// Provider field type (`phone_number`, `password`, `2fa_code`, `token`, …).
    pub field_type: String,
    pub description: String,
    pub pattern: String,
    /// A password/2FA/token field whose value must never be displayed or logged.
    pub secret: bool,
}

/// What the caller should show the user next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginUpdate {
    /// Render a QR the user scans with the provider app.
    Qr {
        step_id: String,
        data: String,
        instructions: String,
    },
    /// Show a short code/emoji the user confirms elsewhere.
    Code {
        step_id: String,
        code: String,
        instructions: String,
    },
    /// Collect typed fields (e.g. phone number, 2FA password) and submit them.
    Fields {
        step_id: String,
        step_type: String,
        fields: Vec<LoginField>,
        instructions: String,
    },
    /// The provider account is linked.
    Success { name: String },
    /// The bridge reported an unrecoverable login error.
    Error { message: String },
}

fn field_is_secret(field_type: &str) -> bool {
    matches!(
        field_type,
        "password" | "2fa_code" | "token" | "secret" | "cookie"
    )
}

/// A thin HTTP client for one bridge's provisioning API, bearer-authenticated
/// with the internal Matrix session's access token.
#[derive(Debug, Clone)]
pub struct ProvisioningClient {
    base_url: String,
    access_token: String,
    http: reqwest::Client,
}

impl ProvisioningClient {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self::with_http(base_url, access_token, reqwest::Client::new())
    }

    pub fn with_http(
        base_url: impl Into<String>,
        access_token: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            base_url,
            access_token: access_token.into(),
            http,
        }
    }

    /// List the login flows the bridge advertises.
    pub async fn list_flows(&self) -> Result<Vec<LoginFlow>> {
        let url = format!("{}{PROVISION_LOGIN_BASE}/flows", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .context("request login flows")?;
        let resp = ensure_success(resp).await?;
        let flows: FlowsResponse = resp.json().await.context("decode login flows")?;
        Ok(flows.flows)
    }

    async fn start(&self, flow_id: &str) -> Result<RawStep> {
        let url = format!("{}{PROVISION_LOGIN_BASE}/start/{flow_id}", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("start login flow")?;
        let resp = ensure_success(resp).await?;
        resp.json().await.context("decode login start step")
    }

    async fn submit_step(
        &self,
        login_id: &str,
        step_id: &str,
        step_type: &str,
        body: serde_json::Value,
    ) -> Result<RawStep> {
        let url = format!(
            "{}{PROVISION_LOGIN_BASE}/step/{login_id}/{step_id}/{step_type}",
            self.base_url
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await
            .context("submit login step")?;
        let resp = ensure_success(resp).await?;
        resp.json().await.context("decode login step")
    }
}

async fn ensure_success(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    // Do not surface response bodies verbatim — they can echo tokens/cookies.
    Err(anyhow!("bridge provisioning returned HTTP {}", status))
}

/// An in-progress login against one bridge flow. Holds the current step and the
/// login-process id needed to submit the next one.
#[derive(Debug)]
pub struct LoginSession {
    client: ProvisioningClient,
    login_id: String,
    current: RawStep,
}

impl LoginSession {
    /// Begin a login: list flows, choose one with `select`, and start it.
    /// `select` receives the advertised flows and returns the chosen flow id.
    pub async fn begin<F>(client: ProvisioningClient, select: F) -> Result<(Self, LoginUpdate)>
    where
        F: FnOnce(&[LoginFlow]) -> Option<String>,
    {
        let flows = client.list_flows().await?;
        let flow_id = select(&flows).ok_or_else(|| anyhow!("no matching login flow"))?;
        let step = client.start(&flow_id).await?;
        let login_id = step.login_id.clone();
        if login_id.is_empty() {
            return Err(anyhow!("bridge did not return a login id"));
        }
        let session = Self {
            client,
            login_id,
            current: step,
        };
        let update = session.current_update();
        Ok((session, update))
    }

    /// Map the current step to a UI update without any network round-trip.
    pub fn current_update(&self) -> LoginUpdate {
        map_step(&self.current)
    }

    /// Whether the current step is a `display_and_wait` (QR/code) that the
    /// caller advances by long-polling [`Self::wait`].
    pub fn is_display_and_wait(&self) -> bool {
        self.current.step_type == "display_and_wait"
    }

    /// Long-poll a `display_and_wait` step (empty submit) until the bridge
    /// advances to the next step (e.g. the QR was scanned).
    pub async fn wait(&mut self) -> Result<LoginUpdate> {
        if !self.is_display_and_wait() {
            return Err(anyhow!("current step is not display_and_wait"));
        }
        let step_id = self.current.step_id.clone();
        let next = self
            .client
            .submit_step(
                &self.login_id,
                &step_id,
                "display_and_wait",
                serde_json::json!({}),
            )
            .await?;
        self.advance(next)
    }

    /// Submit typed field values for a `user_input`/`cookies` step.
    pub async fn submit(&mut self, values: BTreeMap<String, String>) -> Result<LoginUpdate> {
        let step_type = self.current.step_type.clone();
        if step_type != "user_input" && step_type != "cookies" {
            return Err(anyhow!("current step does not accept field input"));
        }
        let step_id = self.current.step_id.clone();
        let body = serde_json::to_value(&values).unwrap_or_else(|_| serde_json::json!({}));
        let next = self
            .client
            .submit_step(&self.login_id, &step_id, &step_type, body)
            .await?;
        self.advance(next)
    }

    fn advance(&mut self, mut next: RawStep) -> Result<LoginUpdate> {
        if next.login_id.is_empty() {
            next.login_id = self.login_id.clone();
        }
        self.login_id = next.login_id.clone();
        self.current = next;
        Ok(self.current_update())
    }
}

fn map_step(step: &RawStep) -> LoginUpdate {
    match step.step_type.as_str() {
        "display_and_wait" => {
            let dw = step.display_and_wait.clone().unwrap_or_default();
            match dw.display_type.as_str() {
                "qr" => LoginUpdate::Qr {
                    step_id: step.step_id.clone(),
                    data: dw.data,
                    instructions: step.instructions.clone(),
                },
                "code" | "emoji" => LoginUpdate::Code {
                    step_id: step.step_id.clone(),
                    code: dw.data,
                    instructions: step.instructions.clone(),
                },
                _ => LoginUpdate::Code {
                    step_id: step.step_id.clone(),
                    code: String::new(),
                    instructions: step.instructions.clone(),
                },
            }
        }
        "user_input" | "cookies" => {
            let ui = step.user_input.clone().unwrap_or_default();
            let fields = ui
                .fields
                .into_iter()
                .map(|f| LoginField {
                    label: if f.name.is_empty() {
                        f.id.clone()
                    } else {
                        f.name
                    },
                    secret: field_is_secret(&f.field_type),
                    id: f.id,
                    field_type: f.field_type,
                    description: f.description,
                    pattern: f.pattern,
                })
                .collect();
            LoginUpdate::Fields {
                step_id: step.step_id.clone(),
                step_type: step.step_type.clone(),
                fields,
                instructions: step.instructions.clone(),
            }
        }
        "complete" => LoginUpdate::Success {
            name: step
                .complete
                .clone()
                .map(|c| c.user_login_name)
                .unwrap_or_default(),
        },
        other => LoginUpdate::Error {
            message: format!("unsupported login step type: {other}"),
        },
    }
}

/// Choose a QR flow from advertised flows (prefers ids/names mentioning "qr").
pub fn select_qr_flow(flows: &[LoginFlow]) -> Option<String> {
    flows
        .iter()
        .find(|f| {
            f.id.to_ascii_lowercase().contains("qr") || f.name.to_ascii_lowercase().contains("qr")
        })
        .map(|f| f.id.clone())
}
