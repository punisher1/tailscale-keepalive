use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Default probe interval, in seconds.
///
/// Chosen from field data: the shortest idle UDP timeout observed on a real
/// path was ~2 minutes, so 25s keeps a comfortable margin.
pub const DEFAULT_INTERVAL: u64 = 25;
/// Default per-probe timeout, in seconds.
pub const DEFAULT_TIMEOUT: u64 = 2;
/// Default number of consecutive failures before the link is reported as down.
pub const DEFAULT_FAIL_THRESHOLD: u32 = 3;

#[derive(Debug, Parser)]
#[command(
    name = "tailscale-keepalive",
    version,
    about = "Keep NAT mappings on a Tailscale path alive with periodic ICMP probes",
    long_about = "Sends a small ICMP echo to a peer's Tailscale IP at a fixed interval.\n\
                  Each packet travels inside the same WireGuard/UDP flow Tailscale uses, so\n\
                  every NAT on the path sees traffic well before its idle timeout expires\n\
                  and the mapping is never reclaimed."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Peer Tailscale IP to probe (e.g. 100.x.y.z).
    #[arg(short = 't', long, global = true, value_name = "IP")]
    pub target: Option<String>,

    /// Probe interval in seconds.
    #[arg(short = 'i', long, global = true, value_name = "SECONDS")]
    pub interval: Option<u64>,

    /// Timeout for a single probe, in seconds.
    #[arg(short = 'W', long, global = true, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// Consecutive failures before logging that the link looks down.
    #[arg(short = 'f', long, global = true, value_name = "N")]
    pub fail_threshold: Option<u32>,

    /// Configuration file path.
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Append log lines (with timestamps) to this file.
    #[arg(short = 'L', long, global = true, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    /// Log every probe, not only failures.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Output format for `status`.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Run the keepalive loop in the foreground (default).
    Run,
    /// Register the keepalive as a system service and start it.
    Install,
    /// Stop and remove the system service.
    Uninstall,
    /// Show the service state and the effective configuration.
    Status,
    /// Print the effective configuration and exit.
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}
