//! The persisted commute list. Serializes to/from JSON; callers own file IO.

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
}

#[cfg(test)]
mod tests {
    use super::CommuteStore;
    use crate::commute::model::{Commute, TimeOfDay, Weekdays};
    use time::Weekday::Monday;

    fn sample_store() -> CommuteStore {
        let c = Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            None,
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
}
