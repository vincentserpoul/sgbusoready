//! The persisted commute list. Serializes to/from JSON; callers own file IO.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commute::model::Commute;
use crate::error::CoreError;

/// The full, ordered list of configured commutes. Order is the user's display
/// order; the UI reorders by mutating `commutes`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommuteStore {
    pub commutes: Vec<Commute>,
}

impl CommuteStore {
    /// Serialize the list to pretty JSON.
    pub fn to_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(self).map_err(|e| CoreError::Parse(e.to_string()))
    }

    /// Parse a list from JSON produced by [`Self::to_json`].
    pub fn from_json(json: &str) -> Result<Self, CoreError> {
        serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))
    }

    /// Load the store from `path`. A missing file yields an empty store (the
    /// natural first-run state); a present-but-unparseable file is an error.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        match fs::read_to_string(path) {
            Ok(json) => Self::from_json(&json),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CoreError::Io(e.to_string())),
        }
    }

    /// Persist the store to `path`, creating parent directories as needed.
    /// Writes to a sibling temp file then renames, so a crash mid-write cannot
    /// leave a half-written settings file.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::Io(e.to_string()))?;
        }
        let json = self.to_json()?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| CoreError::Io(e.to_string()))?;
        fs::rename(&tmp, path).map_err(|e| CoreError::Io(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::CommuteStore;
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
    use std::path::PathBuf;
    use time::Weekday::Monday;

    fn sample_store() -> CommuteStore {
        let c = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned()],
            }],
        )
        .expect("valid commute");
        CommuteStore { commutes: vec![c] }
    }

    #[test]
    fn json_round_trip_preserves_commutes() {
        let store = sample_store();
        let json = store.to_json().expect("serialize");
        let back = CommuteStore::from_json(&json).expect("deserialize");
        assert_eq!(store, back);
    }

    #[test]
    fn empty_store_round_trips() {
        let store = CommuteStore { commutes: vec![] };
        let json = store.to_json().expect("serialize");
        let back = CommuteStore::from_json(&json).expect("deserialize");
        assert_eq!(store, back);
    }

    #[test]
    fn garbage_json_is_parse_error() {
        let err = CommuteStore::from_json("not json").unwrap_err();
        assert!(matches!(err, crate::error::CoreError::Parse(_)));
    }

    /// A unique, process-isolated temp file path (no `tempfile` dependency).
    fn temp_store_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("sgbr-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.push("commutes.json");
        dir
    }

    #[test]
    fn load_missing_file_returns_empty_store() {
        let path = temp_store_path("missing");
        // Ensure it does not exist.
        let _ = std::fs::remove_file(&path);
        let store = CommuteStore::load(&path).expect("load missing");
        assert_eq!(store, CommuteStore::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_store_path("roundtrip");
        let store = sample_store();
        store.save(&path).expect("save");
        let back = CommuteStore::load(&path).expect("load");
        assert_eq!(store, back);
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let mut path = temp_store_path("nested");
        path.pop(); // drop commutes.json
        path.push("deeper");
        path.push("commutes.json");
        let store = sample_store();
        store.save(&path).expect("save into new dir");
        assert!(path.exists());
    }

    #[test]
    fn load_corrupt_file_is_parse_error() {
        let path = temp_store_path("corrupt");
        std::fs::write(&path, "not json").expect("write corrupt");
        let err = CommuteStore::load(&path).unwrap_err();
        assert!(matches!(err, crate::error::CoreError::Parse(_)));
    }

    #[test]
    fn multi_stop_commute_round_trips() {
        let c = Commute::new(
            Some("Morning to work".to_owned()),
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![
                CommuteStop {
                    code: "83139".to_owned(),
                    name: "Opp Blk 123".to_owned(),
                    buses: vec!["14".to_owned(), "14e".to_owned()],
                },
                CommuteStop {
                    code: "17009".to_owned(),
                    name: "Bef Clementi Stn".to_owned(),
                    buses: vec!["96".to_owned()],
                },
            ],
        )
        .expect("valid commute");
        let store = CommuteStore { commutes: vec![c] };
        let json = store.to_json().expect("serialize");
        let back = CommuteStore::from_json(&json).expect("deserialize");
        assert_eq!(store, back);
    }
}
