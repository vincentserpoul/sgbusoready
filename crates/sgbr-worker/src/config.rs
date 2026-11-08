//! Environment-derived worker configuration.

use std::fs;
use std::path::PathBuf;

use thiserror::Error;

/// Failures building [`Config`] from the environment.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("missing required env var {0}")]
    MissingEnv(String),
    #[error("invalid value for env var {0}: {1}")]
    InvalidEnv(String, String),
    #[error("failed to create data dir {0}: {1}")]
    DataDir(String, String),
}

/// Worker runtime configuration, read once at `serve` startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub lta_account_key: String,
    pub bind: String,
    pub data_dir: PathBuf,
    pub admin_token: String,
    /// Arrivals poll interval per subscribed stop (see `registry::Registry`).
    pub poll_secs: u64,
}

const DEFAULT_BIND: &str = "127.0.0.1:8080";
const DEFAULT_DATA_DIR: &str = "./data";
const DEFAULT_POLL_SECS: u64 = 15;
/// Floor for `SGBR_POLL_SECS`: anything lower turns every subscribed stop
/// into a back-to-back request loop against `DataMall` (`0` would not sleep
/// at all) and risks the shared `AccountKey` being rate-limited.
const MIN_POLL_SECS: u64 = 5;

impl Config {
    /// Build from the process environment (`LTA_API_ACCOUNT_KEY`,
    /// `SGBR_ADMIN_TOKEN` required; `SGBR_BIND`, `SGBR_DATA_DIR`,
    /// `SGBR_POLL_SECS` optional). Creates `SGBR_DATA_DIR` if missing.
    ///
    /// # Errors
    /// [`ConfigError::MissingEnv`] if a required var is unset,
    /// [`ConfigError::InvalidEnv`] if `SGBR_POLL_SECS` isn't a `u64` of at
    /// least `MIN_POLL_SECS`,
    /// [`ConfigError::DataDir`] if the data dir can't be created.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Same as [`Self::from_env`], but reads through `lookup` instead of the
    /// real process environment — lets tests exercise every branch without
    /// touching (unsafe, process-global) env vars.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let lta_account_key = lookup("LTA_API_ACCOUNT_KEY")
            .ok_or_else(|| ConfigError::MissingEnv("LTA_API_ACCOUNT_KEY".to_owned()))?;
        let admin_token = lookup("SGBR_ADMIN_TOKEN")
            .ok_or_else(|| ConfigError::MissingEnv("SGBR_ADMIN_TOKEN".to_owned()))?;
        let bind = lookup("SGBR_BIND").unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let data_dir =
            PathBuf::from(lookup("SGBR_DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.to_owned()));
        fs::create_dir_all(&data_dir)
            .map_err(|e| ConfigError::DataDir(data_dir.display().to_string(), e.to_string()))?;
        let poll_secs = match lookup("SGBR_POLL_SECS") {
            Some(v) => v
                .parse::<u64>()
                .map_err(|e| ConfigError::InvalidEnv("SGBR_POLL_SECS".to_owned(), e.to_string()))?,
            None => DEFAULT_POLL_SECS,
        };
        if poll_secs < MIN_POLL_SECS {
            return Err(ConfigError::InvalidEnv(
                "SGBR_POLL_SECS".to_owned(),
                format!("must be at least {MIN_POLL_SECS}"),
            ));
        }
        Ok(Self {
            lta_account_key,
            bind,
            data_dir,
            admin_token,
            poll_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn missing_lta_key_is_an_error() {
        let err =
            Config::from_lookup(lookup(&[("SGBR_ADMIN_TOKEN", "tok")])).expect_err("missing key");
        assert_eq!(
            err,
            ConfigError::MissingEnv("LTA_API_ACCOUNT_KEY".to_owned())
        );
    }

    #[test]
    fn missing_admin_token_is_an_error() {
        let err = Config::from_lookup(lookup(&[("LTA_API_ACCOUNT_KEY", "key")]))
            .expect_err("missing token");
        assert_eq!(err, ConfigError::MissingEnv("SGBR_ADMIN_TOKEN".to_owned()));
    }

    #[test]
    fn defaults_apply_when_optional_vars_are_unset() {
        let tmp =
            std::env::temp_dir().join(format!("sgbr_worker_cfg_defaults_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let config = Config::from_lookup(lookup(&[
            ("LTA_API_ACCOUNT_KEY", "key"),
            ("SGBR_ADMIN_TOKEN", "tok"),
            ("SGBR_DATA_DIR", tmp.to_str().unwrap_or_default()),
        ]))
        .expect("build config");
        assert_eq!(config.bind, "127.0.0.1:8080");
        assert_eq!(config.poll_secs, 15);
        assert!(tmp.is_dir());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn invalid_poll_secs_is_an_error() {
        let tmp =
            std::env::temp_dir().join(format!("sgbr_worker_cfg_badpoll_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let err = Config::from_lookup(lookup(&[
            ("LTA_API_ACCOUNT_KEY", "key"),
            ("SGBR_ADMIN_TOKEN", "tok"),
            ("SGBR_DATA_DIR", tmp.to_str().unwrap_or_default()),
            ("SGBR_POLL_SECS", "not-a-number"),
        ]));
        assert!(matches!(err, Err(ConfigError::InvalidEnv(_, _))));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn poll_secs_below_floor_is_an_error() {
        let tmp =
            std::env::temp_dir().join(format!("sgbr_worker_cfg_lowpoll_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for value in ["0", "4"] {
            let err = Config::from_lookup(lookup(&[
                ("LTA_API_ACCOUNT_KEY", "key"),
                ("SGBR_ADMIN_TOKEN", "tok"),
                ("SGBR_DATA_DIR", tmp.to_str().unwrap_or_default()),
                ("SGBR_POLL_SECS", value),
            ]));
            assert!(
                matches!(err, Err(ConfigError::InvalidEnv(_, _))),
                "{value} should be rejected"
            );
        }
        let config = Config::from_lookup(lookup(&[
            ("LTA_API_ACCOUNT_KEY", "key"),
            ("SGBR_ADMIN_TOKEN", "tok"),
            ("SGBR_DATA_DIR", tmp.to_str().unwrap_or_default()),
            ("SGBR_POLL_SECS", "5"),
        ]))
        .expect("floor value is accepted");
        assert_eq!(config.poll_secs, 5);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn creates_missing_data_dir() {
        let tmp =
            std::env::temp_dir().join(format!("sgbr_worker_cfg_mkdir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(!tmp.exists());
        let config = Config::from_lookup(lookup(&[
            ("LTA_API_ACCOUNT_KEY", "key"),
            ("SGBR_ADMIN_TOKEN", "tok"),
            ("SGBR_DATA_DIR", tmp.to_str().unwrap_or_default()),
        ]))
        .expect("build config");
        assert!(config.data_dir.is_dir());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
