//! Static LTA bus catalog: a stop directory + the stop→services map, with a
//! cached fetch and fuzzy search. Pure logic; the app owns refresh scheduling.

#[cfg(feature = "lta-upstream")]
pub mod fetch;
pub mod model;
// `pub`, not `pub(crate)`: `sgbr-worker` assembles `GET /v1/bus-routes`
// responses from `services_by_stop` and its tests round-trip them back
// through `build_services_by_stop` to assert losslessness.
pub mod parse;
pub mod search;
pub mod store;
