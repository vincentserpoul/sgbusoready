//! Persisted worker device credentials — atomic JSON cache (see
//! `crate::json_file`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::json_file;

/// Device identity + bearer token issued by the worker at registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCredentials {
    pub token: String,
    pub device_id: String,
    pub secret_seed_b64: String,
    pub pubkey_b64: String,
}

/// Load credentials from `path`. A missing or invalid file is an error; the
/// caller then re-registers.
///
/// # Errors
/// [`CoreError::Io`] if the file can't be read, [`CoreError::Parse`] if it
/// isn't valid JSON.
pub fn load(path: &Path) -> Result<WorkerCredentials, CoreError> {
    json_file::load(path)
}

impl WorkerCredentials {
    /// Save `self` to `path` atomically (temp file + rename), creating parent
    /// directories as needed.
    ///
    /// # Errors
    /// [`CoreError::Io`] on any filesystem failure, [`CoreError::Parse`] if
    /// serialization somehow fails.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        json_file::save(self, path)
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerCredentials, load};
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sgbr_worker_creds_{}_{name}.json",
            std::process::id()
        ))
    }

    fn sample() -> WorkerCredentials {
        WorkerCredentials {
            token: "a".repeat(64),
            device_id: "device-1".to_owned(),
            secret_seed_b64: "c2VlZA==".to_owned(),
            pubkey_b64: "cHVia2V5".to_owned(),
        }
    }

    #[test]
    fn round_trips() {
        let path = tmp_path("round");
        let creds = sample();
        creds.save(&path).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded, creds);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_is_io_error() {
        let path = tmp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert!(matches!(load(&path), Err(crate::error::CoreError::Io(_))));
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let mut path = tmp_path("nested");
        path.pop();
        path.push("deeper");
        path.push("creds.json");
        let creds = sample();
        creds.save(&path).expect("save into new dir");
        assert!(path.exists());
    }
}
