mod cli;
mod keepalive;
mod lock;
mod logging;
mod paths;
mod probe;
mod service;
mod settings;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::sync::watch;

use cli::{Cli, Command, OutputFormat};
use lock::InstanceLock;
use settings::{Layer, Settings};

fn main() -> Result<ExitCode> {
    #[cfg(windows)]
    if service::try_run_as_service()? {
        return Ok(ExitCode::SUCCESS);
    }

    let cli = Cli::parse();
    dispatch(cli)?;
    Ok(ExitCode::SUCCESS)
}

fn dispatch(cli: Cli) -> Result<()> {
    let command = cli.command.unwrap_or(Command::Run);

    match command {
        Command::Run => {
            let settings = build_settings(&cli)?;
            logging::init(settings.log_file.as_deref(), settings.verbose, false)?;
            run_foreground(&settings)?;
            Ok(())
        }
        Command::Install => {
            let settings = build_settings(&cli).context(
                "`install` needs a target: pass --target, set TS_KEEPALIVE_TARGET, or provide \
                 --config with a `target` entry",
            )?;
            logging::init(settings.log_file.as_deref(), settings.verbose, false)?;
            info!(
                "installing the system service via {}",
                service::platform_name()
            );
            let report = service::install(&settings)?;
            info!("{}", report.detail);
            info!(
                "service config: {}",
                service::service_config_path()?.display()
            );
            Ok(())
        }
        Command::Uninstall => {
            logging::init(None, cli.verbose, false)?;
            let report = service::uninstall()?;
            info!("{}", report.detail);
            Ok(())
        }
        Command::Status => {
            logging::init(None, cli.verbose, true)?;
            let report = service::status()?;
            let settings = build_settings(&cli).ok();
            if cli.output == OutputFormat::Json {
                print_status_json(&report, settings.as_ref());
            } else {
                print_status(&report, settings.as_ref(), cli.output);
            }
            Ok(())
        }
        Command::Config => {
            logging::init(None, cli.verbose, true)?;
            let settings = build_settings(&cli)?;
            print!("{}", settings.to_toml()?);
            Ok(())
        }
    }
}

/// Resolve settings from all four layers, highest priority first.
fn build_settings(cli: &Cli) -> Result<Settings> {
    let config_path = cli.config.clone().or_else(paths::default_config_path);

    let file_layer = match &config_path {
        Some(path) => settings::load_file(path)?
            .map(Layer::from)
            .unwrap_or_default(),
        None => Layer::default(),
    };
    let env_layer = settings::env_layer(|name| std::env::var(name).ok())?;
    let cli_layer = Layer {
        target: cli.target.clone(),
        interval: cli.interval,
        timeout: cli.timeout,
        fail_threshold: cli.fail_threshold,
        log_file: cli.log_file.clone(),
        verbose: cli.verbose.then_some(true),
    };

    Settings::resolve(&[file_layer, env_layer, cli_layer])
}

fn lock_path() -> PathBuf {
    if let Some(path) = std::env::var_os("TS_KEEPALIVE_LOCK").filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    match paths::state_dir() {
        Some(dir) => dir.join("instance.lock"),
        None => std::env::temp_dir().join("tailscale-keepalive.lock"),
    }
}

fn run_foreground(settings: &Settings) -> Result<()> {
    let lock = InstanceLock::acquire(&lock_path())?;
    debug!("instance lock: {}", lock.path().display());

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;

    runtime.block_on(async move {
        let ctrl_c = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = shutdown_tx.send(true);
            }
        });

        let result = keepalive::run(settings, shutdown_rx).await;
        ctrl_c.abort();
        result
    })
}

/// Entry point used by the Windows service dispatcher: no CLI is parsed from a
/// shell, the command line comes from the SCM registration.
#[cfg(windows)]
pub fn service_run(shutdown: watch::Receiver<bool>) -> Result<()> {
    let cli = Cli::parse_from(std::env::args_os());
    let settings = build_settings(&cli)?;
    logging::init(settings.log_file.as_deref(), settings.verbose, true)?;
    let _lock = InstanceLock::acquire(&lock_path())?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    runtime.block_on(keepalive::run(&settings, shutdown))
}

fn print_status(
    report: &service::ServiceReport,
    settings: Option<&Settings>,
    _format: OutputFormat,
) {
    println!("service: {}", report.state.as_str());
    println!("backend: {}", service::platform_name());
    println!("detail:  {}", report.detail);
    match service::service_config_path() {
        Ok(path) => println!("config:  {}", path.display()),
        Err(err) => println!("config:  <unavailable: {err}>"),
    }
    match settings {
        Some(s) => println!("effective settings: {}", s.summary()),
        None => println!("effective settings: <no target configured>"),
    }
}

fn print_status_json(report: &service::ServiceReport, settings: Option<&Settings>) {
    let settings_json = match settings {
        Some(s) => format!(
            r#"{{"target":"{}","interval":{},"timeout":{},"fail_threshold":{},"log_file":{},"verbose":{}}}"#,
            escape_json(&s.target.to_string()),
            s.interval.as_secs(),
            s.timeout.as_secs(),
            s.fail_threshold,
            match &s.log_file {
                Some(p) => format!("\"{}\"", escape_json(&p.display().to_string())),
                None => "null".to_string(),
            },
            s.verbose
        ),
        None => "null".to_string(),
    };

    println!(
        r#"{{"service":{{"state":"{}","backend":"{}","detail":"{}"}},"settings":{}}}"#,
        report.state.as_str(),
        escape_json(service::platform_name()),
        escape_json(&report.detail),
        settings_json
    );
}

fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
