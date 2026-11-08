//! Atomic JSON cache for the catalog (see `crate::json_file`).

use std::path::Path;

use crate::bus_catalog::model::BusCatalog;
use crate::error::CoreError;
use crate::json_file;

/// Load a catalog from `path`. A missing or invalid file is an error; the caller
/// may then fetch a fresh one.
pub fn load(path: &Path) -> Result<BusCatalog, CoreError> {
    json_file::load(path)
}

/// Save `catalog` to `path` atomically (temp file + rename), creating parents.
pub fn save(catalog: &BusCatalog, path: &Path) -> Result<(), CoreError> {
    json_file::save(catalog, path)
}

#[cfg(test)]
mod tests {
    use super::{load, save};
    use crate::bus_catalog::model::{BusCatalog, BusStop};
    use crate::error::CoreError;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sgbr_catalog_{}_{name}.json", std::process::id()))
    }

    #[test]
    fn round_trips() {
        let mut map = BTreeMap::new();
        map.insert("83139".to_owned(), vec!["15".to_owned()]);
        let catalog = BusCatalog {
            stops: vec![BusStop {
                code: "83139".to_owned(),
                name: "Clementi".to_owned(),
                road: "Ave 2".to_owned(),
            }],
            services_by_stop: map,
            fetched_at_unix: 42,
        };
        let path = tmp_path("round");
        save(&catalog, &path).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded, catalog);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_is_err() {
        assert!(matches!(load(&tmp_path("nope")), Err(CoreError::Io(_))));
    }
}
