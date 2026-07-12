//! Atomic JSON device/invite store — mirrors
//! `sgbr_core::bus_catalog::store`'s temp-file+rename approach. **Only token
//! hashes touch disk**; the raw bearer token is returned once, at
//! registration, and never persisted.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::auth::{to_hex, token_hash};

/// A persisted, registered device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRec {
    pub device_id: String,
    pub pubkey_b64: String,
    pub token_sha256: String,
    pub invite_code: String,
    pub created_at_unix: i64,
}

/// A persisted invite code, single-use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteRec {
    pub code: String,
    pub created_at_unix: i64,
    pub used_by: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoreData {
    devices: Vec<DeviceRec>,
    invites: Vec<InviteRec>,
}

/// Failures from store operations.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io failed: {0}")]
    Io(String),
    #[error("failed to (de)serialize store: {0}")]
    Serde(String),
    #[error("random generation failed: {0}")]
    Rng(String),
    #[error("unknown invite code")]
    UnknownInvite,
    #[error("invite already used")]
    InviteUsed,
}

/// Invite-code alphabet: uppercase letters and digits, minus the visually
/// confusable `I`, `L`, `O`, `0`, `1`.
const INVITE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const INVITE_LEN: usize = 8;

/// `Mutex`-guarded, disk-backed store of devices and invites.
#[derive(Debug)]
pub struct DeviceStore {
    data: Mutex<StoreData>,
    path: PathBuf,
}

