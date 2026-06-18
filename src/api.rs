// FFI-friendly API wrappers — take &str, convert to &Path internally
// Function names are prefixed with api_ to avoid shadowing crate-internal functions
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::transport::tor::TorTransport;
use crate::types::{ApiContact, ApiMessage, ApiStatus};

fn expand_profile(profile_path: &str) -> PathBuf {
    crate::expand_home(Path::new(profile_path))
}

pub fn api_init_profile(profile_path: &str, display_name: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    if crate::identity_path(&profile).exists() {
        return Ok(());
    }
    let name = display_name.trim();
    let chosen = if name.is_empty() {
        crate::default_display_name(&profile)
    } else {
        name.to_string()
    };
    crate::init_profile_with_name(&profile, &chosen)
}

pub fn api_list_contacts(profile_path: &str) -> Result<Vec<ApiContact>> {
    let profile = expand_profile(profile_path);
    let contacts = crate::load_contacts(&profile)?;
    let mut out: Vec<ApiContact> = contacts
        .values()
        .map(|c| ApiContact {
            name: c.name.clone(),
            onion: c.onion.clone(),
            ed25519_pubkey_b64: c.pubkey_b64.clone(),
            x25519_pubkey_b64: c.x25519_pubkey_b64.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn api_add_contact(profile_path: &str, contact: ApiContact) -> Result<()> {
    let profile = expand_profile(profile_path);
    let x25519 = contact
        .x25519_pubkey_b64
        .ok_or_else(|| anyhow!("x25519 pubkey is required"))?;
    crate::contact_add(
        &profile,
        &contact.name,
        &contact.onion,
        &contact.ed25519_pubkey_b64,
        &x25519,
    )
}

pub fn api_delete_contact(profile_path: &str, name: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::contact_delete(&profile, name)
}

pub async fn api_send_message(profile_path: &str, to: &str, body: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    let onion = crate::resolve_to(&profile, to)?;
    let tor_client = TorTransport::bootstrap(&profile).await?;
    crate::send(&profile, &onion, body, to, None, tor_client, false).await
}

pub async fn api_send_file(profile_path: &str, to: &str, file_path: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    let tor_client = TorTransport::bootstrap(&profile).await?;
    crate::send_file(&profile, to, file_path, None, tor_client).await
}

pub fn api_list_messages(
    profile_path: &str,
    contact: Option<&str>,
    limit: usize,
) -> Result<Vec<ApiMessage>> {
    let profile = expand_profile(profile_path);
    let rows = crate::load_history(&profile, contact, limit)?;
    Ok(rows
        .into_iter()
        .map(|r| ApiMessage {
            id: r.id,
            direction: r.direction,
            contact: r.contact,
            onion: r.onion,
            body: r.body,
            timestamp_ms: r.timestamp_ms,
            status: match r.status {
                2 => "failed".to_string(),
                1 => "delivered".to_string(),
                _ => "sent".to_string(),
            },
            created_at: r.created_at,
        })
        .collect())
}

pub fn api_list_transfers(profile_path: &str) -> Result<Vec<String>> {
    let profile = expand_profile(profile_path);
    crate::list_transfers(&profile)
}

pub async fn api_resume_transfer(profile_path: &str, hash: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    let Some((contact, file_path)) = crate::outbound_transfer_target(&profile, hash)? else {
        return Ok(false);
    };
    let tor_client = TorTransport::bootstrap(&profile).await?;
    crate::send_file(
        &profile,
        &contact,
        &file_path,
        None,
        Arc::clone(&tor_client),
    )
    .await?;
    Ok(true)
}

pub fn api_cancel_transfer(profile_path: &str, hash: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::cancel_outbound_transfer(&profile, hash)
}

pub fn api_status(profile_path: &str) -> Result<ApiStatus> {
    let profile = expand_profile(profile_path);
    let contacts = crate::load_contacts(&profile)?;
    let transfers = crate::list_transfers(&profile)?;
    Ok(ApiStatus {
        profile: profile.display().to_string(),
        display_name: crate::load_display_name(&profile)?,
        contact_count: contacts.len(),
        transfer_count: transfers.len(),
    })
}
