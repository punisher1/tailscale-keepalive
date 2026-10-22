//! System service integration: register, remove, and inspect the daemon on
//! Windows (SCM), Linux (systemd), and macOS (launchd).

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::settings::Settings;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub const SERVICE_NAME: &str = "tailscale-keepalive";
#[cfg(windows)]
pub const SERVICE_DISPLAY_NAME: &str = "Tailscale Keepalive";
#[cfg(any(windows, target_os = "linux"))]
pub const SERVICE_DESCRIPTION: &str =
    "Keeps NAT mappings on the Tailscale path alive with periodic ICMP probes.";
#[cfg(target_os = "macos")]
pub const LAUNCHD_LABEL: &str = "com.punisher1.tailscale-keepalive";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Stopped,
    NotInstalled,
}

impl ServiceState {
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceState::Running => "running",
            ServiceState::Stopped => "stopped",
            ServiceState::NotInstalled => "not installed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServiceReport {
    pub state: ServiceState,
    /// Platform detail: unit state, SCM status, launchd state, or the error.
    pub detail: String,
}

/// Where the service reads its configuration from, and where it writes logs.
///
/// Services are installed system-wide so they start before anyone logs in, so
/// these are not the per-user paths used by a foreground `run`.
pub struct ServicePaths {
    pub config: PathBuf,
    pub log: PathBuf,
}

impl ServicePaths {
    pub fn detect() -> Result<ServicePaths> {
        #[cfg(windows)]
        {
            let base = std::env::var_os("ProgramData")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
            let dir = base.join(SERVICE_NAME);
            Ok(ServicePaths {
                config: dir.join("config.toml"),
                log: dir.join("keepalive.log"),
            })
        }

        #[cfg(target_os = "macos")]
        {
            let dir = PathBuf::from("/Library/Application Support").join(SERVICE_NAME);
            Ok(ServicePaths {
                config: dir.join("config.toml"),
                log: PathBuf::from("/Library/Logs").join(format!("{SERVICE_NAME}.log")),
            })
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            Ok(ServicePaths {
                config: PathBuf::from("/etc").join(SERVICE_NAME).join("config.toml"),
                log: PathBuf::from("/var/log")
                    .join(SERVICE_NAME)
                    .join("keepalive.log"),
            })
        }
    }

    /// Settings as they will be persisted for the service: the log always goes
    /// to a file, because a service has no console to print to.
    pub fn service_settings(&self, settings: &Settings) -> Settings {
        let mut settings = settings.clone();
        if settings.log_file.is_none() {
            settings.log_file = Some(self.log.clone());
        }
        settings
    }

    fn write_config(&self, settings: &Settings) -> Result<PathBuf> {
        if let Some(parent) = self.config.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(log) = &settings.log_file
            && let Some(parent) = log.parent()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.config, settings.to_toml()?)?;
        Ok(self.config.clone())
    }
}

/// The absolute path of the currently running executable.
pub fn current_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

pub fn install(settings: &Settings) -> Result<ServiceReport> {
    let paths = ServicePaths::detect()?;
    let settings = paths.service_settings(settings);
    let config = paths.write_config(&settings)?;
    let exe = current_exe()?;
    imp::install(&exe, &config, &paths)
}

pub fn uninstall() -> Result<ServiceReport> {
    let paths = ServicePaths::detect()?;
    imp::uninstall(&paths)
}

pub fn status() -> Result<ServiceReport> {
    let paths = ServicePaths::detect()?;
    imp::status(&paths)
}

pub fn service_config_path() -> Result<PathBuf> {
    Ok(ServicePaths::detect()?.config)
}

/// Quote a path for a command line, if it needs it.
#[allow(dead_code)]
pub fn quote(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text
    }
}

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

#[cfg(windows)]
pub use windows::try_run_as_service;

/// Install the service with the platform's own mechanism.
#[cfg(windows)]
pub fn platform_name() -> &'static str {
    "Windows Service (SCM)"
}

#[cfg(target_os = "linux")]
pub fn platform_name() -> &'static str {
    "systemd"
}

#[cfg(target_os = "macos")]
pub fn platform_name() -> &'static str {
    "launchd"
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn platform_name() -> &'static str {
    "unsupported unix"
}
