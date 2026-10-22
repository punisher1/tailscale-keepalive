//! Minimal timestamped logger.
//!
//! Every line carries a local timestamp: when the keepalive is silently dying,
//! the log is the only evidence of *when* and *how often* it happened.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

struct Sink {
    file: Option<Mutex<File>>,
    verbose: bool,
    quiet: bool,
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install the global logger. Safe to call once; later calls are ignored.
pub fn init(log_file: Option<&Path>, verbose: bool, quiet: bool) -> Result<()> {
    let file = match log_file {
        Some(path) => {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            Some(Mutex::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .with_context(|| format!("failed to open log file {}", path.display()))?,
            ))
        }
        None => None,
    };

    let _ = SINK.set(Sink {
        file,
        verbose,
        quiet,
    });
    Ok(())
}

/// The log target a message is written to.
pub fn record(level: Level, message: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("{stamp} {:<5} {message}", level.label());

    let Some(sink) = SINK.get() else {
        if !matches!(level, Level::Debug) {
            println!("{line}");
        }
        return;
    };

    if matches!(level, Level::Debug) && !sink.verbose {
        return;
    }

    if !sink.quiet {
        if matches!(level, Level::Debug) {
            println!("{line}");
        } else {
            eprintln!("{line}");
        }
    }

    if let Some(file) = &sink.file
        && let Ok(mut handle) = file.lock()
    {
        let _ = writeln!(handle, "{line}");
        let _ = handle.flush();
    }
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::logging::record($crate::logging::Level::Info, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::logging::record($crate::logging::Level::Warn, &format!($($arg)*)) };
}

#[macro_export]
#[allow(unused_macros)]
macro_rules! error {
    ($($arg:tt)*) => { $crate::logging::record($crate::logging::Level::Error, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::logging::record($crate::logging::Level::Debug, &format!($($arg)*)) };
}
