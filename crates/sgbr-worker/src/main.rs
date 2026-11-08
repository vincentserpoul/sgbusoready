//! `sgbr-worker`: the axum backend that hides the LTA `DataMall` `AccountKey`
//! and serves device registration, the bus catalog, the live arrivals SSE
//! stream, and admin endpoints. `serve` runs the server; the other
//! subcommands are thin clients for the admin API.

use std::sync::Arc;

use clap::{Parser, Subcommand};

use sgbr_worker::catalog::{self, CatalogCache};
use sgbr_worker::config::Config;
use sgbr_worker::registry::{ArrivalFetcher, LtaFetcher, Registry};
use sgbr_worker::routes;
use sgbr_worker::state::AppState;
use sgbr_worker::store::DeviceStore;

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

/// Resolve on SIGINT (Ctrl-C) *or* SIGTERM — the latter is what
/// `systemctl stop` / podman send, so without it the graceful-shutdown path
/// never ran in the container and open SSE streams were killed outright instead.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        let _ = ctrl_c.await;
        return;
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
}

#[derive(Debug, Clone, Copy)]
enum Verb {
    Get,
    Post,
    Delete,
}

/// One call against the admin API at `{url}{path}`, authenticated with
/// `SGBR_ADMIN_TOKEN`. Returns the JSON body, or `None` for an empty one
/// (`204` on revoke).
fn admin_call(
    verb: Verb,
    url: &str,
    path: &str,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    let token =
        std::env::var("SGBR_ADMIN_TOKEN").map_err(|_var_err| "SGBR_ADMIN_TOKEN is not set")?;
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
    let full = format!("{url}{path}");
    let mut response = match verb {
        Verb::Get => agent.get(&full).header("X-Admin-Token", &token).call()?,
        Verb::Post => agent
            .post(&full)
            .header("X-Admin-Token", &token)
            .send_empty()?,
        Verb::Delete => agent.delete(&full).header("X-Admin-Token", &token).call()?,
    };
    let text = response.body_mut().read_to_string()?;
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::from_str(&text)?))
    }
}

#[allow(clippy::print_stdout, reason = "CLI output")]
fn print_body(body: Option<serde_json::Value>) {
    if let Some(body) = body {
        println!("{body}");
    }
}

fn invite_cmd(action: InviteAction) -> Result<(), Box<dyn std::error::Error>> {
    let body = match action {
        InviteAction::New { url } => admin_call(Verb::Post, &url, "/v1/admin/invites")?,
        InviteAction::List { url } => admin_call(Verb::Get, &url, "/v1/admin/invites")?,
    };
    print_body(body);
    Ok(())
}

#[allow(clippy::print_stdout, reason = "CLI output")]
fn device_cmd(action: DeviceAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        DeviceAction::List { url } => {
            print_body(admin_call(Verb::Get, &url, "/v1/admin/devices")?);
        }
        DeviceAction::Revoke { id, url } => {
            admin_call(Verb::Delete, &url, &format!("/v1/admin/devices/{id}"))?;
            println!("revoked {id}");
        }
    }
    Ok(())
}
