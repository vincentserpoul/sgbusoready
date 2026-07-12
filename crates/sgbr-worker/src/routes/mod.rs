//! Axum router assembly.

pub mod admin;
pub mod arrivals;
pub mod catalog;
pub mod devices;
pub mod health;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::state::AppState;

/// Build the full worker router bound to `state`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/devices", post(devices::register))
        .route("/v1/arrivals", get(arrivals::arrivals))
        .route("/v1/bus-stops", get(catalog::bus_stops))
        .route("/v1/bus-routes", get(catalog::bus_routes))
        .route("/v1/healthz", get(health::healthz))
        .route(
            "/v1/admin/invites",
            post(admin::mint_invite).get(admin::list_invites),
        )
        .route("/v1/admin/devices", get(admin::list_devices))
        .route("/v1/admin/devices/{id}", delete(admin::revoke_device))
        .with_state(state)
}
