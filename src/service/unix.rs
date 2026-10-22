//! Unix service backends: systemd on Linux, launchd on macOS.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "linux")]
use super::{SERVICE_DESCRIPTION, SERVICE_NAME};
use super::{ServicePaths, ServiceReport, ServiceState};

#[cfg(target_os = "linux")]
pub fn unit_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(format!("{SERVICE_NAME}.service"))
}

#[cfg(target_os = "macos")]
pub fn unit_path() -> PathBuf {
    PathBuf::from("/Library/LaunchDaemons").join(format!("{}.plist", super::LAUNCHD_LABEL))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn unit_path() -> PathBuf {
    PathBuf::from("/etc").join(SERVICE_NAME)
}

fn require_root(action: &str) -> Result<()> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        bail!("{action} needs root: re-run with sudo");
    }
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute `{cmd}`"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push_str("; ");
        }
        text.push_str(&stderr);
    }
    Ok(text)
}

// ---------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
fn render_unit(exe: &Path, config: &Path) -> String {
    format!(
        r#"[Unit]
Description={description}
Documentation=https://github.com/punisher1/{name}
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
ExecStart={exe} run --config {config}
Restart=always
RestartSec=5
# ICMP needs a raw socket. Running as root already allows it; the capability
# keeps things working if the unit is later changed to a non-root user.
AmbientCapabilities=CAP_NET_RAW
CapabilityBoundingSet=CAP_NET_RAW CAP_NET_ADMIN
NoNewPrivileges=true
ProtectSystem=full
ProtectHome=true

[Install]
WantedBy=multi-user.target
"#,
        description = SERVICE_DESCRIPTION,
        name = SERVICE_NAME,
        exe = exe.display(),
        config = config.display(),
    )
}

#[cfg(target_os = "linux")]
pub fn install(exe: &Path, config: &Path, _paths: &ServicePaths) -> Result<ServiceReport> {
    require_root("installing the systemd service")?;

    let unit = unit_path();
    std::fs::write(&unit, render_unit(exe, config))
        .with_context(|| format!("failed to write {}", unit.display()))?;

    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "--now", SERVICE_NAME])
        .context("systemctl enable --now failed")?;

    Ok(ServiceReport {
        state: ServiceState::Running,
        detail: format!("installed {} and started it", unit.display()),
    })
}

#[cfg(target_os = "linux")]
pub fn uninstall(_paths: &ServicePaths) -> Result<ServiceReport> {
    require_root("removing the systemd service")?;

    let unit = unit_path();
    if !unit.exists() {
        return Ok(ServiceReport {
            state: ServiceState::NotInstalled,
            detail: format!("{} does not exist; nothing to do", unit.display()),
        });
    }

    run("systemctl", &["disable", "--now", SERVICE_NAME])?;
    std::fs::remove_file(&unit).with_context(|| format!("failed to remove {}", unit.display()))?;
    run("systemctl", &["daemon-reload"])?;

    Ok(ServiceReport {
        state: ServiceState::NotInstalled,
        detail: format!("stopped and removed {}", unit.display()),
    })
}

#[cfg(target_os = "linux")]
pub fn status(_paths: &ServicePaths) -> Result<ServiceReport> {
    let unit = unit_path();
    if !unit.exists() {
        return Ok(ServiceReport {
            state: ServiceState::NotInstalled,
            detail: format!("{} does not exist", unit.display()),
        });
    }

    let active = run("systemctl", &["is-active", SERVICE_NAME])?;
    let enabled = run("systemctl", &["is-enabled", SERVICE_NAME]).unwrap_or_default();
    let state = if active == "active" {
        ServiceState::Running
    } else {
        ServiceState::Stopped
    };

    Ok(ServiceReport {
        state,
        detail: format!(
            "systemctl: active={active} enabled={enabled} unit={}",
            unit.display()
        ),
    })
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn render_plist(exe: &Path, config: &Path, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>run</string>
        <string>--config</string>
        <string>{config}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>5</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
        label = super::LAUNCHD_LABEL,
        exe = exe.display(),
        config = config.display(),
        log = log.display(),
    )
}

#[cfg(target_os = "macos")]
pub fn install(exe: &Path, config: &Path, paths: &ServicePaths) -> Result<ServiceReport> {
    require_root("installing the launchd service")?;

    let plist = unit_path();
    std::fs::write(&plist, render_plist(exe, config, &paths.log))
        .with_context(|| format!("failed to write {}", plist.display()))?;

    // `bootstrap` is the modern form; `load -w` covers older releases.
    let path = plist.display().to_string();
    if run("launchctl", &["bootstrap", "system", &path]).is_err() {
        run("launchctl", &["load", "-w", &path])?;
    }

    Ok(ServiceReport {
        state: ServiceState::Running,
        detail: format!("installed {} and bootstrapped it", plist.display()),
    })
}

#[cfg(target_os = "macos")]
pub fn uninstall(_paths: &ServicePaths) -> Result<ServiceReport> {
    require_root("removing the launchd service")?;

    let plist = unit_path();
    if !plist.exists() {
        return Ok(ServiceReport {
            state: ServiceState::NotInstalled,
            detail: format!("{} does not exist; nothing to do", plist.display()),
        });
    }

    let path = plist.display().to_string();
    if run("launchctl", &["bootout", "system", &path]).is_err() {
        run("launchctl", &["unload", "-w", &path])?;
    }
    std::fs::remove_file(&plist)
        .with_context(|| format!("failed to remove {}", plist.display()))?;

    Ok(ServiceReport {
        state: ServiceState::NotInstalled,
        detail: format!("booted out and removed {}", plist.display()),
    })
}

#[cfg(target_os = "macos")]
pub fn status(_paths: &ServicePaths) -> Result<ServiceReport> {
    let plist = unit_path();
    if !plist.exists() {
        return Ok(ServiceReport {
            state: ServiceState::NotInstalled,
            detail: format!("{} does not exist", plist.display()),
        });
    }

    let listing = run("launchctl", &["print", "system"])?;
    let needle = super::LAUNCHD_LABEL;
    let running = listing
        .lines()
        .any(|line| line.contains(needle) && line.contains("state = running"));

    Ok(ServiceReport {
        state: if running {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        },
        detail: format!("launchctl label={needle} plist={}", plist.display()),
    })
}

// ---------------------------------------------------------------- other

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn install(_exe: &Path, _config: &Path, _paths: &ServicePaths) -> Result<ServiceReport> {
    bail!("service installation is only implemented for Linux (systemd) and macOS (launchd)")
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn uninstall(_paths: &ServicePaths) -> Result<ServiceReport> {
    bail!("service removal is only implemented for Linux (systemd) and macOS (launchd)")
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn status(_paths: &ServicePaths) -> Result<ServiceReport> {
    bail!("service status is only implemented for Linux (systemd) and macOS (launchd)")
}
