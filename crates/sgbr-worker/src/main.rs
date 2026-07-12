//! `sgbr-worker`: the axum backend that hides the LTA `DataMall` `AccountKey`
//! and serves device registration, the bus catalog, and admin endpoints. The
//! arrivals SSE stream lands in Phase 2.

mod auth;
mod catalog;
mod config;
mod error;
mod registry;
mod routes;
mod state;
mod store;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use catalog::CatalogCache;
use config::Config;
use registry::{ArrivalFetcher, LtaFetcher, Registry};
use state::AppState;
use store::DeviceStore;

const DEFAULT_ADMIN_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Parser)]
#[command(name = "sgbr-worker", about = "SG Bus Commute backend worker")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the HTTP server (default when no subcommand is given).
    Serve,
    /// Manage invite codes via the admin API.
    Invite {
        #[command(subcommand)]
        action: InviteAction,
    },
    /// Manage registered devices via the admin API.
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
}

#[derive(Debug, Subcommand)]
enum InviteAction {
    /// Mint a fresh invite code.
    New {
        #[arg(long, default_value = DEFAULT_ADMIN_URL)]
        url: String,
    },
    /// List all invite codes and their used state.
    List {
        #[arg(long, default_value = DEFAULT_ADMIN_URL)]
        url: String,
    },
}

#[derive(Debug, Subcommand)]
enum DeviceAction {
    /// List all registered devices.
    List {
        #[arg(long, default_value = DEFAULT_ADMIN_URL)]
        url: String,
    },
    /// Revoke a device by id.
    Revoke {
        id: String,
        #[arg(long, default_value = DEFAULT_ADMIN_URL)]
        url: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
        Command::Invite { action } => invite_cmd(action),
        Command::Device { action } => device_cmd(action),
    }
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = Config::from_env()?;
    let bind = config.bind.clone();
    let data_dir = config.data_dir.clone();
    let account_key = config.lta_account_key.clone();
    let poll_interval = std::time::Duration::from_secs(config.poll_secs);

    let store = DeviceStore::load_or_default(data_dir.join("store.json"))?;
    let catalog = Arc::new(CatalogCache::load_or_empty(&data_dir));
    let fetcher: Arc<dyn ArrivalFetcher> = Arc::new(LtaFetcher::new(account_key.clone()));
    let registry = Registry::new(fetcher, poll_interval);
    let state = AppState::new(
        Arc::new(config),
        Arc::new(store),
        Arc::clone(&catalog),
        registry,
    );

    tokio::spawn(catalog::run_refresh_loop(catalog, account_key, data_dir));

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("sgbr-worker listening on {bind}");
    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn admin_token() -> Result<String, Box<dyn std::error::Error>> {
    std::env::var("SGBR_ADMIN_TOKEN").map_err(|_var_err| "SGBR_ADMIN_TOKEN is not set".into())
}

fn admin_agent() -> ureq::Agent {
    ureq::Agent::new_with_config(ureq::Agent::config_builder().build())
}

#[allow(clippy::print_stdout, reason = "CLI output")]
fn invite_cmd(action: InviteAction) -> Result<(), Box<dyn std::error::Error>> {
    let token = admin_token()?;
    let agent = admin_agent();
    match action {
        InviteAction::New { url } => {
            let mut response = agent
                .post(format!("{url}/v1/admin/invites"))
                .header("X-Admin-Token", &token)
                .send_empty()?;
            let body: serde_json::Value = response.body_mut().read_json()?;
            println!("{body}");
        }
        InviteAction::List { url } => {
            let mut response = agent
                .get(format!("{url}/v1/admin/invites"))
                .header("X-Admin-Token", &token)
                .call()?;
            let body: serde_json::Value = response.body_mut().read_json()?;
            println!("{body}");
        }
    }
    Ok(())
}

#[allow(clippy::print_stdout, reason = "CLI output")]
fn device_cmd(action: DeviceAction) -> Result<(), Box<dyn std::error::Error>> {
    let token = admin_token()?;
    let agent = admin_agent();
    match action {
        DeviceAction::List { url } => {
            let mut response = agent
                .get(format!("{url}/v1/admin/devices"))
                .header("X-Admin-Token", &token)
                .call()?;
            let body: serde_json::Value = response.body_mut().read_json()?;
            println!("{body}");
        }
        DeviceAction::Revoke { id, url } => {
            agent
                .delete(format!("{url}/v1/admin/devices/{id}"))
                .header("X-Admin-Token", &token)
                .call()?;
            println!("revoked {id}");
        }
    }
    Ok(())
}
