//! Crash-safe JSON files: the one load/save idiom shared by every persisted
//! store (bus catalog, commute list, worker credentials, and the worker's
//! device store). Saves go to a sibling temp file first and are renamed into
//! place, so a crash mid-write can never leave a half-written file behind.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::CoreError;

/// Read and parse `path`.
///
/// # Errors
/// [`CoreError::Io`] if the file can't be read (including when it is
/// missing), [`CoreError::Parse`] if it isn't valid JSON for `T`.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T, CoreError> {
    let bytes = fs::read(path).map_err(|e| CoreError::Io(e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|e| CoreError::Parse(e.to_string()))
}

/// Serialize `value` (compact JSON) and write it atomically to `path`.
///
/// # Errors
/// [`CoreError::Parse`] if serialization fails, [`CoreError::Io`] on any
/// filesystem failure.
pub fn save<T: Serialize>(value: &T, path: &Path) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec(value).map_err(|e| CoreError::Parse(e.to_string()))?;
    write_atomic(path, &bytes)
}

/// Write `bytes` to `path` atomically (temp file + rename), creating parent
/// directories as needed. For callers that already hold the serialized form
/// (e.g. pretty-printed JSON).
///
/// # Errors
/// [`CoreError::Io`] on any filesystem failure.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::Io(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|e| CoreError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| CoreError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{load, save, write_atomic};
    use crate::error::CoreError;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sgbr_json_file_{}_{name}/nested/value.json",
            std::process::id()
        ))
    }

    #[test]
    fn save_creates_parents_and_round_trips() {
        let path = tmp_path("round");
        let _ = std::fs::remove_dir_all(path.parent().and_then(|p| p.parent()).unwrap_or(&path));
        save(&vec![1_u8, 2, 3], &path).expect("save");
        let back: Vec<u8> = load(&path).expect("load");
        assert_eq!(back, vec![1, 2, 3]);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file renamed away"
        );
    }

    #[test]
    fn load_missing_is_io_error() {
        let path = tmp_path("missing");
        assert!(matches!(load::<u8>(&path), Err(CoreError::Io(_))));
    }

    #[test]
    fn load_garbage_is_parse_error() {
        let path = tmp_path("garbage");
        write_atomic(&path, b"not json").expect("write");
        assert!(matches!(load::<u8>(&path), Err(CoreError::Parse(_))));
    }
}
