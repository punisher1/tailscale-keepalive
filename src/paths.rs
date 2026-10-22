//! Platform-specific default locations for config and runtime state.

use std::path::PathBuf;

pub const APP_DIR: &str = "tailscale-keepalive";
pub const CONFIG_FILE: &str = "config.toml";

/// True when running with elevated privileges, where system-wide locations
/// (and a service-managed config) are the right choice.
#[cfg(unix)]
pub fn is_privileged() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(windows)]
#[allow(dead_code)]
pub fn is_privileged() -> bool {
    // Windows has no cheap "am I admin" check without extra API calls; the
    // service manager and UAC decide. User-level paths work for both cases.
    false
}

/// Default config file path.
///
/// Resolution order: `TS_KEEPALIVE_CONFIG`, then a system-wide path when
/// privileged, then a per-user path.
pub fn default_config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TS_KEEPALIVE_CONFIG").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }

    #[cfg(unix)]
    if is_privileged() {
        return Some(PathBuf::from("/etc").join(APP_DIR).join(CONFIG_FILE));
    }

    config_dir().map(|dir| dir.join(CONFIG_FILE))
}

#[cfg(windows)]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(|v| PathBuf::from(v).join(APP_DIR))
}

#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    home_dir().map(|home| {
        home.join("Library")
            .join("Application Support")
            .join(APP_DIR)
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".config")))
        .map(|dir| dir.join(APP_DIR))
}

/// Directory holding runtime files (the single-instance lock, default log).
pub fn state_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    if is_privileged() {
        return Some(PathBuf::from("/var/lib").join(APP_DIR));
    }

    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(|v| PathBuf::from(v).join(APP_DIR))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(".local").join("state")))
            .map(|dir| dir.join(APP_DIR))
    }

    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library").join("Logs").join(APP_DIR))
    }
}

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}