impl DeviceStore {
    /// Load `path` if it exists, otherwise start with an empty store.
    ///
    /// # Errors
    /// [`StoreError::Io`] / [`StoreError::Serde`] if `path` exists but can't
    /// be read or parsed.
    pub fn load_or_default(path: PathBuf) -> Result<Self, StoreError> {
        let data = if path.exists() {
            let bytes = fs::read(&path).map_err(|e| StoreError::Io(e.to_string()))?;
            serde_json::from_slice(&bytes).map_err(|e| StoreError::Serde(e.to_string()))?
        } else {
            StoreData::default()
        };
        Ok(Self {
            data: Mutex::new(data),
            path,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreData> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn persist(&self, data: &StoreData) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let bytes = serde_json::to_vec(data).map_err(|e| StoreError::Serde(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &bytes).map_err(|e| StoreError::Io(e.to_string()))?;
        fs::rename(&tmp, &self.path).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    /// Mint a fresh invite code and persist it.
    ///
    /// # Errors
    /// [`StoreError::Rng`] if code generation fails; [`StoreError::Io`] /
    /// [`StoreError::Serde`] on persist failure.
    pub fn mint_invite(&self) -> Result<InviteRec, StoreError> {
        let rec = InviteRec {
            code: generate_invite_code()?,
            created_at_unix: now_unix(),
            used_by: None,
        };
        let mut data = self.lock();
        data.invites.push(rec.clone());
        self.persist(&data)?;
        Ok(rec)
    }

    /// All invites, used and unused.
    pub fn list_invites(&self) -> Vec<InviteRec> {
        self.lock().invites.clone()
    }

    /// All registered devices.
    pub fn list_devices(&self) -> Vec<DeviceRec> {
        self.lock().devices.clone()
    }

    /// Register a device against `invite_code`, marking the invite used and
    /// inserting the device in one persisted write.
    ///
    /// # Errors
    /// [`StoreError::UnknownInvite`] / [`StoreError::InviteUsed`] if the
    /// invite can't be consumed; [`StoreError::Rng`] on id/token generation
    /// failure; [`StoreError::Io`] / [`StoreError::Serde`] on persist failure.
    pub fn register_device(
        &self,
        invite_code: &str,
        pubkey_b64: &str,
    ) -> Result<(String, String), StoreError> {
        let device_id = random_hex(8)?;
        let token = random_hex(32)?;
        let token_sha256 = token_hash(&token);
        let mut data = self.lock();
        let invite = data
            .invites
            .iter_mut()
            .find(|i| i.code == invite_code)
            .ok_or(StoreError::UnknownInvite)?;
        if invite.used_by.is_some() {
            return Err(StoreError::InviteUsed);
        }
        invite.used_by = Some(device_id.clone());
        data.devices.push(DeviceRec {
            device_id: device_id.clone(),
            pubkey_b64: pubkey_b64.to_owned(),
            token_sha256,
            invite_code: invite_code.to_owned(),
            created_at_unix: now_unix(),
        });
        self.persist(&data)?;
        Ok((device_id, token))
    }

    /// Remove the device matching `device_id`, if any. `true` if a device was
    /// removed.
    ///
    /// # Errors
    /// [`StoreError::Io`] / [`StoreError::Serde`] on persist failure.
    pub fn revoke_device(&self, device_id: &str) -> Result<bool, StoreError> {
        let mut data = self.lock();
        let before = data.devices.len();
        data.devices.retain(|d| d.device_id != device_id);
        let changed = data.devices.len() != before;
        if changed {
            self.persist(&data)?;
        }
        Ok(changed)
    }

    /// Look up a device by its token's sha256 hex hash.
    pub fn device_by_token_hash(&self, hash: &str) -> Option<DeviceRec> {
        self.lock()
            .devices
            .iter()
            .find(|d| d.token_sha256 == hash)
            .cloned()
    }
}

fn now_unix() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

fn random_hex(n_bytes: usize) -> Result<String, StoreError> {
    let mut buf = vec![0_u8; n_bytes];
    getrandom::fill(&mut buf).map_err(|e| StoreError::Rng(e.to_string()))?;
    Ok(to_hex(&buf))
}

/// Generate an `INVITE_LEN`-char code from [`INVITE_ALPHABET`] using
/// rejection sampling, so every alphabet character is equally likely
/// regardless of whether the alphabet size divides 256.
fn generate_invite_code() -> Result<String, StoreError> {
    let alphabet_len = INVITE_ALPHABET.len();
    let cutoff = 256 - (256 % alphabet_len);
    let mut indices = Vec::with_capacity(INVITE_LEN);
    let mut buf = [0_u8; 1];
    while indices.len() < INVITE_LEN {
        getrandom::fill(&mut buf).map_err(|e| StoreError::Rng(e.to_string()))?;
        let b = usize::from(buf[0]);
        if b < cutoff {
            indices.push(b % alphabet_len);
        }
    }
    indices
        .into_iter()
        .map(|idx| {
            INVITE_ALPHABET
                .get(idx)
                .map(|&byte| byte as char)
                .ok_or_else(|| StoreError::Rng("alphabet index out of range".to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{DeviceStore, generate_invite_code};
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sgbr_worker_store_{}_{name}_{}.json",
            std::process::id(),
            name.len()
        ))
    }

    #[test]
    fn invite_code_matches_expected_alphabet_and_length() {
        for _ in 0..50 {
            let code = generate_invite_code().expect("generate");
            assert_eq!(code.len(), 8);
            assert!(
                code.bytes().all(|b| super::INVITE_ALPHABET.contains(&b)),
                "code {code} used a char outside the alphabet"
            );
        }
    }

    #[test]
    fn atomic_write_round_trips() {
        let path = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let invite = store.mint_invite().expect("mint");

        let reloaded = DeviceStore::load_or_default(path.clone()).expect("reload");
        let invites = reloaded.list_invites();
        assert_eq!(invites.len(), 1);
        assert_eq!(
            invites.first().map(|i| i.code.as_str()),
            Some(invite.code.as_str())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn register_device_marks_invite_used_and_persists_in_one_write() {
        let path = tmp_path("register");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let invite = store.mint_invite().expect("mint");

        let (device_id, token) = store
            .register_device(&invite.code, "pubkey-b64")
            .expect("register");
        assert_eq!(device_id.len(), 16);
        assert_eq!(token.len(), 64);

        let reloaded = DeviceStore::load_or_default(path.clone()).expect("reload");
        let invites = reloaded.list_invites();
        assert_eq!(
            invites.first().and_then(|i| i.used_by.clone()),
            Some(device_id.clone())
        );
        let devices = reloaded.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices.first().map(|d| d.device_id.as_str()),
            Some(device_id.as_str())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn register_device_rejects_unknown_invite() {
        let path = tmp_path("unknown");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let err = store
            .register_device("NOPE-CODE", "pubkey-b64")
            .expect_err("should reject");
        assert!(matches!(err, super::StoreError::UnknownInvite));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn register_device_rejects_reused_invite() {
        let path = tmp_path("reused");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let invite = store.mint_invite().expect("mint");
        store
            .register_device(&invite.code, "pubkey-b64")
            .expect("first registration");
        let err = store
            .register_device(&invite.code, "other-pubkey")
            .expect_err("should reject reuse");
        assert!(matches!(err, super::StoreError::InviteUsed));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn store_file_never_contains_raw_token() {
        let path = tmp_path("no-raw-token");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let invite = store.mint_invite().expect("mint");
        let (_device_id, token) = store
            .register_device(&invite.code, "pubkey-b64")
            .expect("register");

        let contents = std::fs::read_to_string(&path).expect("read store file");
        assert!(!contents.contains(&token), "raw token leaked to disk");
        assert!(contents.contains("token_sha256"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn revoke_device_removes_and_reports_change() {
        let path = tmp_path("revoke");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let invite = store.mint_invite().expect("mint");
        let (device_id, _token) = store
            .register_device(&invite.code, "pubkey-b64")
            .expect("register");

        assert!(store.revoke_device(&device_id).expect("revoke"));
        assert!(store.list_devices().is_empty());
        assert!(!store.revoke_device(&device_id).expect("revoke again"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn device_by_token_hash_finds_registered_device() {
        let path = tmp_path("lookup");
        let _ = std::fs::remove_file(&path);
        let store = DeviceStore::load_or_default(path.clone()).expect("load empty");
        let invite = store.mint_invite().expect("mint");
        let (device_id, token) = store
            .register_device(&invite.code, "pubkey-b64")
            .expect("register");

        let hash = super::token_hash(&token);
        let found = store.device_by_token_hash(&hash).expect("found");
        assert_eq!(found.device_id, device_id);
        assert!(store.device_by_token_hash("not-a-hash").is_none());
        let _ = std::fs::remove_file(&path);
    }
}
