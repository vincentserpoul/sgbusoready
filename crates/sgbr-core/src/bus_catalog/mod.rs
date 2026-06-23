//! Static LTA bus catalog: a stop directory + the stop→services map, with a
//! cached fetch and fuzzy search. Pure logic; the app owns refresh scheduling.

pub mod fetch;
pub mod model;
pub(crate) mod parse;
pub mod search;
pub mod store;
