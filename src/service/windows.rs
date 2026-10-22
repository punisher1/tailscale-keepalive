//! Windows service backend (SCM).
//!
//! The same executable is both the console program and the service binary: SCM
//! starts it with `run --config ...`, and `try_run_as_service` connects it to
//! the service dispatcher before the CLI parser ever sees the arguments.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use tokio::sync::watch;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState as ScmState, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::{
    SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME, ServicePaths, ServiceReport,
    ServiceState,
};

windows_service::define_windows_service!(ffi_service_main, service_main);

/// If SCM launched us, run the keepalive under the service dispatcher and never
/// return. Returns `Ok(false)` when running in a normal console.
pub fn try_run_as_service() -> Result<bool> {
    match windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Ok(true),
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
        {
            // Started from a console, not by SCM: fall through to the CLI.
            Ok(false)
        }
        Err(err) => Err(err).context("failed to connect to the Windows service dispatcher"),
    }
}

const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;

fn service_main(_arguments: Vec<OsString>) {
    if let Err(err) = run_service() {
        // Nowhere to print; SCM only records that the service stopped.
        let _ = err;
    }
}

fn run_service() -> Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let event_handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = shutdown_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
    let running = || windows_service::service::ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ScmState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    status_handle.set_service_status(running())?;

    let result = crate::service_run(shutdown_rx);

    let _ = status_handle.set_service_status(windows_service::service::ServiceStatus {
        current_state: ScmState::Stopped,
        ..running()
    });

    result
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("failed to open the service manager (run as Administrator)")
}

pub fn install(exe: &Path, config: &Path, _paths: &ServicePaths) -> Result<ServiceReport> {
    let mgr = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;

    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.to_path_buf(),
        launch_arguments: vec![
            OsString::from("run"),
            OsString::from("--config"),
            config.as_os_str().to_os_string(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let service = mgr
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .context("failed to create the service (it may already exist; run `uninstall` first)")?;

    let _ = service.set_description(SERVICE_DESCRIPTION);

    // Let the service restart itself if it ever exits unexpectedly.
    let _ = service.set_failure_actions_on_non_crash_failures(true);

    service
        .start::<&str>(&[])
        .context("failed to start the service")?;

    Ok(ServiceReport {
        state: ServiceState::Running,
        detail: format!(
            "registered `{SERVICE_NAME}` (auto start) with config {}",
            config.display()
        ),
    })
}

pub fn uninstall(_paths: &ServicePaths) -> Result<ServiceReport> {
    let mgr = manager(ServiceManagerAccess::CONNECT)?;

    let service = match mgr.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(err)) if err.raw_os_error() == Some(1060) => {
            return Ok(ServiceReport {
                state: ServiceState::NotInstalled,
                detail: format!("service `{SERVICE_NAME}` is not installed"),
            });
        }
        Err(err) => return Err(err).context("failed to open the service"),
    };

    if let Ok(status) = service.query_status()
        && status.current_state != ScmState::Stopped
    {
        service.stop().context("failed to stop the service")?;
        for _ in 0..50 {
            match service.query_status() {
                Ok(status) if status.current_state == ScmState::Stopped => break,
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    service.delete().context("failed to delete the service")?;

    Ok(ServiceReport {
        state: ServiceState::NotInstalled,
        detail: format!("stopped and deleted service `{SERVICE_NAME}`"),
    })
}

pub fn status(_paths: &ServicePaths) -> Result<ServiceReport> {
    let mgr = manager(ServiceManagerAccess::CONNECT)?;

    let service = match mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(err)) if err.raw_os_error() == Some(1060) => {
            return Ok(ServiceReport {
                state: ServiceState::NotInstalled,
                detail: format!("service `{SERVICE_NAME}` is not installed"),
            });
        }
        Err(err) => return Err(err).context("failed to open the service"),
    };

    let status = service
        .query_status()
        .context("failed to query service status")?;
    let state = match status.current_state {
        ScmState::Running => ServiceState::Running,
        _ => ServiceState::Stopped,
    };

    Ok(ServiceReport {
        state,
        detail: format!(
            "scm state={:?} pid={:?}",
            status.current_state, status.process_id
        ),
    })
}
