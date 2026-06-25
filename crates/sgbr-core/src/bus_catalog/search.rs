//! Fuzzy stop search over names, with a boost for stop-code prefix matches.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::bus_catalog::model::{BusCatalog, BusStop};

/// Added when the stop code prefix-matches the query, so code hits outrank fuzzy
/// name hits ("83139" jumps that stop to the top).
const CODE_BOOST: u32 = 1_000_000;

/// Up to `limit` stops best matching `query` (fuzzy on name, prefix on code),
/// best first. An empty/whitespace query yields no results.
#[must_use]
pub fn search<'a>(catalog: &'a BusCatalog, query: &str, limit: usize) -> Vec<&'a BusStop> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart);

    let mut buf: Vec<char> = Vec::new();
    let mut scored: Vec<(u32, &BusStop)> = Vec::new();
    for stop in &catalog.stops {
        let name_score = pattern.score(Utf32Str::new(&stop.name, &mut buf), &mut matcher);
        let code_hit = stop.code.starts_with(trimmed);
        match (name_score, code_hit) {
            (Some(s), true) => scored.push((s.saturating_add(CODE_BOOST), stop)),
            (Some(s), false) => scored.push((s, stop)),
            (None, true) => scored.push((CODE_BOOST, stop)),
            (None, false) => {}
        }
    }
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, stop)| stop)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::search;
    use crate::bus_catalog::model::{BusCatalog, BusStop};

    fn stop(code: &str, name: &str) -> BusStop {
        BusStop {
            code: code.to_owned(),
            name: name.to_owned(),
            road: String::new(),
        }
    }

    fn catalog() -> BusCatalog {
        BusCatalog {
            stops: vec![
                stop("17009", "Clementi Int"),
                stop("83139", "Clementi Ave 2 Blk 333"),
                stop("01012", "Hotel Grand Pacific"),
                stop("16009", "NUS Clementi Rd"),
            ],
            ..BusCatalog::default()
        }
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(search(&catalog(), "  ", 10).is_empty());
    }

    #[test]
    fn fuzzy_name_match() {
        let cat = catalog();
        let results = search(&cat, "clementi", 10);
        assert!(results.iter().all(|s| s.name.contains("Clementi")));
        assert!(results.len() >= 3);
        assert!(!results.iter().any(|s| s.code == "01012"));
    }

    #[test]
    fn code_prefix_ranks_first() {
        let cat = catalog();
        let results = search(&cat, "83139", 10);
        assert_eq!(results.first().map(|s| s.code.as_str()), Some("83139"));
    }

    #[test]
    fn limit_zero_returns_empty() {
        let cat = catalog();
        assert!(search(&cat, "clementi", 0).is_empty());
    }
}
